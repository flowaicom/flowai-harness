//! Tenant context for multi-tenancy isolation
//!
//! TenantContext carries tenant identity extracted from auth headers.
//! It is a VALUE - carries identity but has minimal behavior.
//!
//! # Design: Identity Without Behavior
//!
//! Following the "data is just data" principle, TenantContext is a simple
//! struct with accessor methods. Complex operations (like key scoping) are
//! free functions when needed.
//!
//! # Security Invariants
//!
//! - `resource_id` is NEVER derived from user input
//! - Always extracted from verified auth headers (OIDC claims)
//! - In dev mode, synthetic users are created with "dev-" prefix

use super::id::{TenantId, ThreadId};
use serde::{Deserialize, Serialize};

/// User reference for observability.
///
/// Provides two levels of user identification:
/// - `current`: Rotated periodically, safe to log
/// - `stable`: Stable across sessions, for correlation
///
/// Both are HMAC derivatives of the actual user ID, not the raw value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserRef {
    /// Current reference (may rotate)
    pub current: String,
    /// Stable reference (long-lived)
    pub stable: String,
}

impl UserRef {
    /// Create a new UserRef.
    pub fn new(current: impl Into<String>, stable: impl Into<String>) -> Self {
        Self {
            current: current.into(),
            stable: stable.into(),
        }
    }
}

/// Immutable tenant context extracted from auth headers.
///
/// # Invariants
/// - `resource_id` is never empty
/// - `resource_id` is deterministic given auth headers
/// - `resource_id` is NEVER from client/user input
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TenantContext {
    resource_id: TenantId,
    thread_id: Option<ThreadId>,
    user_ref: Option<UserRef>,
}

impl TenantContext {
    /// Create a new TenantContext with only resource ID.
    pub fn new(resource_id: TenantId) -> Self {
        Self {
            resource_id,
            thread_id: None,
            user_ref: None,
        }
    }

    /// Add a thread ID.
    pub fn with_thread(mut self, thread_id: ThreadId) -> Self {
        self.thread_id = Some(thread_id);
        self
    }

    /// Add a user reference for observability.
    pub fn with_user_ref(mut self, user_ref: UserRef) -> Self {
        self.user_ref = Some(user_ref);
        self
    }

    /// Get the resource ID.
    pub fn resource_id(&self) -> &TenantId {
        &self.resource_id
    }

    /// Get the thread ID if set.
    pub fn thread_id(&self) -> Option<&ThreadId> {
        self.thread_id.as_ref()
    }

    /// Get the user reference if set.
    pub fn user_ref(&self) -> Option<&UserRef> {
        self.user_ref.as_ref()
    }

    /// Derive a sub-agent thread ID: `{thread_id}-{agent_name}`
    pub fn derive_sub_thread(&self, agent_name: &str) -> Option<ThreadId> {
        self.thread_id
            .as_ref()
            .map(|t| t.derive_sub_thread(agent_name))
    }

    /// Derive a sub-agent tenant context with thread isolation.
    ///
    /// If the parent has a thread_id, the child gets `{thread_id}-{agent_name}`.
    /// If the parent has no thread_id, the child gets no thread_id either.
    /// User reference is preserved from the parent.
    pub fn with_derived_thread(&self, agent_name: &str) -> Self {
        Self {
            resource_id: self.resource_id.clone(),
            thread_id: self.derive_sub_thread(agent_name),
            user_ref: self.user_ref.clone(),
        }
    }
}

// =============================================================================
// Axum Extractor (behind `axum` feature)
// =============================================================================

/// Error returned when TenantContext is missing from request extensions.
#[derive(Debug, Clone)]
pub struct MissingTenantContext;

impl std::fmt::Display for MissingTenantContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Missing TenantContext in request extensions. Ensure auth middleware is applied."
        )
    }
}

impl std::error::Error for MissingTenantContext {}

#[cfg(feature = "axum")]
mod axum_impl {
    use super::*;

    impl axum_core::response::IntoResponse for MissingTenantContext {
        fn into_response(self) -> axum_core::response::Response {
            (http::StatusCode::UNAUTHORIZED, self.to_string()).into_response()
        }
    }

    impl<S: Send + Sync> axum_core::extract::FromRequestParts<S> for TenantContext {
        type Rejection = MissingTenantContext;

        async fn from_request_parts(
            parts: &mut http::request::Parts,
            _state: &S,
        ) -> Result<Self, Self::Rejection> {
            parts
                .extensions
                .get::<TenantContext>()
                .cloned()
                .ok_or(MissingTenantContext)
        }
    }
}

//=============================================================================
// FREE FUNCTIONS for common operations
//=============================================================================

/// Create a scoped key for KV storage.
///
/// Format: `{resource_id}:{key}`
pub fn scoped_key(tenant: &TenantId, key: &str) -> String {
    format!("{}:{}", tenant, key)
}

/// Create a scoped key from TenantContext.
pub fn scoped_key_from_context(ctx: &TenantContext, key: &str) -> String {
    scoped_key(ctx.resource_id(), key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_tenant() -> TenantId {
        TenantId::new_unchecked("test-tenant-123")
    }

    fn test_thread() -> ThreadId {
        ThreadId::new_unchecked("thread-456")
    }

    #[test]
    fn new_context_has_resource_id() {
        let ctx = TenantContext::new(test_tenant());
        assert_eq!(ctx.resource_id().as_str(), "test-tenant-123");
        assert!(ctx.thread_id().is_none());
        assert!(ctx.user_ref().is_none());
    }

    #[test]
    fn with_thread_adds_thread() {
        let ctx = TenantContext::new(test_tenant()).with_thread(test_thread());
        assert!(ctx.thread_id().is_some());
        assert_eq!(ctx.thread_id().unwrap().as_str(), "thread-456");
    }

    #[test]
    fn with_user_ref_adds_ref() {
        let user_ref = UserRef::new("current-ref", "stable-ref");
        let ctx = TenantContext::new(test_tenant()).with_user_ref(user_ref.clone());
        assert_eq!(ctx.user_ref(), Some(&user_ref));
    }

    #[test]
    fn derive_sub_thread_with_thread() {
        let ctx = TenantContext::new(test_tenant()).with_thread(test_thread());
        let sub = ctx.derive_sub_thread("planner");
        assert_eq!(sub.unwrap().as_str(), "thread-456-planner");
    }

    #[test]
    fn derive_sub_thread_without_thread() {
        let ctx = TenantContext::new(test_tenant());
        let sub = ctx.derive_sub_thread("planner");
        assert!(sub.is_none());
    }

    #[test]
    fn with_derived_thread_creates_child_context() {
        let ctx = TenantContext::new(test_tenant()).with_thread(test_thread());
        let child = ctx.with_derived_thread("planner");
        assert_eq!(child.resource_id().as_str(), "test-tenant-123");
        assert_eq!(child.thread_id().unwrap().as_str(), "thread-456-planner");
    }

    #[test]
    fn with_derived_thread_without_thread() {
        let ctx = TenantContext::new(test_tenant());
        let child = ctx.with_derived_thread("planner");
        assert_eq!(child.resource_id().as_str(), "test-tenant-123");
        assert!(child.thread_id().is_none());
    }

    #[test]
    fn with_derived_thread_preserves_user_ref() {
        let user_ref = UserRef::new("current-ref", "stable-ref");
        let ctx = TenantContext::new(test_tenant())
            .with_thread(test_thread())
            .with_user_ref(user_ref.clone());
        let child = ctx.with_derived_thread("executor");
        assert_eq!(child.user_ref(), Some(&user_ref));
    }

    #[test]
    fn scoped_key_formats_correctly() {
        let key = scoped_key(&test_tenant(), "plan:abc123");
        assert_eq!(key, "test-tenant-123:plan:abc123");
    }

    #[test]
    fn scoped_key_from_context_uses_resource_id() {
        let ctx = TenantContext::new(test_tenant());
        let key = scoped_key_from_context(&ctx, "plan:abc123");
        assert_eq!(key, "test-tenant-123:plan:abc123");
    }
}
