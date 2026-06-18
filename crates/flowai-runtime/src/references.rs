//! Reference registry (reference registry, C2).
//!
//! The registry composes existing framework primitives — JSON Schema validation
//! ([`jsonschema`]), tenant-scoped content hashing
//! ([`agent_fw_core::id::tenant_scoped_hash`]), and the [`KVStore`] algebra's
//! TTL + tenant isolation guarantees — into a typed facade for "named typed
//! memory pointers".
//!
//! # Design choices (locked this session)
//!
//! - **Glimpse is host-precomputed.** [`ReferenceRegistry::create`] takes the
//!   glimpse as an explicit `serde_json::Value` parameter. The host SDK
//!   (Python / TypeScript) runs the user's `glimpse=lambda v: {...}` *before*
//!   calling Rust. The registry stores the result and returns it on every
//!   subsequent resolve — that's how the "computed once on create, served
//!   from cache on resolve" property is upheld without crossing FFI
//!   boundaries with a closure.
//! - **Schemas are precompiled at registry construction.** A malformed
//!   schema fails early via [`ReferenceError::SchemaInit`] — `create` never
//!   has to deal with parse errors. Validation runs only on `create`; a
//!   value that was valid at create time stays valid on resolve.
//! - **IDs are tenant-scoped.** The id derivation uses
//!   [`tenant_scoped_hash`], so the same value under tenants A and B
//!   produces different ids — no cross-tenant collisions even before the
//!   KV layer's L9 isolation kicks in.
//! - **Tenant mismatch on resolve returns `NotFound`** rather than a distinct
//!   error variant, to avoid leaking the existence of refs in other tenants.
//!   In practice the KV trait's L9 (tenant isolation) makes this
//!   unreachable; the explicit check is defence-in-depth.
//!
//! # Acceptance criteria (reference registry)
//!
//! All four are exercised by the `tests` module in this file:
//!
//! 1. A reference created under tenant A is not resolvable from tenant B.
//! 2. A reference with `ttl_ms = 100` is unresolvable after 200ms.
//! 3. Writing a value that violates the schema fails on `create`, never on
//!    `resolve`.
//! 4. The glimpse is computed exactly once (by the host on create), stored
//!    alongside the value, and reused on subsequent resolves.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use agent_fw_algebra::KVStore;
use agent_fw_core::TenantId;
use agent_fw_reference::{
    KvReferenceRegistry, ReferenceError as FrameworkReferenceError,
    ReferenceRegistry as FrameworkReferenceRegistry, StoredReference,
};
use jsonschema::{Draft, JSONSchema};
use serde_json::Value as JsonValue;

use crate::{ArtifactRef, ReferenceSpec};

/// JSON Schema dialect enforced for every [`ReferenceSpec`] schema.
///
/// `JSONSchema::compile` reads the `$schema` keyword to auto-detect the
/// draft; absent that keyword it falls back to `Draft::default()`, which
/// is **Draft 7** in `jsonschema 0.18`. Pinning the dialect at
/// compilation time makes 2020-12-only keywords (`prefixItems`,
/// `unevaluatedProperties`, etc.) enforced rather than silently ignored.
/// Authors who deliberately use an older draft can still override by
/// setting `"$schema": "http://json-schema.org/draft-07/schema#"` in
/// their spec.
const SCHEMA_DRAFT: Draft = Draft::Draft202012;

// ─── ReferenceError ─────────────────────────────────────────────────

/// Errors surfaced by [`ReferenceRegistry`].
#[derive(Debug, thiserror::Error)]
pub enum ReferenceError {
    /// `kind` does not match any `ReferenceSpec` registered at construction.
    #[error("unknown reference kind: {0}")]
    UnknownKind(String),
    /// A `ReferenceSpec.schema` failed to compile during
    /// [`ReferenceRegistry::new`]. Raised eagerly so misconfigured specs
    /// fail at runtime startup, not on the first `create` call.
    #[error("schema compilation failed for kind '{kind}': {error}")]
    SchemaInit { kind: String, error: String },
    /// A value passed to `create` did not validate against the registered
    /// schema. The value is never written to storage in this case.
    #[error("schema validation failed for kind '{kind}': {errors:?}")]
    SchemaValidation { kind: String, errors: Vec<String> },
    /// No body exists under the given `(tenant, kind, id)` triple. Also
    /// returned on cross-tenant resolve attempts (we don't disclose the
    /// existence of refs in other tenants).
    #[error("reference not found: kind={kind} id={id}")]
    NotFound { kind: String, id: String },
    /// Two `ReferenceSpec`s shared a `name` at construction.
    #[error("duplicate reference spec name: {0}")]
    DuplicateSpec(String),
    /// Underlying KV store error.
    #[error("kv error: {0}")]
    Storage(String),
}

impl From<FrameworkReferenceError> for ReferenceError {
    fn from(value: FrameworkReferenceError) -> Self {
        match value {
            FrameworkReferenceError::NotFound { kind, id } => ReferenceError::NotFound { kind, id },
            FrameworkReferenceError::Storage(message) => ReferenceError::Storage(message),
        }
    }
}

// ─── ReferenceRegistry ──────────────────────────────────────────────

struct CompiledSpec {
    schema: JSONSchema,
    ttl: Option<Duration>,
}

/// Typed reference registry composed from a list of [`ReferenceSpec`]s
/// and a [`KVStore`] interpreter.
pub struct ReferenceRegistry {
    specs: HashMap<String, CompiledSpec>,
    inner: Arc<dyn FrameworkReferenceRegistry>,
}

impl std::fmt::Debug for ReferenceRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut names: Vec<&str> = self.specs.keys().map(String::as_str).collect();
        names.sort();
        f.debug_struct("ReferenceRegistry")
            .field("kinds", &names)
            .finish_non_exhaustive()
    }
}

impl ReferenceRegistry {
    /// Construct a registry by precompiling every spec's JSON Schema.
    ///
    /// Raises [`ReferenceError::SchemaInit`] for a malformed schema and
    /// [`ReferenceError::DuplicateSpec`] for two specs sharing a name.
    pub fn new(specs: Vec<ReferenceSpec>, kv: Arc<dyn KVStore>) -> Result<Self, ReferenceError> {
        let mut compiled: HashMap<String, CompiledSpec> = HashMap::new();
        for spec in specs {
            if compiled.contains_key(&spec.name) {
                return Err(ReferenceError::DuplicateSpec(spec.name));
            }
            // Pin the dialect explicitly so a schema without `$schema`
            // doesn't fall back to Draft 7. See SCHEMA_DRAFT for the
            // rationale; callers who want Draft 7 must opt in via
            // `"$schema": "http://json-schema.org/draft-07/schema#"`.
            let schema = JSONSchema::options()
                .with_draft(SCHEMA_DRAFT)
                .compile(&spec.schema)
                .map_err(|e| ReferenceError::SchemaInit {
                    kind: spec.name.clone(),
                    error: e.to_string(),
                })?;
            compiled.insert(
                spec.name,
                CompiledSpec {
                    schema,
                    ttl: spec.ttl_ms.map(Duration::from_millis),
                },
            );
        }
        Ok(Self::from_compiled(
            compiled,
            Arc::new(KvReferenceRegistry::new(kv)),
        ))
    }

    fn from_compiled(
        specs: HashMap<String, CompiledSpec>,
        inner: Arc<dyn FrameworkReferenceRegistry>,
    ) -> Self {
        Self { specs, inner }
    }

    /// Whether the registry knows a given reference kind.
    pub fn has_kind(&self, kind: &str) -> bool {
        self.specs.contains_key(kind)
    }

    /// Create a new reference under the given tenant.
    ///
    /// Validates `value` against the spec's schema before computing the
    /// id or writing to the KV store. Returns the typed [`ArtifactRef`]
    /// suitable for embedding in plan bodies or tool results.
    pub async fn create(
        &self,
        kind: &str,
        value: JsonValue,
        glimpse: JsonValue,
        ctx: &TenantId,
    ) -> Result<ArtifactRef, ReferenceError> {
        let spec = self
            .specs
            .get(kind)
            .ok_or_else(|| ReferenceError::UnknownKind(kind.to_string()))?;

        // 1. Validate against schema BEFORE allocating an id or touching KV.
        if let Err(errors) = spec.schema.validate(&value) {
            let messages: Vec<String> = errors.map(|err| err.to_string()).collect();
            return Err(ReferenceError::SchemaValidation {
                kind: kind.to_string(),
                errors: messages,
            });
        }

        Ok(self
            .inner
            .create(kind, value, glimpse, ctx, spec.ttl)
            .await?)
    }

    /// Resolve a reference. Returns the full [`StoredReference`] body
    /// (value + glimpse + metadata).
    pub async fn resolve(
        &self,
        artifact: &ArtifactRef,
        ctx: &TenantId,
    ) -> Result<StoredReference, ReferenceError> {
        if !self.specs.contains_key(&artifact.kind) {
            return Err(ReferenceError::UnknownKind(artifact.kind.clone()));
        }
        Ok(self.inner.resolve(artifact, ctx).await?)
    }

    /// Look up only the glimpse for a reference. Same underlying KV
    /// fetch as `resolve`; returns a thinner shape for hot paths.
    pub async fn glimpse(
        &self,
        artifact: &ArtifactRef,
        ctx: &TenantId,
    ) -> Result<JsonValue, ReferenceError> {
        if !self.specs.contains_key(&artifact.kind) {
            return Err(ReferenceError::UnknownKind(artifact.kind.clone()));
        }
        Ok(self.inner.glimpse(artifact, ctx).await?)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    // `DashMapKVStore` (production interpreter, in agent-fw-interpreter)
    // enforces L7 (TTL expiry); the test-fixture `InMemoryKVStore`
    // documents itself as no-TTL. We need real TTL semantics for the
    // `ttl_expiry` acceptance test.
    use agent_fw_interpreter::DashMapKVStore;
    use serde_json::json;
    use std::time::Duration;

    fn product_set_spec() -> ReferenceSpec {
        ReferenceSpec {
            name: "ProductSet".into(),
            schema: json!({
                "type": "object",
                "required": ["product_ids"],
                "properties": {
                    "product_ids": {
                        "type": "array",
                        "items": {"type": "string"}
                    }
                }
            }),
            ttl_ms: None,
        }
    }

    fn scope_spec_with_ttl(ttl_ms: u64) -> ReferenceSpec {
        ReferenceSpec {
            name: "Scope".into(),
            schema: json!({
                "type": "object",
                "required": ["region"],
                "properties": {
                    "region": {"type": "string"}
                }
            }),
            ttl_ms: Some(ttl_ms),
        }
    }

    fn build_registry(specs: Vec<ReferenceSpec>) -> ReferenceRegistry {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        ReferenceRegistry::new(specs, kv).expect("registry init")
    }

    // ─── Schema dialect (Draft 2020-12) ──────────────────────────

    /// Proves the registry compiles schemas as Draft 2020-12 (not the
    /// crate's default Draft 7). `prefixItems` is a 2020-12-only
    /// keyword — under Draft 7 it would be silently ignored and the
    /// "wrong shape" array below would *incorrectly* validate.
    #[tokio::test]
    async fn schema_dialect_is_pinned_to_draft_2020_12() {
        let spec = ReferenceSpec {
            name: "Tuple".into(),
            schema: json!({
                "type": "array",
                "prefixItems": [
                    {"type": "string"},
                    {"type": "number"}
                ],
                "items": false
            }),
            ttl_ms: None,
        };
        let registry = build_registry(vec![spec]);
        let tenant = TenantId::new_unchecked("acme");

        // 2020-12 enforces prefixItems → correct tuple validates.
        registry
            .create("Tuple", json!(["hello", 42]), json!({}), &tenant)
            .await
            .expect("correct tuple should validate under Draft 2020-12");

        // 2020-12 enforces prefixItems → wrong shape is rejected.
        // Under Draft 7 this would pass silently (prefixItems unknown).
        let err = registry
            .create("Tuple", json!([42, "hello"]), json!({}), &tenant)
            .await
            .expect_err("wrong tuple shape must fail under Draft 2020-12");
        assert!(
            matches!(err, ReferenceError::SchemaValidation { .. }),
            "expected SchemaValidation, got {err:?}"
        );

        // `items: false` (no extra items) is also enforced.
        let err = registry
            .create("Tuple", json!(["x", 1, "extra"]), json!({}), &tenant)
            .await
            .expect_err("extra items must fail under Draft 2020-12");
        assert!(matches!(err, ReferenceError::SchemaValidation { .. }));
    }

    /// Sanity: an explicit Draft 7 `$schema` still works (the registry's
    /// default is "use 2020-12 when unspecified", not "ignore $schema").
    #[tokio::test]
    async fn explicit_draft_7_dollar_schema_is_honoured() {
        let spec = ReferenceSpec {
            name: "Draft7Legacy".into(),
            schema: json!({
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "object",
                "required": ["name"],
                "properties": {"name": {"type": "string"}}
            }),
            ttl_ms: None,
        };
        let registry = build_registry(vec![spec]);
        let tenant = TenantId::new_unchecked("acme");
        registry
            .create("Draft7Legacy", json!({"name": "ok"}), json!({}), &tenant)
            .await
            .expect("Draft 7 schema with explicit $schema should still compile");
    }

    // ─── Construction ─────────────────────────────────────────────

    #[test]
    fn new_rejects_duplicate_spec_names() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let err =
            ReferenceRegistry::new(vec![product_set_spec(), product_set_spec()], kv).unwrap_err();
        assert!(matches!(err, ReferenceError::DuplicateSpec(name) if name == "ProductSet"));
    }

    #[test]
    fn new_rejects_malformed_schema() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let bad = ReferenceSpec {
            name: "Bad".into(),
            // `type` must be a string or array of strings, not an integer.
            schema: json!({"type": 42}),
            ttl_ms: None,
        };
        let err = ReferenceRegistry::new(vec![bad], kv).unwrap_err();
        assert!(matches!(err, ReferenceError::SchemaInit { kind, .. } if kind == "Bad"));
    }

    // ─── Acceptance 1: tenant isolation ──────────────────────────

    #[tokio::test]
    async fn tenant_isolation() {
        let registry = build_registry(vec![product_set_spec()]);
        let t_a = TenantId::new_unchecked("tenant-a");
        let t_b = TenantId::new_unchecked("tenant-b");
        let value = json!({"product_ids": ["sku-1", "sku-2"]});
        let glimpse = json!({"n_products": 2});

        let artifact = registry
            .create("ProductSet", value.clone(), glimpse, &t_a)
            .await
            .unwrap();

        // Same artifact id used from tenant B → NotFound (not a different
        // body, not a "tenant mismatch" error variant — we don't disclose
        // that refs exist in other tenants).
        let err = registry.resolve(&artifact, &t_b).await.unwrap_err();
        assert!(matches!(err, ReferenceError::NotFound { .. }));

        // Tenant A still resolves it.
        let body = registry.resolve(&artifact, &t_a).await.unwrap();
        assert_eq!(body.value, value);
    }

    #[tokio::test]
    async fn same_value_under_different_tenants_yields_different_ids() {
        let registry = build_registry(vec![product_set_spec()]);
        let value = json!({"product_ids": ["sku-1"]});
        let glimpse = json!({"n_products": 1});

        let a = registry
            .create(
                "ProductSet",
                value.clone(),
                glimpse.clone(),
                &TenantId::new_unchecked("tenant-a"),
            )
            .await
            .unwrap();
        let b = registry
            .create(
                "ProductSet",
                value,
                glimpse,
                &TenantId::new_unchecked("tenant-b"),
            )
            .await
            .unwrap();
        assert_ne!(
            a.id, b.id,
            "tenant-scoped hash should differ across tenants"
        );
    }

    // ─── Acceptance 2: TTL expiry ─────────────────────────────────

    #[tokio::test]
    async fn ttl_expiry() {
        let registry = build_registry(vec![scope_spec_with_ttl(100)]);
        let tenant = TenantId::new_unchecked("acme");
        let artifact = registry
            .create(
                "Scope",
                json!({"region": "eu"}),
                json!({"region": "eu"}),
                &tenant,
            )
            .await
            .unwrap();

        // Immediately resolvable.
        registry.resolve(&artifact, &tenant).await.unwrap();

        // After 200ms, the TTL of 100ms has elapsed.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let err = registry.resolve(&artifact, &tenant).await.unwrap_err();
        assert!(matches!(err, ReferenceError::NotFound { .. }));
    }

    // ─── Acceptance 3: schema validation fires on `create` ────────

    #[tokio::test]
    async fn schema_validation_fires_on_create_not_resolve() {
        let registry = build_registry(vec![product_set_spec()]);
        let tenant = TenantId::new_unchecked("acme");
        // Missing required `product_ids` field.
        let err = registry
            .create("ProductSet", json!({"wrong": "shape"}), json!({}), &tenant)
            .await
            .unwrap_err();
        match err {
            ReferenceError::SchemaValidation { kind, errors } => {
                assert_eq!(kind, "ProductSet");
                assert!(!errors.is_empty(), "schema errors must be reported");
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }

        // Sanity: a fabricated artifact id never written to KV is NotFound,
        // not SchemaValidation — proving resolve doesn't re-validate.
        let bogus = ArtifactRef {
            kind: "ProductSet".into(),
            id: "did-not-exist".into(),
        };
        let err = registry.resolve(&bogus, &tenant).await.unwrap_err();
        assert!(matches!(err, ReferenceError::NotFound { .. }));
    }

    // ─── Acceptance 4: glimpse is cached, not recomputed ─────────

    #[tokio::test]
    async fn glimpse_cached_not_recomputed() {
        let registry = build_registry(vec![product_set_spec()]);
        let tenant = TenantId::new_unchecked("acme");
        let value = json!({"product_ids": ["a", "b", "c"]});
        // Caller's precomputed glimpse — a deliberately distinctive shape
        // so we can confirm it round-trips unchanged.
        let host_glimpse = json!({
            "n_products": 3,
            "host_marker": "from-python-lambda",
        });

        let artifact = registry
            .create("ProductSet", value, host_glimpse.clone(), &tenant)
            .await
            .unwrap();

        // Three resolves return the exact same glimpse object — there's
        // no recomputation path that could produce a different value.
        for _ in 0..3 {
            let g = registry.glimpse(&artifact, &tenant).await.unwrap();
            assert_eq!(g, host_glimpse);
        }

        // resolve() also returns the same glimpse field as part of the body.
        let body = registry.resolve(&artifact, &tenant).await.unwrap();
        assert_eq!(body.glimpse, host_glimpse);
    }

    // ─── Unknown-kind paths ───────────────────────────────────────

    #[tokio::test]
    async fn unknown_kind_on_create_errors() {
        let registry = build_registry(vec![product_set_spec()]);
        let tenant = TenantId::new_unchecked("acme");
        let err = registry
            .create("MissingKind", json!({}), json!({}), &tenant)
            .await
            .unwrap_err();
        assert!(matches!(err, ReferenceError::UnknownKind(k) if k == "MissingKind"));
    }

    #[tokio::test]
    async fn unknown_kind_on_resolve_errors() {
        let registry = build_registry(vec![product_set_spec()]);
        let tenant = TenantId::new_unchecked("acme");
        let bogus = ArtifactRef {
            kind: "MissingKind".into(),
            id: "anything".into(),
        };
        let err = registry.resolve(&bogus, &tenant).await.unwrap_err();
        assert!(matches!(err, ReferenceError::UnknownKind(k) if k == "MissingKind"));
    }

    // ─── Determinism — same value yields same id within a tenant ──

    #[tokio::test]
    async fn same_value_within_tenant_produces_same_id() {
        let registry = build_registry(vec![product_set_spec()]);
        let tenant = TenantId::new_unchecked("acme");
        let value = json!({"product_ids": ["x", "y"]});
        let glimpse = json!({"n_products": 2});

        let first = registry
            .create("ProductSet", value.clone(), glimpse.clone(), &tenant)
            .await
            .unwrap();
        let second = registry
            .create("ProductSet", value, glimpse, &tenant)
            .await
            .unwrap();
        assert_eq!(
            first.id, second.id,
            "content-addressed id must be deterministic"
        );
    }
}
