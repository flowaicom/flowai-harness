//! Startup-time extension validation for tool handlers.
//!
//! [`ToolExtensionManifest`] declares which TypeMap extensions a tool handler
//! requires (or optionally uses). [`ComposedDispatcher::validate_extensions`]
//! checks all manifests against the environment at startup, so missing
//! extensions surface immediately — not during the first user interaction.
//!
//! # Design (fail fast, not fail late)
//!
//! `Has<T>` provides compile-time proof for framework capabilities, but domain
//! extensions live in a dynamic TypeMap. We cannot close the type universe at
//! compile time (the set of extensions is open), but we CAN fail at startup
//! rather than during user interaction. That's what this module provides.

use std::any::TypeId;

/// A single extension requirement.
#[derive(Debug, Clone)]
pub struct ExtensionRequirement {
    /// Human-readable description (e.g., "TargetDatabase for SQL queries").
    pub description: &'static str,
    /// The `TypeId` of `Arc<T>` used by `ToolEnvironment::ext::<T>()`.
    pub type_id: TypeId,
    /// The full type name for diagnostics.
    pub type_name: &'static str,
}

/// A missing extension discovered during validation.
#[derive(Debug, Clone)]
pub struct MissingExtension {
    /// Which tool declared the requirement.
    pub tool_name: String,
    /// The requirement that was not satisfied.
    pub requirement: ExtensionRequirement,
}

impl std::fmt::Display for MissingExtension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Tool '{}' requires extension {} ({})",
            self.tool_name, self.requirement.type_name, self.requirement.description
        )
    }
}

/// Where the duplicate-name collision was detected.
///
/// `ComposedDispatcher` keeps two separate name maps: active handlers (registered
/// for default dispatch) and latent handlers (executable but hidden until a
/// request-scoped activation exposes them). A name may collide within one map or
/// across the two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollisionKind {
    /// Two active handlers share the same name.
    Active,
    /// Two latent handlers share the same name.
    Latent,
    /// An active handler and a latent handler share the same name.
    ActiveVsLatent,
}

impl std::fmt::Display for CollisionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            CollisionKind::Active => "active",
            CollisionKind::Latent => "latent",
            CollisionKind::ActiveVsLatent => "active vs. latent",
        };
        f.write_str(label)
    }
}

/// A tool-name collision discovered during dispatcher composition.
///
/// Surfaced by `ComposedDispatcher::try_merge` and by `try_build()` so that
/// duplicate registrations fail at startup with a clear list of culprits instead
/// of silently overriding one handler with another.
#[derive(Debug, Clone)]
pub struct ToolCollision {
    /// The tool name that was registered more than once.
    pub tool_name: String,
    /// Whether the duplicate is active-vs-active, latent-vs-latent, or mixed.
    pub kind: CollisionKind,
}

impl std::fmt::Display for ToolCollision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Tool '{}' registered more than once ({})",
            self.tool_name, self.kind
        )
    }
}

/// Declares which TypeMap extensions a tool handler needs.
///
/// Build via the chainable `requires` / `optional` methods:
///
/// ```ignore
/// fn required_extensions(&self) -> ToolExtensionManifest {
///     ToolExtensionManifest::new()
///         .requires::<dyn TargetDatabase>("SQL query execution")
/// }
/// ```
#[derive(Debug, Clone, Default)]
pub struct ToolExtensionManifest {
    required: Vec<ExtensionRequirement>,
    optional: Vec<ExtensionRequirement>,
}

impl ToolExtensionManifest {
    /// Create an empty manifest (no requirements).
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a required extension.
    ///
    /// `T` must match the type used in `env.ext::<T>()` / `env.try_ext::<T>()`.
    pub fn requires<T: Send + Sync + 'static + ?Sized>(
        mut self,
        description: &'static str,
    ) -> Self {
        self.required.push(ExtensionRequirement {
            description,
            type_id: TypeId::of::<std::sync::Arc<T>>(),
            type_name: std::any::type_name::<T>(),
        });
        self
    }

    /// Declare an optional extension.
    ///
    /// Optional extensions are logged when missing but don't cause validation failure.
    pub fn optional<T: Send + Sync + 'static + ?Sized>(
        mut self,
        description: &'static str,
    ) -> Self {
        self.optional.push(ExtensionRequirement {
            description,
            type_id: TypeId::of::<std::sync::Arc<T>>(),
            type_name: std::any::type_name::<T>(),
        });
        self
    }

    /// Check all required extensions against the environment.
    ///
    /// Returns `Ok(())` if all required extensions are present,
    /// or `Err(missing)` with the list of missing extensions.
    pub fn validate(
        &self,
        tool_name: &str,
        env: &crate::ToolEnvironment,
    ) -> Result<(), Vec<MissingExtension>> {
        let missing: Vec<MissingExtension> = self
            .required
            .iter()
            .filter(|req| !env.has_ext_by_type_id(req.type_id))
            .map(|req| MissingExtension {
                tool_name: tool_name.to_string(),
                requirement: req.clone(),
            })
            .collect();

        if missing.is_empty() {
            Ok(())
        } else {
            Err(missing)
        }
    }

    /// Combine two manifests. Union of required and optional sets.
    ///
    /// This is a monoid: `a.merge(b).merge(c)` is associative,
    /// and `ToolExtensionManifest::new()` is the identity.
    ///
    /// # Law
    ///
    /// - **Monotonicity**: No requirement is lost through merging.
    ///   `a.merge(b).required_extensions().len() >= a.required_extensions().len()`
    pub fn merge(mut self, other: Self) -> Self {
        self.required.extend(other.required);
        self.optional.extend(other.optional);
        self
    }

    /// Whether this manifest has any requirements at all.
    pub fn is_empty(&self) -> bool {
        self.required.is_empty() && self.optional.is_empty()
    }

    /// The required extension list.
    pub fn required_extensions(&self) -> &[ExtensionRequirement] {
        &self.required
    }

    /// The optional extension list.
    pub fn optional_extensions(&self) -> &[ExtensionRequirement] {
        &self.optional
    }
}

// =============================================================================
// Manifest convenience helpers
// =============================================================================

/// Build a single-extension manifest from a type parameter.
///
/// Bridges the `extension_manifest()` declaration on a tool handler
/// with runtime validation ([`ToolExtensionManifest`]).
///
/// ```ignore
/// fn extension_manifest(&self) -> ToolExtensionManifest {
///     manifest_for::<dyn TargetDatabase>("SQL query execution")
///         .requires::<dyn DataCatalog>("Catalog discovery")
/// }
/// ```
pub fn manifest_for<T: Send + Sync + 'static + ?Sized>(
    description: &'static str,
) -> ToolExtensionManifest {
    ToolExtensionManifest::new().requires::<T>(description)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_manifest_always_validates() {
        let manifest = ToolExtensionManifest::new();
        assert!(manifest.is_empty());

        let env = crate::ToolEnvironment::builder()
            .kv(agent_fw_algebra::testing::NullKVStore)
            .tenant("test")
            .build();
        assert!(manifest.validate("test-tool", &env).is_ok());
    }

    trait TestCapability: Send + Sync {}

    #[test]
    fn required_extension_fails_when_missing() {
        let manifest =
            ToolExtensionManifest::new().requires::<dyn TestCapability>("Test capability");

        let env = crate::ToolEnvironment::builder()
            .kv(agent_fw_algebra::testing::NullKVStore)
            .tenant("test")
            .build();

        let result = manifest.validate("myTool", &env);
        assert!(result.is_err());
        let missing = result.unwrap_err();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].tool_name, "myTool");
        assert!(missing[0].requirement.type_name.contains("TestCapability"));
    }

    #[test]
    fn required_extension_passes_when_present() {
        struct TestImpl;
        impl TestCapability for TestImpl {}

        let manifest =
            ToolExtensionManifest::new().requires::<dyn TestCapability>("Test capability");

        let env = crate::ToolEnvironment::builder()
            .kv(agent_fw_algebra::testing::NullKVStore)
            .tenant("test")
            .build()
            .with_ext::<dyn TestCapability>(std::sync::Arc::new(TestImpl));

        assert!(manifest.validate("myTool", &env).is_ok());
    }

    #[test]
    fn optional_extension_does_not_fail() {
        let manifest = ToolExtensionManifest::new().optional::<dyn TestCapability>("Nice to have");

        let env = crate::ToolEnvironment::builder()
            .kv(agent_fw_algebra::testing::NullKVStore)
            .tenant("test")
            .build();

        // Optional extension missing — still OK
        assert!(manifest.validate("myTool", &env).is_ok());
    }

    trait AnotherCapability: Send + Sync {}

    #[test]
    fn merge_combines_requirements() {
        let a = ToolExtensionManifest::new().requires::<dyn TestCapability>("Cap A");
        let b = ToolExtensionManifest::new().requires::<dyn AnotherCapability>("Cap B");

        let merged = a.merge(b);
        assert_eq!(merged.required_extensions().len(), 2);
    }

    #[test]
    fn merge_identity() {
        let a = ToolExtensionManifest::new().requires::<dyn TestCapability>("Cap A");
        let empty = ToolExtensionManifest::new();

        // a.merge(empty) preserves requirements
        let merged = a.merge(empty);
        assert_eq!(merged.required_extensions().len(), 1);
    }

    #[test]
    fn merge_with_optional() {
        let a = ToolExtensionManifest::new().requires::<dyn TestCapability>("Required");
        let b = ToolExtensionManifest::new().optional::<dyn AnotherCapability>("Optional");

        let merged = a.merge(b);
        assert_eq!(merged.required_extensions().len(), 1);
        assert_eq!(merged.optional_extensions().len(), 1);
    }

    #[test]
    fn manifest_for_helper() {
        let m = super::manifest_for::<dyn TestCapability>("Test");
        assert_eq!(m.required_extensions().len(), 1);
        assert!(m.required_extensions()[0]
            .type_name
            .contains("TestCapability"));
    }

    #[test]
    fn display_tool_collision() {
        let collision = ToolCollision {
            tool_name: "search_catalog".to_string(),
            kind: CollisionKind::Active,
        };
        let msg = collision.to_string();
        assert!(msg.contains("search_catalog"));
        assert!(msg.contains("active"));
    }

    #[test]
    fn display_active_vs_latent_collision() {
        let collision = ToolCollision {
            tool_name: "execute_query".to_string(),
            kind: CollisionKind::ActiveVsLatent,
        };
        let msg = collision.to_string();
        assert!(msg.contains("execute_query"));
        assert!(msg.contains("active vs. latent"));
    }

    #[test]
    fn display_missing_extension() {
        let missing = MissingExtension {
            tool_name: "draft_plan".to_string(),
            requirement: ExtensionRequirement {
                description: "SQL query execution",
                type_id: TypeId::of::<std::sync::Arc<dyn TestCapability>>(),
                type_name: "dyn TestCapability",
            },
        };
        let msg = missing.to_string();
        assert!(msg.contains("draft_plan"));
        assert!(msg.contains("TestCapability"));
        assert!(msg.contains("SQL query execution"));
    }
}
