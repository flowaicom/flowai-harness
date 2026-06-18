//! Token usage types with monoid instance for composition
//!
//! # Design Principle: Never Store Derived Values
//!
//! `total_tokens` is computed during serialization, not stored.
//! This keeps data simple, deriving values when needed.
//!
//! # Laws (Monoid)
//!
//! TokenUsage operations satisfy monoid laws:
//! - L1. Identity:      combine(ZERO, a) = a = combine(a, ZERO)
//! - L2. Associativity: combine(combine(a, b), c) = combine(a, combine(b, c))

use serde::{ser::SerializeStruct, Deserialize, Serialize, Serializer};

use crate::algebra::{Monoid, Semigroup};

/// Token usage statistics.
///
/// Stores `prompt_tokens`, `completion_tokens`, `cache_read_input_tokens`, and
/// `cache_creation_input_tokens`.
/// The `total_tokens` field is computed during serialization.
///
/// Cache read/write tokens are both subsets of `prompt_tokens` (not additive),
/// so `total()` remains `prompt_tokens + completion_tokens`. These fields enable
/// accurate cost reporting since cache reads and cache writes are billed at
/// distinct rates by Anthropic/Bedrock.
///
/// # Monoid Laws
///
/// ```text
/// combine(a, ZERO) = a = combine(ZERO, a)    # Identity
/// combine(combine(a, b), c) = combine(a, combine(b, c))  # Associativity
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Prompt-cache read tokens (subset of `prompt_tokens`, not additive to total).
    pub cache_read_input_tokens: u64,
    /// Prompt-cache write tokens (subset of `prompt_tokens`, not additive to total).
    pub cache_creation_input_tokens: u64,
}

impl TokenUsage {
    /// Monoid identity element.
    pub const ZERO: Self = Self {
        prompt_tokens: 0,
        completion_tokens: 0,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
    };

    /// Create a new TokenUsage with all four fields.
    pub fn new(
        prompt_tokens: u64,
        completion_tokens: u64,
        cache_read_input_tokens: u64,
        cache_creation_input_tokens: u64,
    ) -> Self {
        Self {
            prompt_tokens,
            completion_tokens,
            cache_read_input_tokens,
            cache_creation_input_tokens,
        }
    }

    /// Create a TokenUsage without cache tracking (convenience for call sites
    /// that don't track cache reads).
    pub fn simple(prompt_tokens: u64, completion_tokens: u64) -> Self {
        Self {
            prompt_tokens,
            completion_tokens,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        }
    }

    /// Compute total tokens (derived, never stored).
    ///
    /// Cache reads are a subset of prompt tokens, so total remains
    /// `prompt_tokens + completion_tokens`.
    ///
    /// Uses saturating arithmetic to prevent overflow panics.
    #[inline]
    pub fn total(&self) -> u64 {
        self.prompt_tokens.saturating_add(self.completion_tokens)
    }

    /// Monoid combine operation.
    ///
    /// Uses saturating arithmetic: values cap at u64::MAX rather than overflow.
    /// This is correct for token counting - realistically you'll never hit the
    /// limit, but if you do, capping is more sensible than wrapping.
    ///
    /// # Laws
    /// - Associativity: (a.combine(b)).combine(c) == a.combine(b.combine(c))
    /// - Identity: a.combine(ZERO) == a == ZERO.combine(a)
    pub fn combine(&self, other: &Self) -> Self {
        Self {
            prompt_tokens: self.prompt_tokens.saturating_add(other.prompt_tokens),
            completion_tokens: self
                .completion_tokens
                .saturating_add(other.completion_tokens),
            cache_read_input_tokens: self
                .cache_read_input_tokens
                .saturating_add(other.cache_read_input_tokens),
            cache_creation_input_tokens: self
                .cache_creation_input_tokens
                .saturating_add(other.cache_creation_input_tokens),
        }
    }

    /// Check if this is the identity element.
    pub fn is_zero(&self) -> bool {
        self.prompt_tokens == 0
            && self.completion_tokens == 0
            && self.cache_read_input_tokens == 0
            && self.cache_creation_input_tokens == 0
    }
}

/// Semigroup instance: saturating addition.
impl Semigroup for TokenUsage {
    #[inline]
    fn combine(&self, other: &Self) -> Self {
        Self {
            prompt_tokens: self.prompt_tokens.saturating_add(other.prompt_tokens),
            completion_tokens: self
                .completion_tokens
                .saturating_add(other.completion_tokens),
            cache_read_input_tokens: self
                .cache_read_input_tokens
                .saturating_add(other.cache_read_input_tokens),
            cache_creation_input_tokens: self
                .cache_creation_input_tokens
                .saturating_add(other.cache_creation_input_tokens),
        }
    }
}

/// Monoid instance: zero usage as identity.
impl Monoid for TokenUsage {
    #[inline]
    fn empty() -> Self {
        Self::ZERO
    }
}

/// Custom serialization to include computed totals and explicit cache fields.
impl Serialize for TokenUsage {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut state = s.serialize_struct("TokenUsage", 5)?;
        state.serialize_field("promptTokens", &self.prompt_tokens)?;
        state.serialize_field("completionTokens", &self.completion_tokens)?;
        state.serialize_field("cacheReadInputTokens", &self.cache_read_input_tokens)?;
        state.serialize_field(
            "cacheCreationInputTokens",
            &self.cache_creation_input_tokens,
        )?;
        state.serialize_field("totalTokens", &self.total())?;
        state.end()
    }
}

/// Custom deserialization handles presence/absence of all fields.
impl<'de> Deserialize<'de> for TokenUsage {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Helper {
            prompt_tokens: u64,
            completion_tokens: u64,
            #[serde(default)]
            cache_read_input_tokens: u64,
            #[serde(default)]
            cache_creation_input_tokens: u64,
            #[serde(default)]
            #[allow(dead_code)]
            total_tokens: Option<u64>, // Ignored on deserialize
        }
        let helper = Helper::deserialize(d)?;
        Ok(Self {
            prompt_tokens: helper.prompt_tokens,
            completion_tokens: helper.completion_tokens,
            cache_read_input_tokens: helper.cache_read_input_tokens,
            cache_creation_input_tokens: helper.cache_creation_input_tokens,
        })
    }
}

//=============================================================================
// FREE FUNCTIONS for monoid operations
//=============================================================================

/// Get the identity element (zero usage).
pub fn zero_usage() -> TokenUsage {
    TokenUsage::ZERO
}

/// Combine two usages (monoid operation).
pub fn combine_usage(a: &TokenUsage, b: &TokenUsage) -> TokenUsage {
    a.combine(b)
}

/// Fold a collection of usages into a single usage.
pub fn fold_usage<'a>(usages: impl IntoIterator<Item = &'a TokenUsage>) -> TokenUsage {
    usages
        .into_iter()
        .fold(TokenUsage::ZERO, |acc, u| acc.combine(u))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_is_identity() {
        let usage = TokenUsage::simple(100, 50);
        assert_eq!(usage.combine(&TokenUsage::ZERO), usage);
        assert_eq!(TokenUsage::ZERO.combine(&usage), usage);
    }

    #[test]
    fn total_is_computed() {
        let usage = TokenUsage::simple(100, 50);
        assert_eq!(usage.total(), 150);
    }

    #[test]
    fn is_zero_detection() {
        assert!(TokenUsage::ZERO.is_zero());
        assert!(!TokenUsage::simple(1, 0).is_zero());
        assert!(!TokenUsage::simple(0, 1).is_zero());
    }

    #[test]
    fn serialization_includes_total_and_cache() {
        let usage = TokenUsage::new(100, 50, 30, 10);
        let json = serde_json::to_string(&usage).unwrap();
        assert!(json.contains("\"promptTokens\":100"));
        assert!(json.contains("\"completionTokens\":50"));
        assert!(json.contains("\"cacheReadInputTokens\":30"));
        assert!(json.contains("\"cacheCreationInputTokens\":10"));
        assert!(json.contains("\"totalTokens\":150"));
    }

    #[test]
    fn deserialization_ignores_total() {
        let json = r#"{"promptTokens":100,"completionTokens":50,"cacheReadInputTokens":20,"cacheCreationInputTokens":5,"totalTokens":999}"#;
        let usage: TokenUsage = serde_json::from_str(json).unwrap();
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.completion_tokens, 50);
        assert_eq!(usage.cache_read_input_tokens, 20);
        assert_eq!(usage.cache_creation_input_tokens, 5);
        assert_eq!(usage.total(), 150);
    }

    #[test]
    fn deserialization_without_cache_defaults_to_zero() {
        let json = r#"{"promptTokens":100,"completionTokens":50}"#;
        let usage: TokenUsage = serde_json::from_str(json).unwrap();
        assert_eq!(usage.total(), 150);
        assert_eq!(usage.cache_read_input_tokens, 0);
        assert_eq!(usage.cache_creation_input_tokens, 0);
    }

    #[test]
    fn cache_tokens_combine() {
        let a = TokenUsage::new(100, 50, 30, 10);
        let b = TokenUsage::new(200, 100, 70, 20);
        let combined = a.combine(&b);
        assert_eq!(combined.cache_read_input_tokens, 100);
        assert_eq!(combined.cache_creation_input_tokens, 30);
        assert_eq!(combined.total(), 450);
    }

    #[test]
    fn fold_usage_aggregates() {
        let usages = vec![
            TokenUsage::simple(100, 50),
            TokenUsage::simple(200, 100),
            TokenUsage::simple(50, 25),
        ];
        let total = fold_usage(&usages);
        assert_eq!(total.prompt_tokens, 350);
        assert_eq!(total.completion_tokens, 175);
    }

    #[test]
    fn fold_empty_returns_zero() {
        let usages: Vec<TokenUsage> = vec![];
        let total = fold_usage(&usages);
        assert!(total.is_zero());
    }

    //=========================================================================
    // Property-Based Tests (Hegel)
    //=========================================================================

    use hegel::generators;

    fn draw_usage(tc: &hegel::TestCase) -> TokenUsage {
        TokenUsage::new(
            tc.draw(generators::integers::<u64>()),
            tc.draw(generators::integers::<u64>()),
            tc.draw(generators::integers::<u64>()),
            tc.draw(generators::integers::<u64>()),
        )
    }

    // --- Monoid Laws ---

    #[hegel::test]
    fn law_identity_left(tc: hegel::TestCase) {
        let a = draw_usage(&tc);
        assert_eq!(TokenUsage::ZERO.combine(&a), a);
    }

    #[hegel::test]
    fn law_identity_right(tc: hegel::TestCase) {
        let a = draw_usage(&tc);
        assert_eq!(a.combine(&TokenUsage::ZERO), a);
    }

    #[hegel::test]
    fn law_associativity(tc: hegel::TestCase) {
        let a = draw_usage(&tc);
        let b = draw_usage(&tc);
        let c = draw_usage(&tc);
        assert_eq!(a.combine(&b).combine(&c), a.combine(&b.combine(&c)));
    }

    #[hegel::test]
    fn law_commutativity(tc: hegel::TestCase) {
        let a = draw_usage(&tc);
        let b = draw_usage(&tc);
        assert_eq!(a.combine(&b), b.combine(&a));
    }

    // --- Derived value consistency ---

    #[hegel::test]
    fn total_is_prompt_plus_completion(tc: hegel::TestCase) {
        let a = draw_usage(&tc);
        assert_eq!(
            a.total(),
            a.prompt_tokens.saturating_add(a.completion_tokens)
        );
    }

    // --- Serde roundtrip ---

    #[hegel::test]
    fn roundtrip_serialization(tc: hegel::TestCase) {
        let a = draw_usage(&tc);
        let json = serde_json::to_string(&a).unwrap();
        let b: TokenUsage = serde_json::from_str(&json).unwrap();
        assert_eq!(a, b);
    }

    // --- Trait / inherent coherence ---
    // The Semigroup trait impl and the inherent combine method must agree.

    #[hegel::test]
    fn semigroup_trait_agrees_with_inherent(tc: hegel::TestCase) {
        let a = draw_usage(&tc);
        let b = draw_usage(&tc);
        let via_inherent = a.combine(&b);
        let via_trait = Semigroup::combine(&a, &b);
        assert_eq!(via_inherent, via_trait);
    }

    // --- fold homomorphism ---
    // fold_usage over a list must equal sequential left-combining.

    #[hegel::test]
    fn fold_is_left_combine(tc: hegel::TestCase) {
        let n = tc.draw(generators::integers::<usize>().max_value(20));
        let usages: Vec<TokenUsage> = (0..n).map(|_| draw_usage(&tc)).collect();
        let folded = fold_usage(&usages);
        let manual = usages
            .iter()
            .fold(TokenUsage::ZERO, |acc, u| acc.combine(u));
        assert_eq!(folded, manual);
    }

    // --- is_zero is decidable identity ---

    #[hegel::test]
    fn is_zero_iff_equal_to_zero(tc: hegel::TestCase) {
        let a = draw_usage(&tc);
        assert_eq!(a.is_zero(), a == TokenUsage::ZERO);
    }

    // --- Saturation: combine never wraps ---
    // At the boundary, combining MAX with any non-zero stays at MAX.

    #[hegel::test]
    fn combine_saturates_at_max(tc: hegel::TestCase) {
        let max = TokenUsage::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX);
        let other = draw_usage(&tc);
        let result = max.combine(&other);
        assert_eq!(result.prompt_tokens, u64::MAX);
        assert_eq!(result.completion_tokens, u64::MAX);
        assert_eq!(result.cache_read_input_tokens, u64::MAX);
        assert_eq!(result.cache_creation_input_tokens, u64::MAX);
    }
}
