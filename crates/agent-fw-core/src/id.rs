//! Typed identifiers for framework entities
//!
//! Using newtype wrappers prevents accidentally mixing different ID types.
//! All IDs are validated at construction to ensure non-empty values.
//!
//! # Framework IDs
//!
//! - `TenantId` — Tenant/resource identifier from auth context
//! - `ThreadId` — Conversation thread identifier
//! - `UserId` — User identifier for tracking
//!
//! Domain-specific IDs (PlanId, ScenarioId, etc.) should be defined
//! in the consuming application, not here.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

/// Tenant/resource identifier extracted from auth headers.
///
/// # Invariants
/// - Never empty (validated at construction)
/// - Never derived from user input (always from auth context)
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TenantId(String);

impl TenantId {
    /// Create a new TenantId from a string.
    ///
    /// Returns `None` if the string is empty or whitespace-only.
    pub fn new(s: impl Into<String>) -> Option<Self> {
        let s = s.into();
        if s.trim().is_empty() {
            None
        } else {
            Some(Self(s))
        }
    }

    /// Create a TenantId without validation.
    /// Use sparingly - prefer `new()` with validation.
    pub fn new_unchecked(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Thread identifier for agent memory/conversation context.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ThreadId(String);

impl ThreadId {
    pub fn new(s: impl Into<String>) -> Option<Self> {
        let s = s.into();
        if s.is_empty() {
            None
        } else {
            Some(Self(s))
        }
    }

    pub fn new_unchecked(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Derive a sub-agent thread ID: `{self}-{agent_name}`
    pub fn derive_sub_thread(&self, agent_name: &str) -> Self {
        Self(format!("{}-{}", self.0, agent_name))
    }
}

impl fmt::Display for ThreadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// User identifier for approval tracking.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserId(String);

impl UserId {
    pub fn new(s: impl Into<String>) -> Option<Self> {
        let s = s.into();
        if s.is_empty() {
            None
        } else {
            Some(Self(s))
        }
    }

    /// Create a UserId without validation.
    /// Use sparingly - prefer `new()` with validation.
    pub fn new_unchecked(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a plan within the framework.
///
/// Plans are generic over action type `A`; the PlanId identifies
/// the plan instance regardless of the action payload.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlanId(String);

impl PlanId {
    pub fn new(s: impl Into<String>) -> Option<Self> {
        let s = s.into();
        if s.is_empty() {
            None
        } else {
            Some(Self(s))
        }
    }

    pub fn new_unchecked(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PlanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Identifier for a resolved entity set stored in KV.
///
/// An entity set is the result of resolving a user query into concrete
/// entities (products, SKUs, campaigns, etc.). The set itself is stored
/// in the KV store; this ID is the handle.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntitySetId(String);

impl EntitySetId {
    pub fn new(s: impl Into<String>) -> Option<Self> {
        let s = s.into();
        if s.is_empty() {
            None
        } else {
            Some(Self(s))
        }
    }

    pub fn new_unchecked(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EntitySetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a pending approval request.
///
/// Approvals are per-request: each tool call or plan execution that requires
/// human review allocates its own `ApprovalId`. The id is used as a KV key
/// (scoped by `TenantId`) and as the lookup key into the in-process awaiter
/// map maintained by `PendingApprovalStore` interpreters.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ApprovalId(String);

impl ApprovalId {
    pub fn new(s: impl Into<String>) -> Option<Self> {
        let s = s.into();
        if s.is_empty() {
            None
        } else {
            Some(Self(s))
        }
    }

    pub fn new_unchecked(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ApprovalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for an evaluation run.
///
/// An eval run executes a set of test cases against an agent configuration,
/// producing scored results for each test case.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EvalRunId(String);

impl EvalRunId {
    pub fn new(s: impl Into<String>) -> Option<Self> {
        let s = s.into();
        if s.is_empty() {
            None
        } else {
            Some(Self(s))
        }
    }

    pub fn new_unchecked(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EvalRunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for an authored test case.
///
/// Test cases define expected agent behavior: an input prompt,
/// an expected tool trajectory, and optional structured ground truth.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TestCaseId(String);

impl TestCaseId {
    pub fn new(s: impl Into<String>) -> Option<Self> {
        let s = s.into();
        if s.is_empty() {
            None
        } else {
            Some(Self(s))
        }
    }

    pub fn new_unchecked(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TestCaseId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Workspace identifier — the top-level multi-tenancy entity.
///
/// # Invariants
///
/// - Inner string is always non-blank (trimmed length > 0).
/// - Enforced at construction (`new()`) and deserialization (custom impl).
///
/// # Laws
///
/// - **Totality**: `new(s)` never panics — returns `None` for invalid input.
/// - **Roundtrip**: `deserialize(serialize(id)) == id`.
/// - **Display-Parse consistency**: `WorkspaceId::new(id.to_string()) == Some(id)`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct WorkspaceId(String);

impl<'de> Deserialize<'de> for WorkspaceId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        WorkspaceId::new(s)
            .ok_or_else(|| serde::de::Error::custom("workspace ID must not be blank"))
    }
}

impl WorkspaceId {
    /// Create a new workspace ID. Returns `None` if the string is blank.
    pub fn new(id: impl Into<String>) -> Option<Self> {
        let s = id.into();
        if s.trim().is_empty() {
            None
        } else {
            Some(Self(s))
        }
    }

    /// Create a WorkspaceId without validation.
    pub fn new_unchecked(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// The implicit default workspace (env-configured databases).
    pub fn default_workspace() -> Self {
        Self("default".to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_default(&self) -> bool {
        self.0 == "default"
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<WorkspaceId> for String {
    fn from(id: WorkspaceId) -> Self {
        id.0
    }
}

impl AsRef<str> for WorkspaceId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Deterministic hash of a FilterSet.
///
/// Used to generate content-addressed IDs for stored filter configurations.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FilterHash(String);

impl FilterHash {
    /// Create from raw hash bytes (hex-encoded).
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(hex::encode(bytes))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FilterHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Compute a deterministic content hash (SHA-256, truncated to 24 hex chars).
///
/// This is a free function (not a method) following the "data is just data" principle.
pub fn content_hash<T: AsRef<[u8]>>(content: T) -> String {
    let digest = Sha256::digest(content.as_ref());
    hex::encode(&digest[..12])
}

/// Compute a tenant-scoped deterministic hash.
pub fn tenant_scoped_hash<T: serde::Serialize>(tenant: &TenantId, content: &T) -> String {
    let payload = serde_json::json!({
        "owner": tenant.as_str(),
        "content": content
    });
    content_hash(payload.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_id_rejects_empty() {
        assert!(TenantId::new("").is_none());
        assert!(TenantId::new("   ").is_none());
        assert!(TenantId::new("valid-tenant").is_some());
    }

    #[test]
    fn thread_id_derivation() {
        let parent = ThreadId::new_unchecked("parent-123");
        let child = parent.derive_sub_thread("planner");
        assert_eq!(child.as_str(), "parent-123-planner");
    }

    #[test]
    fn plan_id_rejects_empty() {
        assert!(PlanId::new("").is_none());
        assert!(PlanId::new("plan-123").is_some());
    }

    #[test]
    fn entity_set_id_rejects_empty() {
        assert!(EntitySetId::new("").is_none());
        assert!(EntitySetId::new("eset-1").is_some());
    }

    #[test]
    fn eval_run_id_rejects_empty() {
        assert!(EvalRunId::new("").is_none());
        assert!(EvalRunId::new("run-123").is_some());
    }

    #[test]
    fn test_case_id_rejects_empty() {
        assert!(TestCaseId::new("").is_none());
        assert!(TestCaseId::new("tc-1").is_some());
    }

    #[test]
    fn workspace_id_rejects_empty() {
        assert!(WorkspaceId::new("").is_none());
        assert!(WorkspaceId::new("   ").is_none());
    }

    #[test]
    fn workspace_id_accepts_valid() {
        let id = WorkspaceId::new("ws-123").unwrap();
        assert_eq!(id.as_str(), "ws-123");
        assert!(!id.is_default());
    }

    #[test]
    fn workspace_id_default() {
        let id = WorkspaceId::default_workspace();
        assert_eq!(id.as_str(), "default");
        assert!(id.is_default());
    }

    #[test]
    fn workspace_id_serde_roundtrip() {
        let id = WorkspaceId::new("ws-abc").unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"ws-abc\"");
        let parsed: WorkspaceId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn workspace_id_deserialize_rejects_blank() {
        assert!(serde_json::from_str::<WorkspaceId>("\"\"").is_err());
        assert!(serde_json::from_str::<WorkspaceId>("\"   \"").is_err());
    }

    #[test]
    fn content_hash_is_deterministic() {
        let hash1 = content_hash("hello");
        let hash2 = content_hash("hello");
        assert_eq!(hash1, hash2);

        let hash3 = content_hash("world");
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn tenant_scoped_hash_includes_tenant() {
        let tenant1 = TenantId::new_unchecked("tenant1");
        let tenant2 = TenantId::new_unchecked("tenant2");

        let hash1 = tenant_scoped_hash(&tenant1, &"same content");
        let hash2 = tenant_scoped_hash(&tenant2, &"same content");

        assert_ne!(hash1, hash2);
    }

    //=========================================================================
    // Property-Based Tests (Hegel)
    //=========================================================================

    use hegel::generators;

    /// Draw a non-empty, non-whitespace-only string (valid for all ID constructors).
    fn draw_valid_id(tc: &hegel::TestCase) -> String {
        let s: String = tc.draw(generators::text().min_size(1));
        // If the string is whitespace-only, prepend a visible char to ensure
        // the ID passes validation. We don't use assume() because rejection
        // rates would be high with full-Unicode text generators.
        if s.trim().is_empty() {
            format!("x{s}")
        } else {
            s
        }
    }

    // --- Serde roundtrip for every ID type ---
    // Full Unicode, not ASCII-only: tests that serde handles multi-byte,
    // combining characters, RTL marks, etc.

    macro_rules! id_roundtrip_test {
        ($name:ident, $ty:ty) => {
            #[hegel::test]
            fn $name(tc: hegel::TestCase) {
                let s = draw_valid_id(&tc);
                let id = <$ty>::new(&s).unwrap();
                let json = serde_json::to_string(&id).unwrap();
                let parsed: $ty = serde_json::from_str(&json).unwrap();
                assert_eq!(id, parsed);
            }
        };
    }

    id_roundtrip_test!(tenant_id_serde_roundtrip, TenantId);
    id_roundtrip_test!(thread_id_serde_roundtrip, ThreadId);
    id_roundtrip_test!(user_id_serde_roundtrip, UserId);
    id_roundtrip_test!(plan_id_serde_roundtrip, PlanId);
    id_roundtrip_test!(entity_set_id_serde_roundtrip, EntitySetId);
    id_roundtrip_test!(eval_run_id_serde_roundtrip, EvalRunId);
    id_roundtrip_test!(test_case_id_serde_roundtrip, TestCaseId);
    id_roundtrip_test!(workspace_id_serde_roundtrip_prop, WorkspaceId);

    // --- Display-parse consistency ---

    #[hegel::test]
    fn tenant_id_display_parse(tc: hegel::TestCase) {
        let s = draw_valid_id(&tc);
        let id = TenantId::new(&s).unwrap();
        let reparsed = TenantId::new(id.to_string()).unwrap();
        assert_eq!(id, reparsed);
    }

    #[hegel::test]
    fn workspace_id_display_parse(tc: hegel::TestCase) {
        let s = draw_valid_id(&tc);
        let id = WorkspaceId::new(&s).unwrap();
        let reparsed = WorkspaceId::new(id.to_string()).unwrap();
        assert_eq!(id, reparsed);
    }

    // --- ThreadId sub-thread derivation ---

    #[hegel::test]
    fn thread_id_sub_thread_prefix(tc: hegel::TestCase) {
        let parent_s = draw_valid_id(&tc);
        let agent: String = tc.draw(generators::text().min_size(1).max_size(20));
        let parent_id = ThreadId::new(&parent_s).unwrap();
        let child = parent_id.derive_sub_thread(&agent);
        assert!(
            child.as_str().starts_with(parent_id.as_str()),
            "child {:?} should start with parent {:?}",
            child.as_str(),
            parent_id.as_str()
        );
    }

    // --- content_hash determinism ---

    #[hegel::test]
    fn content_hash_deterministic(tc: hegel::TestCase) {
        let data: String = tc.draw(generators::text());
        assert_eq!(content_hash(&data), content_hash(&data));
    }

    #[hegel::test]
    fn content_hash_deterministic_binary(tc: hegel::TestCase) {
        let data: Vec<u8> = tc.draw(generators::binary());
        assert_eq!(content_hash(&data), content_hash(&data));
    }

    // --- content_hash length is fixed ---

    #[hegel::test]
    fn content_hash_fixed_length(tc: hegel::TestCase) {
        let data: Vec<u8> = tc.draw(generators::binary());
        let hash = content_hash(&data);
        assert_eq!(
            hash.len(),
            24,
            "SHA-256 truncated to 12 bytes = 24 hex chars"
        );
    }

    // --- Validation totality: new() never panics ---

    #[hegel::test]
    fn tenant_id_new_never_panics(tc: hegel::TestCase) {
        let s: String = tc.draw(generators::text());
        let _ = TenantId::new(&s); // must not panic
    }

    #[hegel::test]
    fn workspace_id_new_never_panics(tc: hegel::TestCase) {
        let s: String = tc.draw(generators::text());
        let _ = WorkspaceId::new(&s); // must not panic
    }

    // --- Validation semantics ---
    // TenantId rejects whitespace-only; ThreadId rejects only empty string.
    // These are distinct behaviors — verify with full Unicode whitespace.

    #[hegel::test]
    fn tenant_id_rejects_all_whitespace(tc: hegel::TestCase) {
        let s: String = tc.draw(generators::text());
        match TenantId::new(&s) {
            None => assert!(s.trim().is_empty()),
            Some(id) => assert!(!id.as_str().trim().is_empty()),
        }
    }

    #[hegel::test]
    fn thread_id_rejects_only_empty(tc: hegel::TestCase) {
        let s: String = tc.draw(generators::text());
        match ThreadId::new(&s) {
            None => assert!(s.is_empty()),
            Some(id) => assert!(!id.as_str().is_empty()),
        }
    }

    // --- WorkspaceId deserialization rejects blank ---

    #[hegel::test]
    fn workspace_id_deserialize_blank_rejected(tc: hegel::TestCase) {
        let s: String = tc.draw(generators::text());
        let json = serde_json::to_string(&s).unwrap();
        let result = serde_json::from_str::<WorkspaceId>(&json);
        if s.trim().is_empty() {
            assert!(result.is_err(), "blank string {:?} should be rejected", s);
        } else {
            assert!(
                result.is_ok(),
                "non-blank string {:?} should be accepted",
                s
            );
        }
    }
}
