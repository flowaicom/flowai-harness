//! Universal tool error wrapper with structured diagnostics.
//!
//! `ToolError` carries a message, an `ErrorKind` classification, and optional
//! recovery hints. The `ErrorKind` allows programmatic routing (retry? ask
//! the user? log and continue?) while hints provide LLM-friendly guidance.
//!
//! # Backward Compatibility
//!
//! All existing constructors (`wrap`, `msg`, `cancelled`, `missing_ext`,
//! `domain`, `invalid_input`) produce identical `Display` output (no hints
//! by default). Code that calls `e.to_string()` or `e.message()` is unchanged.

/// Classification of tool errors for programmatic routing.
///
/// Consumers can match on `kind()` to decide recovery strategy:
/// - `Domain` / `InvalidInput` → ask the LLM to self-correct
/// - `NotFound` → suggest alternative search terms
/// - `Storage` / `Database` → transient, retry
/// - `Serialization` → check numeric values
/// - `Cancelled` → user-initiated, do not retry
/// - `MissingDependency` → configuration error
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Domain,
    InvalidInput,
    NotFound,
    Storage,
    Serialization,
    Cancelled,
    MissingDependency,
    Database,
}

/// Universal tool error wrapper with structured diagnostics.
///
/// Erases the domain error into a string representation, avoiding the need for
/// identical wrapper types in every tool module. Now carries an `ErrorKind`
/// and optional hints for LLM recovery.
///
/// # Example
///
/// ```ignore
/// let result = some_fallible_op().await.map_err(ToolError::wrap)?;
///
/// // With diagnostics:
/// ToolError::not_found("Entity 'xyz' not found")
///     .with_hint("Try using query_data first to verify entity names")
///     .with_hint("Or provide an entitySetId from a previous search")
/// ```
#[derive(Debug)]
pub struct ToolError {
    message: String,
    kind: ErrorKind,
    hints: Vec<String>,
}

impl ToolError {
    /// Wrap any displayable error into a `ToolError`.
    pub fn wrap(e: impl std::fmt::Display) -> Self {
        Self {
            message: e.to_string(),
            kind: ErrorKind::Domain,
            hints: Vec::new(),
        }
    }

    /// Create a ToolError from a raw message.
    pub fn msg(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ErrorKind::Domain,
            hints: Vec::new(),
        }
    }

    /// Create a cancellation error.
    pub fn cancelled() -> Self {
        Self {
            message: "Operation cancelled".to_string(),
            kind: ErrorKind::Cancelled,
            hints: Vec::new(),
        }
    }

    /// A required ToolEnvironment extension was not registered.
    ///
    /// Returned by `env.try_ext::<dyn T>()` when the extension
    /// was not registered via `with_ext`.
    pub fn missing_ext(type_name: &str) -> Self {
        Self {
            message: format!(
                "Missing required extension: {type_name}. \
                 Register it via ToolEnvironment::with_ext()."
            ),
            kind: ErrorKind::MissingDependency,
            hints: Vec::new(),
        }
    }

    /// A domain-specific error with an enriched message for LLM recovery.
    pub fn domain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ErrorKind::Domain,
            hints: Vec::new(),
        }
    }

    /// Input validation failed (beyond schema-level validation).
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self {
            message: format!("Invalid input: {}", message.into()),
            kind: ErrorKind::InvalidInput,
            hints: Vec::new(),
        }
    }

    /// Resource not found.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ErrorKind::NotFound,
            hints: Vec::new(),
        }
    }

    /// Storage / KV error (transient, safe to retry).
    pub fn storage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ErrorKind::Storage,
            hints: Vec::new(),
        }
    }

    /// Database query error.
    pub fn database(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ErrorKind::Database,
            hints: Vec::new(),
        }
    }

    /// Serialization / deserialization error.
    pub fn serialization(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ErrorKind::Serialization,
            hints: Vec::new(),
        }
    }

    /// Add a recovery hint for LLM guidance. Chainable.
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hints.push(hint.into());
        self
    }

    /// Override the error kind. Chainable.
    pub fn with_kind(mut self, kind: ErrorKind) -> Self {
        self.kind = kind;
        self
    }

    /// Get the error message (without hints).
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Get the error classification.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Get the recovery hints.
    pub fn hints(&self) -> &[String] {
        &self.hints
    }

    /// Whether any hints are attached.
    pub fn has_hints(&self) -> bool {
        !self.hints.is_empty()
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        if !self.hints.is_empty() {
            write!(f, "\n\nSuggestions:")?;
            for hint in &self.hints {
                write!(f, "\n  - {hint}")?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for ToolError {}

impl From<agent_fw_algebra::KVError> for ToolError {
    fn from(err: agent_fw_algebra::KVError) -> Self {
        ToolError {
            message: err.to_string(),
            kind: ErrorKind::Storage,
            hints: Vec::new(),
        }
    }
}

impl From<agent_fw_algebra::SubAgentError> for ToolError {
    fn from(err: agent_fw_algebra::SubAgentError) -> Self {
        ToolError::wrap(err)
    }
}

impl From<serde_json::Error> for ToolError {
    fn from(err: serde_json::Error) -> Self {
        ToolError {
            message: err.to_string(),
            kind: ErrorKind::Serialization,
            hints: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_displays_error() {
        let err = ToolError::wrap("something failed");
        assert_eq!(err.to_string(), "something failed");
    }

    #[test]
    fn msg_creates_from_string() {
        let err = ToolError::msg("custom message");
        assert_eq!(err.message(), "custom message");
    }

    #[test]
    fn cancelled_has_standard_message() {
        let err = ToolError::cancelled();
        assert_eq!(err.message(), "Operation cancelled");
        assert_eq!(err.kind(), ErrorKind::Cancelled);
    }

    #[test]
    fn implements_std_error() {
        let err: Box<dyn std::error::Error> = Box::new(ToolError::msg("test"));
        assert_eq!(err.to_string(), "test");
    }

    #[test]
    fn from_kv_error() {
        let kv_err = agent_fw_algebra::KVError::Storage("connection lost".into());
        let tool_err: ToolError = kv_err.into();
        assert!(tool_err.message().contains("connection lost"));
        assert_eq!(tool_err.kind(), ErrorKind::Storage);
    }

    #[test]
    fn missing_ext_includes_type_name() {
        let err = ToolError::missing_ext("TargetDatabase");
        assert!(err.message().contains("TargetDatabase"));
        assert!(err.message().contains("with_ext"));
        assert_eq!(err.kind(), ErrorKind::MissingDependency);
    }

    #[test]
    fn domain_preserves_message() {
        let err = ToolError::domain("Plan has no actions");
        assert_eq!(err.message(), "Plan has no actions");
        assert_eq!(err.kind(), ErrorKind::Domain);
    }

    #[test]
    fn invalid_input_prefixed() {
        let err = ToolError::invalid_input("productIds must not be empty");
        assert!(err.message().starts_with("Invalid input:"));
        assert!(err.message().contains("productIds"));
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    // ── New diagnostic tests ──────────────────────────────────

    #[test]
    fn hint_chaining() {
        let err = ToolError::not_found("Entity 'xyz' not found")
            .with_hint("Try query_data first")
            .with_hint("Or provide an entitySetId");
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(err.has_hints());
        assert_eq!(err.hints().len(), 2);
        assert!(err.hints()[0].contains("query_data"));
    }

    #[test]
    fn display_with_hints() {
        let err = ToolError::not_found("No products matched")
            .with_hint("Broaden the filter")
            .with_hint("Check column names");
        let display = err.to_string();
        assert!(display.starts_with("No products matched"));
        assert!(display.contains("Suggestions:"));
        assert!(display.contains("  - Broaden the filter"));
        assert!(display.contains("  - Check column names"));
    }

    #[test]
    fn display_without_hints_unchanged() {
        let err = ToolError::domain("simple error");
        assert_eq!(err.to_string(), "simple error");
        assert!(!err.has_hints());
    }

    #[test]
    fn with_kind_override() {
        let err = ToolError::wrap("db timeout").with_kind(ErrorKind::Database);
        assert_eq!(err.kind(), ErrorKind::Database);
    }

    #[test]
    fn smart_constructors_set_kind() {
        assert_eq!(ToolError::not_found("x").kind(), ErrorKind::NotFound);
        assert_eq!(ToolError::storage("x").kind(), ErrorKind::Storage);
        assert_eq!(ToolError::database("x").kind(), ErrorKind::Database);
        assert_eq!(
            ToolError::serialization("x").kind(),
            ErrorKind::Serialization
        );
    }

    #[test]
    fn from_serde_error_sets_serialization_kind() {
        let serde_err: serde_json::Error = serde_json::from_str::<i32>("bad").unwrap_err();
        let tool_err: ToolError = serde_err.into();
        assert_eq!(tool_err.kind(), ErrorKind::Serialization);
    }

    #[test]
    fn backward_compat_wrap_display() {
        // Existing code that calls .to_string() sees the same output
        let err = ToolError::wrap("something failed");
        assert_eq!(format!("{err}"), "something failed");
    }
}
