//! Plan protocol on `Plan<HarnessAction>` (plan registry, C3).
//!
//! This module fills the gap between the planner emitting a JSON body
//! and the executor running an approved action sequence. It composes
//! existing primitives:
//!
//! - `jsonschema` for
//!   planner-output validation against [`PlanSpec::schema`].
//! - [`agent_fw_plan::Plan<A>`] for the fixed framework lifecycle
//!   (`Draft → Approved → Executing → Executed | Failed`). Transitions
//!   are owned by the state machine; this module never opens new ones.
//! - [`KVStore`] for tenant-scoped plan persistence.
//! - [`ReferenceRegistry`] (reference registry) for the reference-hydration step
//!   that pre-resolves every [`ArtifactRef`] in an approved plan before
//!   the customer's `ActionDispatcher` runs.
//!
//!
//! - **Customer `ActionDispatcher` injection lives in C4**, not here.
//!   This module ships [`HydratingDispatcher<D>`] as a standalone
//!   combinator; the runtime's executor wiring lands in
//!   `flowai-runtime/src/lib.rs` only as a future C4 concern.
//! - **Hydrated context is a runtime-owned concrete struct**
//!   ([`HarnessActionContext`]). The wrapper is mechanical; no
//!   TypeMap, no generic over the user's ctx.
//! - **Planner output convention:** the body validates against
//!   [`PlanSpec::schema`]; after validation the registry extracts the
//!   top-level `"actions"` key into the plan's [`ActionSeq`] and
//!   persists every remaining top-level field into [`Plan::context`]
//!   verbatim (so customer-defined `rationale`, `scope_ref`, etc.
//!   survive the round-trip and are available to the UI / executor).
//! - **Action wire shape is flat.** Customer plan schemas use a flat
//!   discriminated-union per §13.2 of the Harness abstractions doc,
//!   e.g. `{ "kind": "price_change", "product_id": "p", "new_price": 9.99 }`.
//!   The runtime normalizes each action into [`HarnessAction`] by
//!   keeping `kind` and an optional `references` array (the only
//!   framework-reserved keys) and folding every remaining field into
//!   `payload`. The persisted shape is therefore always the canonical
//!   `{ kind, payload, references }` even when the planner emitted a
//!   flat object. This module is shape-agnostic over `payload`'s
//!   contents — the spec schema is responsible for that.
//!
//! # Acceptance criteria (plan registry)
//!
//! 1. Planner output that fails schema validation never reaches the
//!    executor — `propose()` errors with [`PlanProtocolError::SchemaValidation`]
//!    and no KV write occurs.
//! 2. A `Draft` plan cannot be dispatched to the executor until
//!    approval is recorded — enforced by [`agent_fw_plan::PlanExecutor`]
//!    (state check at `executor.rs:163`) and pre-dispatch approval's
//!    `GatedPlanExecutor`. This module only persists the Draft.
//! 3. Invalid framework transitions are rejected — enforced by
//!    `Plan::approve/start/complete/fail` returning
//!    [`agent_fw_plan::plan::Rejected`] with a `TransitionError`.
//! 4. References inside an approved plan are pre-resolved before
//!    executor dispatch — enforced by [`HydratingDispatcher`].

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use agent_fw_algebra::{KVError, KVStore, KVStoreExt};
use agent_fw_core::{PlanId, TenantId, UserId};
use agent_fw_plan::action::{action_seq_from_vec, ActionSeq};
use agent_fw_plan::context::PlanContext;
use agent_fw_plan::executor::ActionDispatcher;
use agent_fw_plan::persist::plan_key;
use agent_fw_plan::plan::{create_plan, ExecutionResult, Plan, PlanStatus, TransitionError};
use async_trait::async_trait;
use jsonschema::{Draft, JSONSchema};
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::references::{ReferenceError, ReferenceRegistry};
use crate::{ArtifactRef, HarnessAction, PlanSpec};

/// KV key prefix for stored plan bodies. Tenant scoping is the KV
/// layer's responsibility (KVStore L9).
const KV_PREFIX: &str = "plan";

/// JSON Schema dialect pinned for every [`PlanSpec`] schema (mirrors
/// the reference registry fix on the reference registry). See `references.rs`
/// for the rationale.
const SCHEMA_DRAFT: Draft = Draft::Draft202012;

// ─── PlanProtocolError ──────────────────────────────────────────────

/// Errors surfaced by [`PlanRegistry`].
#[derive(Debug, thiserror::Error)]
pub enum PlanProtocolError {
    /// `spec_name` does not match any [`PlanSpec`] registered at
    /// construction.
    #[error("unknown plan spec: {0}")]
    UnknownSpec(String),
    /// A [`PlanSpec::schema`] failed to compile during
    /// [`PlanRegistry::new`]. Raised eagerly so misconfigured specs
    /// fail at runtime startup, not on the first `propose` call.
    #[error("schema compilation failed for spec '{spec}': {error}")]
    SchemaInit { spec: String, error: String },
    /// The planner output failed validation against the spec schema.
    /// No KV write occurs — the value never reaches persistence.
    #[error("schema validation failed for spec '{spec}': {errors:?}")]
    SchemaValidation { spec: String, errors: Vec<String> },
    /// The body validated but `body["actions"]` was missing entirely.
    /// Documented convention: the planner output must include a
    /// top-level `"actions"` array.
    #[error("planner output for spec '{spec}' is missing the required `actions` field")]
    MissingActions { spec: String },
    /// `body["actions"]` exists but is not a JSON array.
    #[error("`actions` for spec '{spec}' must be a JSON array")]
    ActionsNotAnArray { spec: String },
    /// `body["actions"]` is present and an array, but empty.
    /// [`ActionSeq`] is `NonEmpty`-backed; we surface this explicitly
    /// rather than let `action_seq_from_vec` return `None` opaquely.
    #[error("planner output for spec '{spec}' produced an empty action sequence")]
    EmptyActions { spec: String },
    /// An item in `body["actions"]` failed to deserialise into
    /// [`HarnessAction`]. The spec schema may not strictly constrain
    /// individual actions; this catches the structural mismatch
    /// regardless.
    #[error("action #{index} for spec '{spec}' failed to decode: {error}")]
    ActionDecode {
        spec: String,
        index: usize,
        error: String,
    },
    /// No plan body exists at the given `(tenant, plan_id)`. Also
    /// returned on cross-tenant load attempts (we don't disclose the
    /// existence of plans in other tenants).
    #[error("plan not found: {plan_id}")]
    NotFound { plan_id: PlanId },
    /// Two [`PlanSpec`]s shared a `name` at construction.
    #[error("duplicate plan spec name: {0}")]
    DuplicateSpec(String),
    /// Underlying KV store error.
    #[error("kv error: {0}")]
    Storage(String),
    /// The Plan state machine rejected a transition. Propagated from
    /// [`Plan::approve`] / `start` / `complete` / `fail`.
    #[error("plan transition error: {0}")]
    Transition(#[from] TransitionError),
}

impl From<KVError> for PlanProtocolError {
    fn from(e: KVError) -> Self {
        PlanProtocolError::Storage(e.to_string())
    }
}

// ─── PlanRegistry ───────────────────────────────────────────────────

struct CompiledPlanSpec {
    schema: JSONSchema,
    display_aliases: HashMap<PlanStatus, String>,
}

/// Typed plan registry composed from a list of [`PlanSpec`]s, a
/// [`KVStore`], and the [`ReferenceRegistry`] (held for hydration in
/// the dispatcher combinator; the registry itself doesn't touch
/// references during `propose` / `load`).
pub struct PlanRegistry {
    specs: HashMap<String, CompiledPlanSpec>,
    kv: Arc<dyn KVStore>,
    references: Arc<ReferenceRegistry>,
}

impl std::fmt::Debug for PlanRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut names: Vec<&str> = self.specs.keys().map(String::as_str).collect();
        names.sort();
        f.debug_struct("PlanRegistry")
            .field("specs", &names)
            .finish_non_exhaustive()
    }
}

impl PlanRegistry {
    /// Construct a registry by precompiling every spec's JSON Schema
    /// and folding its display aliases into a lookup table.
    pub fn new(
        specs: Vec<PlanSpec>,
        kv: Arc<dyn KVStore>,
        references: Arc<ReferenceRegistry>,
    ) -> Result<Self, PlanProtocolError> {
        let mut compiled: HashMap<String, CompiledPlanSpec> = HashMap::new();
        for spec in specs {
            if compiled.contains_key(&spec.name) {
                return Err(PlanProtocolError::DuplicateSpec(spec.name));
            }
            let schema = JSONSchema::options()
                .with_draft(SCHEMA_DRAFT)
                .compile(&spec.schema)
                .map_err(|e| PlanProtocolError::SchemaInit {
                    spec: spec.name.clone(),
                    error: e.to_string(),
                })?;
            let mut aliases: HashMap<PlanStatus, String> = HashMap::new();
            for alias in spec.display_aliases {
                aliases.insert(alias.status, alias.alias);
            }
            compiled.insert(
                spec.name,
                CompiledPlanSpec {
                    schema,
                    display_aliases: aliases,
                },
            );
        }
        Ok(Self {
            specs: compiled,
            kv,
            references,
        })
    }

    /// Whether the registry knows a given plan spec.
    pub fn has_spec(&self, name: &str) -> bool {
        self.specs.contains_key(name)
    }

    /// Look up the display alias for a `(spec_name, status)` pair.
    /// Returns `None` if the spec is unknown or the status has no
    /// alias configured.
    pub fn display_alias(&self, spec_name: &str, status: PlanStatus) -> Option<&str> {
        self.specs
            .get(spec_name)
            .and_then(|c| c.display_aliases.get(&status))
            .map(String::as_str)
    }

    /// Validate the planner's JSON body against the spec, extract
    /// `body.actions`, build `Plan<HarnessAction>` in `Draft`, persist.
    ///
    /// `body` is consumed by reference for validation; the `actions`
    /// array is then taken out by mutating a clone. (We can't `&mut`
    /// the caller's value since validation needs an immutable view.)
    pub async fn propose(
        &self,
        spec_name: &str,
        plan_id: PlanId,
        body: JsonValue,
        tenant: &TenantId,
    ) -> Result<Plan<HarnessAction>, PlanProtocolError> {
        let spec = self
            .specs
            .get(spec_name)
            .ok_or_else(|| PlanProtocolError::UnknownSpec(spec_name.to_string()))?;

        // 1. Schema-validate the full body before any extraction or KV write.
        if let Err(errors) = spec.schema.validate(&body) {
            let messages: Vec<String> = errors.map(|err| err.to_string()).collect();
            return Err(PlanProtocolError::SchemaValidation {
                spec: spec_name.to_string(),
                errors: messages,
            });
        }

        // 2. Extract body["actions"] — the convention is fixed at the
        //    top level. Spec schemas that require `actions` will have
        //    already enforced presence and type; this code catches
        //    schemas that don't require it but still hit propose().
        let actions_value =
            body.get("actions")
                .ok_or_else(|| PlanProtocolError::MissingActions {
                    spec: spec_name.to_string(),
                })?;
        let actions_array =
            actions_value
                .as_array()
                .ok_or_else(|| PlanProtocolError::ActionsNotAnArray {
                    spec: spec_name.to_string(),
                })?;

        // 3. Normalise each item into HarnessAction. Customer plan
        //    schemas describe actions in a flat shape (per §13.2 of the
        //    Harness abstractions doc); `normalize_action` folds every
        //    non-framework field into `payload` and preserves the
        //    optional `references` array.
        let mut actions: Vec<HarnessAction> = Vec::with_capacity(actions_array.len());
        for (index, item) in actions_array.iter().enumerate() {
            actions.push(normalize_action(spec_name, index, item)?);
        }

        // 4. Wrap in NonEmpty / ActionSeq. ActionSeq enforces len >= 1.
        let seq = action_seq_from_vec(actions).ok_or_else(|| PlanProtocolError::EmptyActions {
            spec: spec_name.to_string(),
        })?;

        // 5. Build the plan in Draft state, attach every non-`actions`
        //    top-level body field to PlanContext (so customer-defined
        //    fields like `rationale`, `scope_ref`, review metadata
        //    survive the round-trip), and persist.
        let mut plan = create_plan(plan_id.clone(), tenant.clone(), seq);
        plan.context = plan_context_from_body(&body);
        self.kv
            .put(tenant.as_str(), &plan_key(KV_PREFIX, &plan_id), &plan, None)
            .await?;
        Ok(plan)
    }

    /// Load a stored plan by id, scoped to the given tenant. Returns
    /// `Ok(None)` when no plan exists at the `(tenant, plan_id)`
    /// coordinate or when a plan is found but belongs to another
    /// tenant (defence-in-depth, mirroring [`ReferenceRegistry::resolve`]).
    pub async fn load(
        &self,
        plan_id: &PlanId,
        tenant: &TenantId,
    ) -> Result<Option<Plan<HarnessAction>>, PlanProtocolError> {
        let plan: Option<Plan<HarnessAction>> = self
            .kv
            .get::<Plan<HarnessAction>>(tenant.as_str(), &plan_key(KV_PREFIX, plan_id))
            .await?;
        let plan = match plan {
            None => return Ok(None),
            Some(p) => p,
        };
        // Defence in depth: the KV trait's L9 already isolates by
        // tenant, but checking the persisted `owner` field guards
        // against any interpreter that accidentally short-circuits
        // tenant scoping.
        if &plan.owner != tenant {
            return Ok(None);
        }
        Ok(Some(plan))
    }

    /// Mark a draft plan approved and persist it.
    ///
    /// Returns `Ok(true)` when this call performed the approval transition,
    /// `Ok(false)` when the plan was already past `Draft`.
    pub(crate) async fn approve_for_execution(
        &self,
        plan_id: &PlanId,
        tenant: &TenantId,
        approver: UserId,
    ) -> Result<bool, PlanProtocolError> {
        let key = plan_key(KV_PREFIX, plan_id);
        let Some(plan): Option<Plan<HarnessAction>> = self
            .kv
            .get::<Plan<HarnessAction>>(tenant.as_str(), &key)
            .await?
        else {
            return Err(PlanProtocolError::NotFound {
                plan_id: plan_id.clone(),
            });
        };
        if &plan.owner != tenant {
            return Err(PlanProtocolError::NotFound {
                plan_id: plan_id.clone(),
            });
        }
        if plan.status != PlanStatus::Draft {
            return Ok(false);
        }

        let approved = plan
            .approve(approver)
            .map_err(|rejected| PlanProtocolError::Transition(rejected.error))?;
        self.kv.put(tenant.as_str(), &key, &approved, None).await?;
        Ok(true)
    }

    /// Borrowed access to the reference registry the registry was
    /// built with. Mostly useful so [`HydratingDispatcher`] doesn't
    /// have to be threaded its own clone of the same `Arc`.
    pub fn references(&self) -> &Arc<ReferenceRegistry> {
        &self.references
    }
}

// ─── HarnessActionContext + HydratingDispatcher ─────────────────────

/// Context handed to the customer's `ActionDispatcher` after
/// reference hydration. Every distinct [`ArtifactRef`] found in the
/// plan's actions has been pre-resolved into a `serde_json::Value`
/// keyed by the artifact's `(kind, id)` pair.
#[derive(Debug, Clone, Default)]
pub struct HarnessActionContext {
    pub resolved_refs: HashMap<ArtifactRef, JsonValue>,
}

/// `ActionDispatcher` combinator that pre-resolves every
/// [`ArtifactRef`] referenced by a plan's actions, then delegates to
/// an inner dispatcher with the resolved bodies attached to its
/// context.
///
/// # Hydration semantics
///
/// - References are deduplicated by `(kind, id)` before resolution
///   — a single ref appearing in N actions costs one
///   [`ReferenceRegistry::resolve`] call, not N.
/// - Resolution runs in parallel via `futures::future::join_all`.
/// - On any [`ReferenceError::NotFound`] the dispatcher returns
///   [`HydrationError::MissingReference`]; the inner dispatcher is
///   never called. (A plan referencing a non-existent artifact is
///   not eligible for dispatch.)
/// - Tenant scoping is fixed at construction (matches the runtime's
///   resource id), so the dispatcher and the customer can't disagree
///   on which tenant is reading.
pub struct HydratingDispatcher<D> {
    inner: D,
    references: Arc<ReferenceRegistry>,
    tenant: TenantId,
}

impl<D> HydratingDispatcher<D> {
    pub fn new(inner: D, references: Arc<ReferenceRegistry>, tenant: TenantId) -> Self {
        Self {
            inner,
            references,
            tenant,
        }
    }
}

#[async_trait]
impl<D> ActionDispatcher for HydratingDispatcher<D>
where
    D: ActionDispatcher<Action = HarnessAction, Context = HarnessActionContext>,
{
    type Action = HarnessAction;
    /// The caller passes `()`; we build the real context from the
    /// resolved refs and hand it to the inner dispatcher.
    type Context = ();
    type Error = HydrationError<D::Error>;

    async fn dispatch(
        &self,
        actions: &ActionSeq<HarnessAction>,
        _ctx: &(),
    ) -> Result<ExecutionResult, Self::Error> {
        // 1. Deduplicate ArtifactRefs across actions[*].references.
        let mut seen: HashSet<ArtifactRef> = HashSet::new();
        let mut to_resolve: Vec<ArtifactRef> = Vec::new();
        for action in actions.iter() {
            for reference in &action.references {
                if seen.insert(reference.clone()) {
                    to_resolve.push(reference.clone());
                }
            }
        }

        // 2. Resolve in parallel. We resolve into the runtime-internal
        //    `StoredReference`; the customer only sees the `value`
        //    field via the `resolved_refs` map.
        let futures = to_resolve.iter().map(|artifact| {
            let registry = self.references.clone();
            let tenant = self.tenant.clone();
            let artifact = artifact.clone();
            async move {
                let body = registry.resolve(&artifact, &tenant).await;
                (artifact, body)
            }
        });
        let resolved = futures::future::join_all(futures).await;

        // 3. Fold results, converting NotFound into MissingReference
        //    and any other registry error into Reference(_).
        let mut resolved_refs: HashMap<ArtifactRef, JsonValue> =
            HashMap::with_capacity(resolved.len());
        for (artifact, result) in resolved {
            match result {
                Ok(stored) => {
                    resolved_refs.insert(artifact, stored.value);
                }
                Err(ReferenceError::NotFound { kind, id }) => {
                    return Err(HydrationError::MissingReference { kind, id });
                }
                Err(other) => return Err(HydrationError::Reference(other)),
            }
        }

        // 4. Delegate to the inner dispatcher with the hydrated context.
        let ctx = HarnessActionContext { resolved_refs };
        self.inner
            .dispatch(actions, &ctx)
            .await
            .map_err(HydrationError::Inner)
    }
}

/// Errors surfaced by [`HydratingDispatcher`].
#[derive(Debug, thiserror::Error)]
pub enum HydrationError<E: std::error::Error> {
    /// A plan referenced an [`ArtifactRef`] that the registry could
    /// not resolve. The inner dispatcher was never called.
    #[error("reference required by plan is missing: kind={kind} id={id}")]
    MissingReference { kind: String, id: String },
    /// Non-`NotFound` error from the reference registry.
    #[error("reference registry error: {0}")]
    Reference(#[from] ReferenceError),
    /// The inner dispatcher failed after hydration.
    #[error("inner dispatcher failed: {0}")]
    Inner(E),
}

// ─── Body / action normalisation helpers ────────────────────────────

/// Framework-reserved keys inside a flat action object. Every other
/// field is folded into [`HarnessAction::payload`] during normalisation.
const ACTION_RESERVED_KEYS: &[&str] = &["kind", "references"];

/// Normalise one item from `body["actions"]` into a [`HarnessAction`].
///
/// Customer plan schemas describe actions in a flat discriminated-union
/// shape (e.g. `{ "kind": "price_change", "product_id": "p", "new_price": 9.99 }`).
/// Plain `HarnessAction::deserialize` would silently drop those extra
/// fields — serde keeps only `kind`, `payload`, and `references` and
/// discards the rest. This helper instead:
///
/// 1. Requires a JSON object with a string `kind`.
/// 2. Pulls `references` out as `Vec<ArtifactRef>` if present (defaults
///    to empty otherwise).
/// 3. Folds every other field into `HarnessAction::payload` verbatim,
///    so customer-defined data survives the round-trip into the
///    executor.
///
/// The persisted shape is therefore always `{ kind, payload, references }`,
/// regardless of whether the planner emitted the flat shape or the
/// already-canonical shape.
fn normalize_action(
    spec_name: &str,
    index: usize,
    item: &JsonValue,
) -> Result<HarnessAction, PlanProtocolError> {
    let obj = item
        .as_object()
        .ok_or_else(|| PlanProtocolError::ActionDecode {
            spec: spec_name.to_string(),
            index,
            error: "action must be a JSON object".into(),
        })?;

    let kind = obj
        .get("kind")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| PlanProtocolError::ActionDecode {
            spec: spec_name.to_string(),
            index,
            error: "action is missing required string field `kind`".into(),
        })?
        .to_string();

    let references = match obj.get("references") {
        None => Vec::new(),
        Some(value) => {
            Vec::<ArtifactRef>::deserialize(value).map_err(|e| PlanProtocolError::ActionDecode {
                spec: spec_name.to_string(),
                index,
                error: format!("`references` did not match `[ArtifactRef]`: {e}"),
            })?
        }
    };

    let mut payload_map = serde_json::Map::with_capacity(obj.len());
    for (key, value) in obj.iter() {
        if ACTION_RESERVED_KEYS.contains(&key.as_str()) {
            continue;
        }
        payload_map.insert(key.clone(), value.clone());
    }

    Ok(HarnessAction {
        kind,
        payload: JsonValue::Object(payload_map),
        references,
    })
}

/// Build a [`PlanContext`] from every top-level field of the planner
/// body except `actions`. Used so customer-defined plan metadata
/// (`rationale`, `scope_ref`, review hints, etc.) is durable on the
/// persisted plan and addressable from the UI / executor through
/// [`Plan::context`]. Non-object bodies (which the spec schema should
/// already have rejected) yield an empty context.
fn plan_context_from_body(body: &JsonValue) -> PlanContext {
    let Some(map) = body.as_object() else {
        return PlanContext::new();
    };
    let mut context = PlanContext::new();
    for (key, value) in map.iter() {
        if key == "actions" {
            continue;
        }
        context.set(key.clone(), value.clone());
    }
    context
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::references::ReferenceRegistry;
    use crate::{PlanDisplayAlias, ReferenceSpec};
    use agent_fw_interpreter::DashMapKVStore;
    use agent_fw_plan::action::single_action;
    use agent_fw_plan::executor::PlanExecutionError;
    use agent_fw_plan::plan::{PlanError, PlanStatus};
    use agent_fw_plan::PlanExecutor;
    use serde_json::json;

    // ─── Fixtures ─────────────────────────────────────────────────

    fn scenario_plan_spec() -> PlanSpec {
        PlanSpec {
            name: "ScenarioPlan".into(),
            schema: json!({
                "type": "object",
                "required": ["actions"],
                "properties": {
                    "actions": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["kind"],
                            "properties": {
                                "kind": {"type": "string"}
                            }
                        }
                    },
                    "rationale": {"type": "string"}
                }
            }),
            display_aliases: vec![PlanDisplayAlias {
                status: PlanStatus::Draft,
                alias: "pending_approval".into(),
            }],
        }
    }

    /// Spec without `actions` in required fields, so we can hit the
    /// `MissingActions` branch.
    fn loose_plan_spec(name: &str) -> PlanSpec {
        PlanSpec {
            name: name.into(),
            schema: json!({"type": "object"}),
            display_aliases: vec![],
        }
    }

    fn product_set_ref_spec() -> ReferenceSpec {
        ReferenceSpec {
            name: "ProductSet".into(),
            schema: json!({
                "type": "object",
                "required": ["product_ids"],
                "properties": {
                    "product_ids": {"type": "array", "items": {"type": "string"}}
                }
            }),
            ttl_ms: None,
        }
    }

    fn build_registries() -> (Arc<DashMapKVStore>, Arc<ReferenceRegistry>, PlanRegistry) {
        let kv = Arc::new(DashMapKVStore::new());
        let kv_dyn: Arc<dyn KVStore> = kv.clone();
        let references = Arc::new(
            ReferenceRegistry::new(vec![product_set_ref_spec()], kv_dyn.clone())
                .expect("reference registry init"),
        );
        let plans = PlanRegistry::new(
            vec![scenario_plan_spec()],
            kv_dyn.clone(),
            references.clone(),
        )
        .expect("plan registry init");
        (kv, references, plans)
    }

    // ─── Construction ─────────────────────────────────────────────

    #[test]
    fn new_rejects_duplicate_spec_names() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let references = Arc::new(ReferenceRegistry::new(vec![], kv.clone()).unwrap());
        let err = PlanRegistry::new(
            vec![scenario_plan_spec(), scenario_plan_spec()],
            kv,
            references,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            PlanProtocolError::DuplicateSpec(name) if name == "ScenarioPlan"
        ));
    }

    #[test]
    fn new_rejects_malformed_schema() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let references = Arc::new(ReferenceRegistry::new(vec![], kv.clone()).unwrap());
        let bad = PlanSpec {
            name: "Bad".into(),
            schema: json!({"type": 42}),
            display_aliases: vec![],
        };
        let err = PlanRegistry::new(vec![bad], kv, references).unwrap_err();
        assert!(matches!(err, PlanProtocolError::SchemaInit { spec, .. } if spec == "Bad"));
    }

    // ─── Acceptance 1: schema validation blocks persistence ──────

    #[tokio::test]
    async fn schema_validation_fail_blocks_persistence() {
        let (kv, _refs, registry) = build_registries();
        let tenant = TenantId::new_unchecked("acme");
        // Missing required "actions" — schema requires it.
        let err = registry
            .propose(
                "ScenarioPlan",
                PlanId::new_unchecked("plan-1"),
                json!({"rationale": "no actions"}),
                &tenant,
            )
            .await
            .unwrap_err();
        match err {
            PlanProtocolError::SchemaValidation { spec, errors } => {
                assert_eq!(spec, "ScenarioPlan");
                assert!(!errors.is_empty());
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }

        // No KV write happened.
        let body: Option<Plan<HarnessAction>> = kv
            .get::<Plan<HarnessAction>>("acme", "plan:plan-1")
            .await
            .unwrap();
        assert!(
            body.is_none(),
            "rejected propose must not have written to KV"
        );
    }

    // ─── Acceptance 2: Draft cannot be executed directly ─────────

    #[tokio::test]
    async fn draft_cannot_be_executed_directly() {
        use async_trait::async_trait;

        let (kv_arc, _refs, registry) = build_registries();
        let kv_dyn: Arc<dyn KVStore> = kv_arc.clone();
        let tenant = TenantId::new_unchecked("acme");
        let plan_id = PlanId::new_unchecked("plan-draft");
        let body = json!({
            "actions": [
                {"kind": "price_change", "payload": {"new_price": 9.99}, "references": []}
            ]
        });
        registry
            .propose("ScenarioPlan", plan_id.clone(), body, &tenant)
            .await
            .unwrap();

        // Dispatch must fail with InvalidState(Draft) because the
        // gate (pre-dispatch approval) hasn't run.
        struct NoopDispatcher;
        #[derive(Debug)]
        struct NeverError;
        impl std::fmt::Display for NeverError {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("unreachable")
            }
        }
        impl std::error::Error for NeverError {}
        #[async_trait]
        impl ActionDispatcher for NoopDispatcher {
            type Action = HarnessAction;
            type Context = ();
            type Error = NeverError;
            async fn dispatch(
                &self,
                _actions: &ActionSeq<HarnessAction>,
                _ctx: &(),
            ) -> Result<ExecutionResult, NeverError> {
                panic!("dispatcher must never be called on a Draft plan");
            }
        }

        let executor = PlanExecutor::new(kv_dyn.as_ref(), &tenant, &NoopDispatcher, KV_PREFIX);
        let err = executor.execute(&plan_id, &()).await.unwrap_err();
        assert!(matches!(
            err,
            PlanExecutionError::InvalidState(PlanStatus::Draft)
        ));
    }

    // ─── Acceptance 3: invalid state transitions rejected ────────

    #[tokio::test]
    async fn invalid_state_transitions_rejected_after_round_trip() {
        let (_kv, _refs, registry) = build_registries();
        let tenant = TenantId::new_unchecked("acme");
        let plan_id = PlanId::new_unchecked("plan-trans");
        let body = json!({
            "actions": [
                {"kind": "noop", "payload": {}, "references": []}
            ]
        });
        let plan = registry
            .propose("ScenarioPlan", plan_id.clone(), body, &tenant)
            .await
            .unwrap();
        // Round-trip through KV.
        let loaded = registry.load(&plan_id, &tenant).await.unwrap().unwrap();
        assert_eq!(loaded.status, PlanStatus::Draft);
        // Calling complete() on a Draft (skipping Approved/Executing)
        // must be rejected by the state machine.
        let err = loaded.complete(ExecutionResult::default()).unwrap_err();
        assert!(matches!(
            err.error,
            TransitionError::InvalidState {
                expected: PlanStatus::Executing,
                actual: PlanStatus::Draft,
            }
        ));
        // Sanity: the original plan returned by propose() also rejects.
        let err = plan.fail(PlanError::new("nope")).unwrap_err();
        assert!(matches!(err.error, TransitionError::InvalidState { .. }));
    }

    // ─── Acceptance 4: references pre-resolved before dispatch ───

    #[tokio::test]
    async fn references_pre_resolved_before_dispatch() {
        use async_trait::async_trait;
        use std::sync::Mutex;

        let (_kv, refs, _registry) = build_registries();
        let tenant = TenantId::new_unchecked("acme");

        // Create two distinct refs in the registry so the dispatcher
        // can find resolved bodies under both artifact ids.
        let product_set_a = refs
            .create(
                "ProductSet",
                json!({"product_ids": ["sku-1"]}),
                json!({"n_products": 1}),
                &tenant,
            )
            .await
            .unwrap();
        let product_set_b = refs
            .create(
                "ProductSet",
                json!({"product_ids": ["sku-2", "sku-3"]}),
                json!({"n_products": 2}),
                &tenant,
            )
            .await
            .unwrap();

        // Build an ActionSeq referencing both.
        let actions = action_seq_from_vec(vec![
            HarnessAction {
                kind: "act_a".into(),
                payload: json!({}),
                references: vec![product_set_a.clone()],
            },
            HarnessAction {
                kind: "act_b".into(),
                payload: json!({}),
                references: vec![product_set_b.clone()],
            },
        ])
        .unwrap();

        // Recording inner dispatcher: captures the ctx it was called with.
        struct RecordingInner {
            seen: Mutex<Option<HarnessActionContext>>,
        }
        #[derive(Debug)]
        struct NoErr;
        impl std::fmt::Display for NoErr {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("noerr")
            }
        }
        impl std::error::Error for NoErr {}
        #[async_trait]
        impl ActionDispatcher for RecordingInner {
            type Action = HarnessAction;
            type Context = HarnessActionContext;
            type Error = NoErr;
            async fn dispatch(
                &self,
                _actions: &ActionSeq<HarnessAction>,
                ctx: &HarnessActionContext,
            ) -> Result<ExecutionResult, NoErr> {
                *self.seen.lock().unwrap() = Some(ctx.clone());
                Ok(ExecutionResult {
                    entities_affected: 0,
                    summary: None,
                    details: None,
                })
            }
        }
        let inner = RecordingInner {
            seen: Mutex::new(None),
        };
        let dispatcher = HydratingDispatcher::new(inner, refs.clone(), tenant.clone());
        dispatcher.dispatch(&actions, &()).await.unwrap();

        let ctx = dispatcher.inner.seen.lock().unwrap().take().unwrap();
        assert_eq!(ctx.resolved_refs.len(), 2, "both refs resolved");
        assert_eq!(
            ctx.resolved_refs.get(&product_set_a).unwrap(),
            &json!({"product_ids": ["sku-1"]})
        );
        assert_eq!(
            ctx.resolved_refs.get(&product_set_b).unwrap(),
            &json!({"product_ids": ["sku-2", "sku-3"]})
        );
    }

    // ─── Shape: missing / empty actions ──────────────────────────

    #[tokio::test]
    async fn missing_actions_field_errors() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let references = Arc::new(ReferenceRegistry::new(vec![], kv.clone()).unwrap());
        let registry = PlanRegistry::new(vec![loose_plan_spec("Loose")], kv, references).unwrap();
        let tenant = TenantId::new_unchecked("acme");
        let err = registry
            .propose(
                "Loose",
                PlanId::new_unchecked("p"),
                json!({"unrelated": true}),
                &tenant,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            PlanProtocolError::MissingActions { spec } if spec == "Loose"
        ));
    }

    #[tokio::test]
    async fn empty_actions_array_errors() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let references = Arc::new(ReferenceRegistry::new(vec![], kv.clone()).unwrap());
        let registry = PlanRegistry::new(vec![loose_plan_spec("Loose")], kv, references).unwrap();
        let tenant = TenantId::new_unchecked("acme");
        let err = registry
            .propose(
                "Loose",
                PlanId::new_unchecked("p"),
                json!({"actions": []}),
                &tenant,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            PlanProtocolError::EmptyActions { spec } if spec == "Loose"
        ));
    }

    #[tokio::test]
    async fn actions_not_an_array_errors() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let references = Arc::new(ReferenceRegistry::new(vec![], kv.clone()).unwrap());
        let registry = PlanRegistry::new(vec![loose_plan_spec("Loose")], kv, references).unwrap();
        let tenant = TenantId::new_unchecked("acme");
        let err = registry
            .propose(
                "Loose",
                PlanId::new_unchecked("p"),
                json!({"actions": "not-an-array"}),
                &tenant,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            PlanProtocolError::ActionsNotAnArray { spec } if spec == "Loose"
        ));
    }

    #[tokio::test]
    async fn unknown_action_kind_decodes_fine() {
        // `HarnessAction` is shape-agnostic over `payload` and accepts
        // any string `kind`. The schema can constrain it via enum
        // values; if it doesn't, the runtime persists whatever the
        // planner emitted (in the canonical flat shape).
        let (_kv, _refs, registry) = build_registries();
        let tenant = TenantId::new_unchecked("acme");
        let plan = registry
            .propose(
                "ScenarioPlan",
                PlanId::new_unchecked("p"),
                json!({"actions": [{"kind": "totally_made_up", "x": 1}]}),
                &tenant,
            )
            .await
            .unwrap();
        assert_eq!(plan.actions.first().kind, "totally_made_up");
    }

    // ─── P1: flat-action normalisation ───────────────────────────

    /// PR review (P1): customer plan schemas use the flat
    /// discriminated-union shape from §13.2 of the Harness
    /// abstractions doc. Plain `HarnessAction::deserialize` silently
    /// drops unknown fields, so the executor would receive an empty
    /// payload. The runtime now folds every non-framework field into
    /// `HarnessAction.payload`.
    #[tokio::test]
    async fn flat_action_fields_are_folded_into_payload() {
        let (_kv, _refs, registry) = build_registries();
        let tenant = TenantId::new_unchecked("acme");
        let plan = registry
            .propose(
                "ScenarioPlan",
                PlanId::new_unchecked("flat-1"),
                json!({
                    "actions": [
                        {"kind": "price_change", "product_id": "p-1", "new_price": 9.99}
                    ]
                }),
                &tenant,
            )
            .await
            .unwrap();
        let action = plan.actions.first();
        assert_eq!(action.kind, "price_change");
        assert_eq!(
            action.payload,
            json!({"product_id": "p-1", "new_price": 9.99})
        );
        assert!(action.references.is_empty());
    }

    /// `references` is the only other framework-reserved key — it
    /// must be lifted into `HarnessAction.references` rather than
    /// staying in payload.
    #[tokio::test]
    async fn flat_action_with_references_keeps_references_separate() {
        let (_kv, _refs, registry) = build_registries();
        let tenant = TenantId::new_unchecked("acme");
        let plan = registry
            .propose(
                "ScenarioPlan",
                PlanId::new_unchecked("flat-2"),
                json!({
                    "actions": [{
                        "kind": "price_change",
                        "product_id": "p-1",
                        "new_price": 9.99,
                        "references": [
                            {"kind": "ProductSet", "id": "ref-abc"}
                        ]
                    }]
                }),
                &tenant,
            )
            .await
            .unwrap();
        let action = plan.actions.first();
        assert_eq!(
            action.payload,
            json!({"product_id": "p-1", "new_price": 9.99}),
        );
        assert_eq!(
            action.references,
            vec![ArtifactRef {
                kind: "ProductSet".into(),
                id: "ref-abc".into(),
            }],
        );
    }

    #[tokio::test]
    async fn non_object_action_is_rejected() {
        // Use a loose spec so schema validation doesn't shadow the
        // normalisation error — we want to hit the helper directly.
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let references = Arc::new(ReferenceRegistry::new(vec![], kv.clone()).unwrap());
        let registry = PlanRegistry::new(vec![loose_plan_spec("Loose")], kv, references).unwrap();
        let tenant = TenantId::new_unchecked("acme");
        let err = registry
            .propose(
                "Loose",
                PlanId::new_unchecked("p"),
                json!({"actions": ["not-an-object"]}),
                &tenant,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            PlanProtocolError::ActionDecode { spec, index: 0, .. } if spec == "Loose"
        ));
    }

    #[tokio::test]
    async fn action_missing_kind_is_rejected() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let references = Arc::new(ReferenceRegistry::new(vec![], kv.clone()).unwrap());
        let registry = PlanRegistry::new(vec![loose_plan_spec("Loose")], kv, references).unwrap();
        let tenant = TenantId::new_unchecked("acme");
        let err = registry
            .propose(
                "Loose",
                PlanId::new_unchecked("p"),
                json!({"actions": [{"product_id": "p-1"}]}),
                &tenant,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            PlanProtocolError::ActionDecode { spec, index: 0, .. } if spec == "Loose"
        ));
    }

    #[tokio::test]
    async fn invalid_references_shape_is_rejected() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let references = Arc::new(ReferenceRegistry::new(vec![], kv.clone()).unwrap());
        let registry = PlanRegistry::new(vec![loose_plan_spec("Loose")], kv, references).unwrap();
        let tenant = TenantId::new_unchecked("acme");
        let err = registry
            .propose(
                "Loose",
                PlanId::new_unchecked("p"),
                json!({"actions": [{"kind": "x", "references": "not-an-array"}]}),
                &tenant,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            PlanProtocolError::ActionDecode { spec, index: 0, .. } if spec == "Loose"
        ));
    }

    // ─── P2: non-`actions` body fields persist into Plan.context ─

    /// PR review (P2): top-level fields outside `actions` (rationale,
    /// scope_ref, review metadata) must survive the round-trip onto
    /// `Plan.context` so the UI plan card and the executor can read
    /// them later.
    #[tokio::test]
    async fn non_actions_top_level_fields_are_persisted_in_context() {
        let (_kv, _refs, registry) = build_registries();
        let tenant = TenantId::new_unchecked("acme");
        let plan_id = PlanId::new_unchecked("scenario-context");
        let body = json!({
            "actions": [
                {"kind": "price_change", "product_id": "p-1", "new_price": 9.99}
            ],
            "rationale": "lift Q3 margins on top SKUs",
            "scope_ref": "scope-abc"
        });
        let proposed = registry
            .propose("ScenarioPlan", plan_id.clone(), body, &tenant)
            .await
            .unwrap();
        assert_eq!(
            proposed.context.get("rationale"),
            Some(&json!("lift Q3 margins on top SKUs"))
        );
        assert_eq!(proposed.context.get("scope_ref"), Some(&json!("scope-abc")));
        // `actions` is the one key stripped — it is not duplicated into context.
        assert!(proposed.context.get("actions").is_none());

        // Round-trip through KV preserves the same context.
        let loaded = registry.load(&plan_id, &tenant).await.unwrap().unwrap();
        assert_eq!(
            loaded.context.get("rationale"),
            Some(&json!("lift Q3 margins on top SKUs"))
        );
        assert_eq!(loaded.context.get("scope_ref"), Some(&json!("scope-abc")));
    }

    #[tokio::test]
    async fn context_is_empty_when_body_only_has_actions() {
        let (_kv, _refs, registry) = build_registries();
        let tenant = TenantId::new_unchecked("acme");
        let plan = registry
            .propose(
                "ScenarioPlan",
                PlanId::new_unchecked("only-actions"),
                json!({"actions": [{"kind": "noop"}]}),
                &tenant,
            )
            .await
            .unwrap();
        assert!(
            plan.context.is_empty(),
            "context must be empty when body carries no non-actions fields"
        );
    }

    // ─── Shape: unknown spec ─────────────────────────────────────

    #[tokio::test]
    async fn unknown_spec_on_propose_errors() {
        let (_kv, _refs, registry) = build_registries();
        let tenant = TenantId::new_unchecked("acme");
        let err = registry
            .propose(
                "MissingSpec",
                PlanId::new_unchecked("p"),
                json!({"actions": []}),
                &tenant,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            PlanProtocolError::UnknownSpec(s) if s == "MissingSpec"
        ));
    }

    // ─── Shape: tenant isolation on load ─────────────────────────

    #[tokio::test]
    async fn tenant_isolation_on_load() {
        let (_kv, _refs, registry) = build_registries();
        let t_a = TenantId::new_unchecked("tenant-a");
        let t_b = TenantId::new_unchecked("tenant-b");
        let plan_id = PlanId::new_unchecked("shared-id");
        let body = json!({"actions": [{"kind": "noop", "payload": {}, "references": []}]});
        registry
            .propose("ScenarioPlan", plan_id.clone(), body, &t_a)
            .await
            .unwrap();
        let loaded = registry.load(&plan_id, &t_a).await.unwrap();
        assert!(loaded.is_some());
        let leaked = registry.load(&plan_id, &t_b).await.unwrap();
        assert!(leaked.is_none(), "tenant B must not see tenant A's plan");
    }

    // ─── Shape: display alias lookup ─────────────────────────────

    #[test]
    fn display_alias_lookup_returns_configured_alias_then_none() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let references = Arc::new(ReferenceRegistry::new(vec![], kv.clone()).unwrap());
        let registry = PlanRegistry::new(vec![scenario_plan_spec()], kv, references).unwrap();
        assert_eq!(
            registry.display_alias("ScenarioPlan", PlanStatus::Draft),
            Some("pending_approval")
        );
        // No alias configured for Approved.
        assert_eq!(
            registry.display_alias("ScenarioPlan", PlanStatus::Approved),
            None
        );
        // Unknown spec → None (no panic).
        assert_eq!(
            registry.display_alias("MissingSpec", PlanStatus::Draft),
            None
        );
    }

    // ─── Hydration error paths ───────────────────────────────────

    #[tokio::test]
    async fn missing_reference_during_hydration_errors() {
        use async_trait::async_trait;

        let (_kv, refs, _registry) = build_registries();
        let tenant = TenantId::new_unchecked("acme");
        // A plan that references an ArtifactRef the registry never created.
        let bogus = ArtifactRef {
            kind: "ProductSet".into(),
            id: "never-created".into(),
        };
        let actions = single_action(HarnessAction {
            kind: "noop".into(),
            payload: json!({}),
            references: vec![bogus.clone()],
        });
        struct PanicInner;
        #[derive(Debug)]
        struct E;
        impl std::fmt::Display for E {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("e")
            }
        }
        impl std::error::Error for E {}
        #[async_trait]
        impl ActionDispatcher for PanicInner {
            type Action = HarnessAction;
            type Context = HarnessActionContext;
            type Error = E;
            async fn dispatch(
                &self,
                _actions: &ActionSeq<HarnessAction>,
                _ctx: &HarnessActionContext,
            ) -> Result<ExecutionResult, E> {
                panic!("inner must not be called when hydration fails");
            }
        }
        let dispatcher = HydratingDispatcher::new(PanicInner, refs.clone(), tenant);
        let err = dispatcher.dispatch(&actions, &()).await.unwrap_err();
        match err {
            HydrationError::MissingReference { kind, id } => {
                assert_eq!(kind, "ProductSet");
                assert_eq!(id, "never-created");
            }
            other => panic!("expected MissingReference, got {other:?}"),
        }
    }

    /// Dedup: two actions reference the same ArtifactRef → the
    /// resolved_refs map has a single entry, and the inner
    /// dispatcher sees the same value under that key.
    #[tokio::test]
    async fn hydration_deduplicates_references() {
        use async_trait::async_trait;
        use std::sync::Mutex;

        let (_kv, refs, _registry) = build_registries();
        let tenant = TenantId::new_unchecked("acme");
        let shared = refs
            .create(
                "ProductSet",
                json!({"product_ids": ["a"]}),
                json!({"n_products": 1}),
                &tenant,
            )
            .await
            .unwrap();
        let actions = action_seq_from_vec(vec![
            HarnessAction {
                kind: "x".into(),
                payload: json!({}),
                references: vec![shared.clone()],
            },
            HarnessAction {
                kind: "y".into(),
                payload: json!({}),
                references: vec![shared.clone(), shared.clone()],
            },
            HarnessAction {
                kind: "z".into(),
                payload: json!({}),
                references: vec![shared.clone()],
            },
        ])
        .unwrap();
        struct Inner(Mutex<usize>);
        #[derive(Debug)]
        struct E;
        impl std::fmt::Display for E {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("e")
            }
        }
        impl std::error::Error for E {}
        #[async_trait]
        impl ActionDispatcher for Inner {
            type Action = HarnessAction;
            type Context = HarnessActionContext;
            type Error = E;
            async fn dispatch(
                &self,
                _actions: &ActionSeq<HarnessAction>,
                ctx: &HarnessActionContext,
            ) -> Result<ExecutionResult, E> {
                *self.0.lock().unwrap() = ctx.resolved_refs.len();
                Ok(ExecutionResult::default())
            }
        }
        let inner = Inner(Mutex::new(0));
        let dispatcher = HydratingDispatcher::new(inner, refs.clone(), tenant);
        dispatcher.dispatch(&actions, &()).await.unwrap();
        let size = *dispatcher.inner.0.lock().unwrap();
        assert_eq!(
            size, 1,
            "shared ref must appear exactly once in resolved_refs"
        );
    }

    /// Sanity that the dispatcher's `Inner` error variant is reachable.
    #[tokio::test]
    async fn inner_dispatcher_error_propagates_after_hydration() {
        use async_trait::async_trait;

        let (_kv, refs, _registry) = build_registries();
        let tenant = TenantId::new_unchecked("acme");
        let actions = single_action(HarnessAction {
            kind: "noop".into(),
            payload: json!({}),
            references: vec![],
        });
        struct Boom;
        #[derive(Debug)]
        struct Boomed;
        impl std::fmt::Display for Boomed {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("boomed")
            }
        }
        impl std::error::Error for Boomed {}
        #[async_trait]
        impl ActionDispatcher for Boom {
            type Action = HarnessAction;
            type Context = HarnessActionContext;
            type Error = Boomed;
            async fn dispatch(
                &self,
                _: &ActionSeq<HarnessAction>,
                _: &HarnessActionContext,
            ) -> Result<ExecutionResult, Boomed> {
                Err(Boomed)
            }
        }
        let err = HydratingDispatcher::new(Boom, refs.clone(), tenant)
            .dispatch(&actions, &())
            .await
            .unwrap_err();
        assert!(matches!(err, HydrationError::Inner(Boomed)));
    }
}
