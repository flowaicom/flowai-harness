//! ErrorEnricher algebraic law test harness.
//!
//! Verifies that `ErrorEnricher` implementations satisfy:
//!
//! - **E1 (Identity)**: `IdentityEnricher.enrich(e) == e`
//! - **E2 (Associativity)**: `(a.then(b)).then(c).enrich(e) == a.then(b.then(c)).enrich(e)`
//! - **E3 (First-match-wins)**: `PatternEnricher` applies only the first matching rule
//! - **E4 (Empty is identity)**: `IdentityEnricher.enrich(e) == e` (identity element of the monoid)
//!
//! # Usage
//!
//! ```ignore
//! #[test]
//! fn error_enricher_laws() {
//!     agent_fw_test::error_enricher_laws::test_all();
//! }
//! ```

use agent_fw_tool::{ErrorEnricher, IdentityEnricher, PatternEnricherBuilder};
use agent_fw_tool::{ErrorKind, ToolError};

fn make_error(msg: &str) -> ToolError {
    ToolError::msg(msg)
}

/// Run all error enricher laws.
pub fn test_all() {
    law_identity();
    law_associativity();
    law_first_match_wins();
    law_empty_is_identity();
}

/// E1 (Identity): `IdentityEnricher.enrich(e)` preserves message and kind.
pub fn law_identity() {
    let e = make_error("something went wrong");
    let enriched = IdentityEnricher.enrich(e);
    assert_eq!(enriched.message(), "something went wrong");
    assert_eq!(enriched.kind(), ErrorKind::Domain);
}

/// E2 (Associativity): `(a.then(b)).then(c).enrich(e) == a.then(b.then(c)).enrich(e)`.
///
/// We use three PatternEnrichers that match different substrings.
/// The enrichment result (hints, kind) must be identical regardless
/// of how the enrichers are composed.
pub fn law_associativity() {
    let a = PatternEnricherBuilder::new()
        .on_contains("product", ErrorKind::NotFound)
        .with_hint("hint-a")
        .build();

    let b = PatternEnricherBuilder::new()
        .on_contains("product", ErrorKind::InvalidInput)
        .with_hint("hint-b")
        .build();

    let c = PatternEnricherBuilder::new()
        .on_contains("product", ErrorKind::Storage)
        .with_hint("hint-c")
        .build();

    let e1 = make_error("no product found");
    let e2 = make_error("no product found");

    // (a.then(b)).then(c)
    let left = a.then(b).then(c);
    let result_left = left.enrich(e1);

    // a.then(b.then(c)) — need fresh enrichers since then() consumes
    let a2 = PatternEnricherBuilder::new()
        .on_contains("product", ErrorKind::NotFound)
        .with_hint("hint-a")
        .build();
    let b2 = PatternEnricherBuilder::new()
        .on_contains("product", ErrorKind::InvalidInput)
        .with_hint("hint-b")
        .build();
    let c2 = PatternEnricherBuilder::new()
        .on_contains("product", ErrorKind::Storage)
        .with_hint("hint-c")
        .build();

    let right = a2.then(b2.then(c2));
    let result_right = right.enrich(e2);

    assert_eq!(result_left.kind(), result_right.kind(), "E2: kind mismatch");
    assert_eq!(
        result_left.hints(),
        result_right.hints(),
        "E2: hints mismatch"
    );
}

/// E3 (First-match-wins): Only the first matching rule in a PatternEnricher applies.
pub fn law_first_match_wins() {
    let enricher = PatternEnricherBuilder::new()
        .on_contains("error", ErrorKind::NotFound)
        .with_hint("first-rule")
        .on_contains("error", ErrorKind::Storage)
        .with_hint("second-rule")
        .build();

    let e = make_error("an error occurred");
    let enriched = enricher.enrich(e);

    // First rule should win
    assert_eq!(
        enriched.kind(),
        ErrorKind::NotFound,
        "E3: first rule must win for kind"
    );
    assert!(
        enriched.hints().contains(&"first-rule".to_string()),
        "E3: first rule hint must be present"
    );
    assert!(
        !enriched.hints().contains(&"second-rule".to_string()),
        "E3: second rule hint must not be present"
    );
}

/// E4 (Empty is identity): The identity enricher preserves the error unchanged.
pub fn law_empty_is_identity() {
    let enricher = IdentityEnricher;
    let e = make_error("test error");
    let enriched = enricher.enrich(e);

    assert_eq!(
        enriched.message(),
        "test error",
        "E4: message must be preserved"
    );
    assert_eq!(
        enriched.kind(),
        ErrorKind::Domain,
        "E4: kind must be preserved"
    );
    assert!(enriched.hints().is_empty(), "E4: no hints should be added");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_laws() {
        test_all();
    }
}
