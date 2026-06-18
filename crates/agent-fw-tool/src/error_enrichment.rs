//! Error enrichment registry — reusable building blocks for error recovery hints.
//!
//! Domain code owns specific enrichment rules; the framework provides
//! the combinable, composable building blocks.
//!
//! # Design
//!
//! - [`ErrorEnricher`] trait: single method `enrich(error) -> error`
//! - [`PatternEnricher`]: builder-pattern enricher that matches error messages
//! - [`IdentityEnricher`]: monoid identity (passes errors through unchanged)
//! - [`ComposedEnricher`]: chains two enrichers (monoid `append`)
//!
//! # Laws
//!
//! - **Identity**: `identity.enrich(e) == e`
//! - **Associativity**: `(a.then(b)).then(c).enrich(e) == a.then(b.then(c)).enrich(e)`
//!
//! Verified by `agent_fw_test::error_enricher_laws`.
//!
//! # Why dynamic dispatch (`Box<dyn ErrorEnricher>`)
//!
//! Enricher chains are built at startup and never inspected — the only
//! operation is `enrich(error) -> error`. The monoid laws (identity,
//! associativity) hold regardless of whether composition uses generics
//! or dynamic dispatch. Since the set of enrichers is open-world (domain
//! code contributes rules that the framework doesn't know about at compile
//! time), `Box<dyn ErrorEnricher>` is the honest encoding: it says
//! "any enricher" rather than requiring a closed enum. The cost is one
//! vtable indirection per chain link, which is negligible for error-path code.

use crate::error::{ErrorKind, ToolError};

/// Trait for enriching tool errors with recovery hints.
///
/// Enrichers are composable: chain them with [`ComposedEnricher`] or
/// the [`ErrorEnricher::then`] combinator.
pub trait ErrorEnricher: Send + Sync {
    /// Enrich an error with additional context, hints, or kind overrides.
    ///
    /// Must not panic. If no pattern matches, return the error unchanged.
    fn enrich(&self, error: ToolError) -> ToolError;

    /// Compose this enricher with another (this runs first, then `other`).
    fn then<E: ErrorEnricher + 'static>(self, other: E) -> ComposedEnricher
    where
        Self: Sized + 'static,
    {
        ComposedEnricher {
            first: Box::new(self),
            second: Box::new(other),
        }
    }
}

/// Identity enricher — monoid identity. Passes errors through unchanged.
pub struct IdentityEnricher;

impl ErrorEnricher for IdentityEnricher {
    fn enrich(&self, error: ToolError) -> ToolError {
        error
    }
}

/// Composed enricher — runs `first`, then `second`.
pub struct ComposedEnricher {
    first: Box<dyn ErrorEnricher>,
    second: Box<dyn ErrorEnricher>,
}

impl ErrorEnricher for ComposedEnricher {
    fn enrich(&self, error: ToolError) -> ToolError {
        let enriched = self.first.enrich(error);
        self.second.enrich(enriched)
    }
}

/// A pattern-matching rule for error enrichment.
struct PatternRule {
    /// Substring to match in the error message.
    pattern: String,
    /// Optional error kind override.
    kind: Option<ErrorKind>,
    /// Hints to add when the pattern matches.
    hints: Vec<String>,
}

/// Immutable pattern-matching enricher. Apply via `ErrorEnricher::enrich`.
///
/// Construct via [`PatternEnricherBuilder`] or [`PatternEnricher::from_domain_patterns`].
///
/// # Example
///
/// ```ignore
/// let enricher = PatternEnricherBuilder::new()
///     .on_contains("no matching products", ErrorKind::NotFound)
///         .with_hint("Try using query_data first")
///         .with_hint("Or broaden the search criteria")
///     .on_contains("connection refused", ErrorKind::Storage)
///         .with_hint("Check database connectivity")
///     .build();
/// ```
pub struct PatternEnricher {
    rules: Vec<PatternRule>,
}

// ─── Typestate builder ───────────────────────────────────────────────

/// Typestate: no rule has been added yet. `with_hint()` is unavailable.
pub struct NeedPattern;
/// Typestate: at least one rule exists. `with_hint()` and `build()` are available.
pub struct HasPattern;

/// Builder for [`PatternEnricher`] with phantom typestate.
///
/// `S` is either [`NeedPattern`] or [`HasPattern`]. The `with_hint()` method
/// only exists when `S = HasPattern`, making it a compile-time error to call
/// `with_hint()` before `on_contains()`.
pub struct PatternEnricherBuilder<S> {
    rules: Vec<PatternRule>,
    _state: std::marker::PhantomData<S>,
}

impl PatternEnricherBuilder<NeedPattern> {
    /// Create a new builder with no rules.
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            _state: std::marker::PhantomData,
        }
    }

    /// Add the first pattern match rule. Transitions to `HasPattern` state.
    pub fn on_contains(
        self,
        pattern: impl Into<String>,
        kind: ErrorKind,
    ) -> PatternEnricherBuilder<HasPattern> {
        let mut rules = self.rules;
        rules.push(PatternRule {
            pattern: pattern.into(),
            kind: Some(kind),
            hints: Vec::new(),
        });
        PatternEnricherBuilder {
            rules,
            _state: std::marker::PhantomData,
        }
    }
}

impl Default for PatternEnricherBuilder<NeedPattern> {
    fn default() -> Self {
        Self::new()
    }
}

impl PatternEnricherBuilder<HasPattern> {
    /// Add another pattern match rule.
    pub fn on_contains(mut self, pattern: impl Into<String>, kind: ErrorKind) -> Self {
        self.rules.push(PatternRule {
            pattern: pattern.into(),
            kind: Some(kind),
            hints: Vec::new(),
        });
        self
    }

    /// Add a hint to the most recently added rule.
    ///
    /// Only available after `on_contains()` — the typestate guarantees
    /// at least one rule exists, so the `unwrap()` cannot fail.
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.rules
            .last_mut()
            .unwrap() // Structurally guaranteed by HasPattern
            .hints
            .push(hint.into());
        self
    }

    /// Consume the builder and produce an immutable [`PatternEnricher`].
    pub fn build(self) -> PatternEnricher {
        PatternEnricher { rules: self.rules }
    }
}

impl PatternEnricher {
    /// Build from a list of (pattern, kind, hints) tuples.
    /// Ergonomic shorthand for domain implementations.
    pub fn from_domain_patterns(patterns: Vec<(&str, Option<ErrorKind>, Vec<&str>)>) -> Self {
        let mut rules = Vec::new();
        for (pattern, kind, hints) in patterns {
            rules.push(PatternRule {
                pattern: pattern.to_string(),
                kind,
                hints: hints.into_iter().map(|h| h.to_string()).collect(),
            });
        }
        PatternEnricher { rules }
    }
}

impl Default for PatternEnricher {
    fn default() -> Self {
        PatternEnricher { rules: Vec::new() }
    }
}

// ─── String-based enrichment (for FFI / Python consumers) ───────────

/// A lightweight string → string enricher.
///
/// Same first-match-wins semantics as [`PatternEnricher`], but operates on
/// plain strings instead of [`ToolError`].  Suitable for Python consumers
/// that work with error messages directly.
#[derive(Clone)]
pub struct StringPatternEnricher {
    rules: Vec<StringRule>,
}

#[derive(Clone)]
struct StringRule {
    pattern: String,
    hints: Vec<String>,
}

impl StringPatternEnricher {
    /// Create a new empty enricher.
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Register a pattern with one or more hints.  Returns self for chaining.
    pub fn on(mut self, pattern: impl Into<String>, hints: Vec<String>) -> Self {
        self.rules.push(StringRule {
            pattern: pattern.into(),
            hints,
        });
        self
    }

    /// Enrich an error message.  First matching pattern wins.
    /// Returns the original message unchanged when no pattern matches.
    pub fn enrich(&self, message: &str) -> String {
        let lower = message.to_lowercase();
        for rule in &self.rules {
            if lower.contains(&rule.pattern.to_lowercase()) {
                if rule.hints.is_empty() {
                    return message.to_string();
                }
                let hints = rule
                    .hints
                    .iter()
                    .map(|h| format!("  - {h}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                return format!("{message}\n\nSuggestions:\n{hints}");
            }
        }
        message.to_string()
    }

    /// Number of registered rules.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// True if no rules are registered.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Compose two enrichers: self's rules first, then other's rules.
    /// First-match-wins across the combined rule set.
    ///
    /// This is the monoid append: `StringPatternEnricher::new()` is identity.
    pub fn compose(mut self, other: StringPatternEnricher) -> StringPatternEnricher {
        self.rules.extend(other.rules);
        self
    }
}

impl Default for StringPatternEnricher {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Pre-built enrichers ────────────────────────────────────────────

/// Pre-configured enricher for plan-building errors.
///
/// Covers the 7 common failure categories:
/// empty entity sets, expired entity references, constraint validation,
/// general validation, database/SQL, serialization/JSON, storage/KV.
pub fn build_plan_enricher() -> StringPatternEnricher {
    StringPatternEnricher::new()
        .on(
            "no entities",
            vec![
                "Try broadening the entity filter (fewer terms, different columns).".into(),
                "Use query_data first to verify the entity names match the data model.".into(),
            ],
        )
        .on(
            "0 entities",
            vec![
                "Try broadening the entity filter (fewer terms, different columns).".into(),
                "Use query_data first to verify the entity names match the data model.".into(),
            ],
        )
        .on(
            "entity_set",
            vec![
                "The entity set may have expired. Create a new one with create_entity_set.".into(),
            ],
        )
        .on(
            "entityset",
            vec![
                "The entity set may have expired. Create a new one with create_entity_set.".into(),
            ],
        )
        .on(
            "constraint",
            vec!["Check that constraint values are valid numbers (e.g., ceiling: '15.00').".into()],
        )
        .on(
            "validation",
            vec!["Check that at least one action is provided and entity filters are valid.".into()],
        )
        .on(
            "database",
            vec![
                "The query may reference columns that don't exist. Use get_table_columns to verify."
                    .into(),
            ],
        )
        .on(
            "sql",
            vec![
                "The query may reference columns that don't exist. Use get_table_columns to verify."
                    .into(),
            ],
        )
        .on(
            "query",
            vec![
                "The query may reference columns that don't exist. Use get_table_columns to verify."
                    .into(),
            ],
        )
        .on(
            "serialization",
            vec!["Check that action values are valid (e.g., numeric strings for decimals).".into()],
        )
        .on(
            "json",
            vec!["Check that action values are valid (e.g., numeric strings for decimals).".into()],
        )
        .on(
            "storage",
            vec!["This is a transient infrastructure issue. Retry the operation.".into()],
        )
        .on(
            "kv",
            vec!["This is a transient infrastructure issue. Retry the operation.".into()],
        )
}

/// Convenience: enrich a build-plan error message using the default patterns.
pub fn enrich_build_error(message: &str) -> String {
    // Static-like — we rebuild each time which is cheap (no allocations beyond the vec).
    // If hot-path perf matters, callers should cache the enricher.
    build_plan_enricher().enrich(message)
}

impl ErrorEnricher for PatternEnricher {
    fn enrich(&self, mut error: ToolError) -> ToolError {
        let msg = error.message().to_lowercase();
        for rule in &self.rules {
            if msg.contains(&rule.pattern.to_lowercase()) {
                if let Some(kind) = rule.kind {
                    error = error.with_kind(kind);
                }
                for hint in &rule.hints {
                    error = error.with_hint(hint.clone());
                }
                // First match wins — consistent with pattern matching semantics
                break;
            }
        }
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_passes_through() {
        let enricher = IdentityEnricher;
        let error = ToolError::domain("test error");
        let result = enricher.enrich(error);
        assert_eq!(result.message(), "test error");
        assert_eq!(result.kind(), ErrorKind::Domain);
        assert!(!result.has_hints());
    }

    #[test]
    fn pattern_enricher_matches_and_adds_hints() {
        let enricher = PatternEnricherBuilder::new()
            .on_contains("no matching products", ErrorKind::NotFound)
            .with_hint("Try query_data")
            .with_hint("Or broaden filter")
            .build();

        let error = ToolError::domain("Error: no matching products found");
        let result = enricher.enrich(error);

        assert_eq!(result.kind(), ErrorKind::NotFound);
        assert_eq!(result.hints().len(), 2);
        assert!(result.hints()[0].contains("query_data"));
    }

    #[test]
    fn pattern_enricher_case_insensitive() {
        let enricher = PatternEnricherBuilder::new()
            .on_contains("connection refused", ErrorKind::Storage)
            .with_hint("Check connectivity")
            .build();

        let error = ToolError::domain("Connection Refused by server");
        let result = enricher.enrich(error);

        assert_eq!(result.kind(), ErrorKind::Storage);
        assert_eq!(result.hints().len(), 1);
    }

    #[test]
    fn pattern_enricher_no_match_passes_through() {
        let enricher = PatternEnricherBuilder::new()
            .on_contains("specific error", ErrorKind::NotFound)
            .with_hint("Hint")
            .build();

        let error = ToolError::domain("different error");
        let result = enricher.enrich(error);

        assert_eq!(result.kind(), ErrorKind::Domain); // unchanged
        assert!(!result.has_hints());
    }

    #[test]
    fn pattern_enricher_first_match_wins() {
        let enricher = PatternEnricherBuilder::new()
            .on_contains("error", ErrorKind::NotFound)
            .with_hint("First hint")
            .on_contains("error", ErrorKind::Storage)
            .with_hint("Second hint")
            .build();

        let error = ToolError::domain("some error occurred");
        let result = enricher.enrich(error);

        assert_eq!(result.kind(), ErrorKind::NotFound); // first rule wins
        assert_eq!(result.hints().len(), 1);
        assert!(result.hints()[0].contains("First"));
    }

    #[test]
    fn composed_enricher_chains() {
        let first = PatternEnricherBuilder::new()
            .on_contains("timeout", ErrorKind::Storage)
            .with_hint("Retry later")
            .build();
        let second = PatternEnricherBuilder::new()
            .on_contains("timeout", ErrorKind::Database)
            .with_hint("Check DB")
            .build();

        // first runs, sets kind=Storage + hint. second runs, overrides kind=Database + adds hint.
        let composed = first.then(second);

        let error = ToolError::domain("Connection timeout");
        let result = composed.enrich(error);

        // Second enricher overrides kind
        assert_eq!(result.kind(), ErrorKind::Database);
        // Both hints accumulated
        assert_eq!(result.hints().len(), 2);
    }

    #[test]
    fn identity_is_neutral_in_composition() {
        let enricher = PatternEnricherBuilder::new()
            .on_contains("fail", ErrorKind::NotFound)
            .with_hint("Check input")
            .build();

        let composed = IdentityEnricher.then(enricher);

        let error = ToolError::domain("Operation failed");
        let result = composed.enrich(error);

        assert_eq!(result.kind(), ErrorKind::NotFound);
        assert_eq!(result.hints().len(), 1);
    }

    #[test]
    fn empty_pattern_enricher_is_identity() {
        let enricher = PatternEnricher::default();
        let error = ToolError::not_found("missing");
        let result = enricher.enrich(error);
        assert_eq!(result.kind(), ErrorKind::NotFound);
        assert_eq!(result.message(), "missing");
    }

    // ── StringPatternEnricher tests ─────────────────────────────────

    #[test]
    fn string_enricher_identity_when_empty() {
        let enricher = StringPatternEnricher::new();
        assert_eq!(enricher.enrich("hello"), "hello");
    }

    #[test]
    fn string_enricher_adds_hints() {
        let enricher =
            StringPatternEnricher::new().on("fail", vec!["Check input".into(), "Retry".into()]);
        let result = enricher.enrich("Operation failed");
        assert!(result.contains("Suggestions:"));
        assert!(result.contains("  - Check input"));
        assert!(result.contains("  - Retry"));
    }

    #[test]
    fn string_enricher_case_insensitive() {
        let enricher = StringPatternEnricher::new().on("error", vec!["Fix it".into()]);
        let result = enricher.enrich("An ERROR occurred");
        assert!(result.contains("Fix it"));
    }

    #[test]
    fn string_enricher_first_match_wins() {
        let enricher = StringPatternEnricher::new()
            .on("error", vec!["First".into()])
            .on("error", vec!["Second".into()]);
        let result = enricher.enrich("some error");
        assert!(result.contains("First"));
        assert!(!result.contains("Second"));
    }

    #[test]
    fn string_enricher_no_match_passthrough() {
        let enricher = StringPatternEnricher::new().on("xyz", vec!["Hint".into()]);
        assert_eq!(enricher.enrich("abc"), "abc");
    }

    // ── enrich_build_error tests ────────────────────────────────────

    #[test]
    fn build_error_no_entities() {
        let result = enrich_build_error("No entities found (0 entities)");
        assert!(result.contains("Suggestions:"));
        assert!(result.contains("query_data"));
    }

    #[test]
    fn build_error_entity_set_expired() {
        let result = enrich_build_error("entity_set not found");
        assert!(result.contains("expired"));
    }

    #[test]
    fn build_error_constraint() {
        let result = enrich_build_error("constraint validation failed");
        assert!(result.contains("valid numbers"));
    }

    #[test]
    fn build_error_database() {
        let result = enrich_build_error("database error: column not found");
        assert!(result.contains("get_table_columns"));
    }

    #[test]
    fn build_error_passthrough() {
        let msg = "Something completely unknown";
        assert_eq!(enrich_build_error(msg), msg);
    }
}
