//! Retry profiles and transient error detection for interpreter-level retries.
//!
//! Provides pre-configured retry policies and a `TransientError` trait for
//! determining whether errors are worth retrying.
//!
//! Public API for interpreter implementations and downstream consumers.

use std::fmt::Display;
use std::time::Duration;

use agent_fw_algebra::retry::{retry_when_observed, RetryContext, RetryPolicy};
use tracing::Instrument;

/// Pre-configured retry profiles for common scenarios.
#[derive(Debug, Clone, Copy)]
pub enum RetryProfile {
    /// 50ms initial, 3 retries — for fast local operations.
    Fast,
    /// 100ms initial, 3 retries — for typical network operations.
    Normal,
    /// 200ms initial, 5 retries — for cold-start scenarios (e.g., Lambda, container spin-up).
    ColdStart,
}

impl RetryProfile {
    /// Convert to a concrete `RetryPolicy`.
    pub fn policy(self) -> RetryPolicy {
        match self {
            Self::Fast => RetryPolicy::exponential_backoff(3, Duration::from_millis(50)),
            Self::Normal => RetryPolicy::exponential_backoff(3, Duration::from_millis(100)),
            Self::ColdStart => RetryPolicy::exponential_backoff(5, Duration::from_millis(200)),
        }
    }
}

/// Trait for detecting transient (retryable) errors.
///
/// Implementors provide `error_message()` which is checked against known
/// transient error keywords. Override `is_transient()` for custom logic.
pub trait TransientError {
    /// Extract an error message for keyword matching.
    fn error_message(&self) -> Option<&str>;

    /// Whether this error is transient and worth retrying.
    ///
    /// Default implementation checks `error_message()` against known transient keywords.
    fn is_transient(&self) -> bool {
        let Some(msg) = self.error_message() else {
            return false;
        };
        let lower = msg.to_lowercase();
        TRANSIENT_KEYWORDS
            .iter()
            .any(|keyword| lower.contains(keyword))
    }
}

/// Keywords that indicate a transient error.
const TRANSIENT_KEYWORDS: &[&str] = &[
    "connection reset",
    "connection refused",
    "broken pipe",
    "timeout",
    "timed out",
    "temporarily unavailable",
    "too many connections",
    "deadlock",
    "lock timeout",
    "resource busy",
    "try again",
    "service unavailable",
    "503",
    "429",
];

impl TransientError for agent_fw_algebra::target_db::DbError {
    fn error_message(&self) -> Option<&str> {
        match self {
            Self::Connection(msg) => Some(msg),
            Self::Execution(msg) => Some(msg),
            Self::Timeout(_) => Some("timeout"),
            Self::InvalidQuery(_) => None,    // not transient
            Self::Deserialization(_) => None, // not transient
        }
    }
}

impl TransientError for agent_fw_algebra::kv_store::KVError {
    fn error_message(&self) -> Option<&str> {
        match self {
            Self::Storage(msg) => Some(msg),
            Self::Serialization(_) => None,   // not transient
            Self::Deserialization(_) => None, // not transient
        }
    }
}

impl TransientError for agent_fw_catalog::CatalogError {
    fn error_message(&self) -> Option<&str> {
        match self {
            Self::Unavailable(msg) => Some(msg),
            Self::NotFound(_) => None,     // not transient
            Self::InvalidQuery(_) => None, // not transient
        }
    }
}

impl TransientError for agent_fw_algebra::vector_store::VectorStoreError {
    fn error_message(&self) -> Option<&str> {
        match self {
            Self::Connection(msg) => Some(msg),
            Self::Execution(msg) => Some(msg),
            Self::NotConfigured => None,            // not transient
            Self::DimensionMismatch { .. } => None, // not transient
        }
    }
}

impl TransientError for agent_fw_algebra::writable_db::WriteDbError {
    fn error_message(&self) -> Option<&str> {
        match self {
            Self::Connection(msg) => Some(msg),
            Self::Ddl(msg) => Some(msg),
            Self::Dml(msg) => Some(msg),
            Self::Transaction(msg) => Some(msg),
            Self::Timeout(_) => Some("timeout"),
            Self::InvalidSql(_) => None, // not transient
        }
    }
}

/// Create a tracing-based retry observer for structured logging.
pub fn retry_observer<E: Display>(service: &str) -> impl Fn(&RetryContext<&E>) + '_ {
    move |ctx: &RetryContext<&E>| {
        tracing::warn!(
            service = service,
            attempt = ctx.attempt,
            max_retries = ctx.max_retries,
            delay_ms = ctx.delay.as_millis() as u64,
            elapsed_ms = ctx.elapsed.as_millis() as u64,
            error = %ctx.last_error,
            "retrying transient error"
        );
    }
}

/// Composed retry combinator for KV-like operations.
///
/// Uses the given profile, transient error detection, and structured logging.
pub async fn retry_kv<T, E, F, Fut>(profile: RetryProfile, service: &str, mut f: F) -> Result<T, E>
where
    E: TransientError + Display,
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let policy = profile.policy();
    let observer = retry_observer::<E>(service);
    retry_when_observed(&policy, &mut f, |e: &E| e.is_transient(), &observer).await
}

/// Retry an operation with a named profile, structured logging, and a tracing span.
///
/// This is the stock interpreter/runtime helper when callers want the default
/// retry semantics plus span correlation without rebuilding a local wrapper.
pub async fn retry_in_span<T, E, F, Fut>(
    profile: RetryProfile,
    service: &str,
    span: tracing::Span,
    mut f: F,
) -> Result<T, E>
where
    E: TransientError + Display,
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let policy = profile.policy();
    let observer = retry_observer::<E>(service);
    retry_when_observed(&policy, &mut f, |e: &E| e.is_transient(), &observer)
        .instrument(span)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_profile() {
        let policy = RetryProfile::Fast.policy();
        assert_eq!(policy.max_retries(), 3);
        assert_eq!(policy.initial_delay(), Duration::from_millis(50));
    }

    #[test]
    fn normal_profile() {
        let policy = RetryProfile::Normal.policy();
        assert_eq!(policy.max_retries(), 3);
        assert_eq!(policy.initial_delay(), Duration::from_millis(100));
    }

    #[test]
    fn cold_start_profile() {
        let policy = RetryProfile::ColdStart.policy();
        assert_eq!(policy.max_retries(), 5);
        assert_eq!(policy.initial_delay(), Duration::from_millis(200));
    }

    #[test]
    fn db_error_transient() {
        use agent_fw_algebra::target_db::DbError;
        let err = DbError::Connection("connection reset by peer".into());
        assert!(err.is_transient());
    }

    #[test]
    fn db_error_not_transient() {
        use agent_fw_algebra::target_db::DbError;
        let err = DbError::InvalidQuery("bad SQL".into());
        assert!(!err.is_transient());
    }

    #[test]
    fn kv_error_transient() {
        use agent_fw_algebra::kv_store::KVError;
        let err = KVError::Storage("connection refused".into());
        assert!(err.is_transient());
    }

    #[test]
    fn kv_error_not_transient() {
        use agent_fw_algebra::kv_store::KVError;
        let err = KVError::Serialization("invalid json".into());
        assert!(!err.is_transient());
    }

    #[test]
    fn catalog_error_transient() {
        use agent_fw_catalog::CatalogError;
        let err = CatalogError::Unavailable("service unavailable".into());
        assert!(err.is_transient());
    }

    #[test]
    fn catalog_error_not_transient() {
        use agent_fw_catalog::CatalogError;
        let err = CatalogError::NotFound("missing".into());
        assert!(!err.is_transient());
    }

    #[test]
    fn vector_store_error_transient() {
        use agent_fw_algebra::vector_store::VectorStoreError;
        let err = VectorStoreError::Connection("connection reset".into());
        assert!(err.is_transient());
        let err2 = VectorStoreError::Execution("timeout".into());
        assert!(err2.is_transient());
    }

    #[test]
    fn vector_store_error_not_transient() {
        use agent_fw_algebra::vector_store::VectorStoreError;
        let err = VectorStoreError::NotConfigured;
        assert!(!err.is_transient());
        let err2 = VectorStoreError::DimensionMismatch {
            expected: 768,
            actual: 1536,
        };
        assert!(!err2.is_transient());
    }

    #[test]
    fn write_db_error_transient() {
        use agent_fw_algebra::writable_db::WriteDbError;
        assert!(WriteDbError::Connection("connection reset".into()).is_transient());
        assert!(WriteDbError::Ddl("connection refused".into()).is_transient());
        assert!(WriteDbError::Dml("timeout".into()).is_transient());
        assert!(WriteDbError::Transaction("deadlock".into()).is_transient());
        assert!(WriteDbError::Timeout(Duration::from_secs(30)).is_transient());
    }

    #[test]
    fn write_db_error_not_transient() {
        use agent_fw_algebra::writable_db::WriteDbError;
        assert!(!WriteDbError::InvalidSql("bad SQL".into()).is_transient());
        // DDL/DML errors without transient keywords are not retried
        assert!(!WriteDbError::Ddl("syntax error near 'FOO'".into()).is_transient());
    }

    #[tokio::test]
    async fn retry_in_span_retries_transient_error() {
        #[derive(Debug, Clone)]
        struct TestErr(&'static str);

        impl std::fmt::Display for TestErr {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl TransientError for TestErr {
            fn error_message(&self) -> Option<&str> {
                Some(self.0)
            }
        }

        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = attempts.clone();

        let result = retry_in_span(
            RetryProfile::Fast,
            "test-service",
            tracing::Span::none(),
            move || {
                let seen = seen.clone();
                async move {
                    let attempt = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if attempt == 0 {
                        Err(TestErr("connection reset"))
                    } else {
                        Ok::<_, TestErr>(42)
                    }
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(result, 42);
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
