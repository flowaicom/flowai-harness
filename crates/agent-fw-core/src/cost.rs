//! Model pricing and cost estimation.
//!
//! Provides per-model pricing rates and a pure function to compute
//! estimated cost from token usage. Supports prompt caching (both
//! cache reads and cache creation).
//!
//! # Design
//!
//! Rate cards (`RateCard`) are pure data — rates without identity.
//! `ModelPricing` pairs a rate card with a model identifier.
//!
//! Cost estimation is a total function:
//! `estimate_cost(usage, cache_tokens, pricing) → f64`.
//!
//! # Pricing Formula
//!
//! ```text
//! cost = (input_tokens - cache_read_tokens - cache_creation_tokens) × input_rate
//!      + completion_tokens × output_rate
//!      + cache_read_tokens × cache_read_rate
//!      + cache_creation_tokens × cache_creation_rate
//! ```

use serde::{Deserialize, Serialize};

use crate::usage::TokenUsage;

/// Rate card — pricing rates without model identity.
///
/// This is the `const`-safe type used for well-known pricing tables.
/// All rates are USD per million tokens.
///
/// # Known Limitation (#13)
///
/// Rates use `f64`, which has inherent precision issues for financial
/// arithmetic (e.g. `0.1 + 0.2 ≠ 0.3`). For cost *estimation* this is
/// acceptable — we're computing approximate LLM costs, not billing ledger
/// entries. If exact accounting is ever needed, rates should migrate to
/// `rust_decimal::Decimal` or integer micro-dollars (`u64` representing
/// USD × 10⁶). The `const` rate card tables would need to use a
/// `Decimal::from_parts` constructor since `Decimal` is not `const`-safe.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateCard {
    /// Cost per million input tokens (USD).
    pub input_per_million: f64,
    /// Cost per million output tokens (USD).
    pub output_per_million: f64,
    /// Cost per million cache-read tokens (USD).
    pub cache_read_per_million: f64,
    /// Cost per million cache-creation tokens (USD).
    /// Typically more expensive than input_per_million (Anthropic: 1.25× input).
    pub cache_creation_per_million: f64,
}

/// Per-model pricing: a rate card paired with a model identifier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPricing {
    /// Model identifier (e.g., "claude-sonnet-4-5-20250514").
    pub model: String,
    /// The rate card for this model.
    #[serde(flatten)]
    pub rates: RateCard,
}

impl ModelPricing {
    /// Create a new pricing entry.
    pub fn new(
        model: impl Into<String>,
        input_per_million: f64,
        output_per_million: f64,
        cache_read_per_million: f64,
    ) -> Self {
        Self {
            model: model.into(),
            rates: RateCard {
                input_per_million,
                output_per_million,
                cache_read_per_million,
                cache_creation_per_million: input_per_million * 1.25,
            },
        }
    }

    /// Create with explicit cache creation rate.
    pub fn with_cache_creation(
        model: impl Into<String>,
        input_per_million: f64,
        output_per_million: f64,
        cache_read_per_million: f64,
        cache_creation_per_million: f64,
    ) -> Self {
        Self {
            model: model.into(),
            rates: RateCard {
                input_per_million,
                output_per_million,
                cache_read_per_million,
                cache_creation_per_million,
            },
        }
    }

    /// Convenience: access input rate.
    pub fn input_per_million(&self) -> f64 {
        self.rates.input_per_million
    }
    /// Convenience: access output rate.
    pub fn output_per_million(&self) -> f64 {
        self.rates.output_per_million
    }
    /// Convenience: access cache-read rate.
    pub fn cache_read_per_million(&self) -> f64 {
        self.rates.cache_read_per_million
    }
    /// Convenience: access cache-creation rate.
    pub fn cache_creation_per_million(&self) -> f64 {
        self.rates.cache_creation_per_million
    }

    /// Check that all pricing rates are finite and non-negative.
    pub fn is_valid(&self) -> bool {
        [
            self.rates.input_per_million,
            self.rates.output_per_million,
            self.rates.cache_read_per_million,
            self.rates.cache_creation_per_million,
        ]
        .iter()
        .all(|&r| r.is_finite() && r >= 0.0)
    }

    /// Built-in Sonnet family pricing for tests and demos.
    pub fn sonnet_4() -> Self {
        Self {
            model: "claude-sonnet-4".to_string(),
            rates: SONNET_4_RATES,
        }
    }

    /// Built-in Opus family pricing for tests and demos.
    pub fn opus_4() -> Self {
        Self {
            model: "claude-opus-4".to_string(),
            rates: OPUS_4_RATES,
        }
    }

    /// Built-in Haiku family pricing for tests and demos.
    pub fn haiku_4() -> Self {
        Self {
            model: "claude-haiku-4".to_string(),
            rates: HAIKU_4_RATES,
        }
    }

    /// Look up built-in pricing for a known model family.
    ///
    /// This is a convenience wrapper around [`try_pricing_for_model`].
    pub fn for_model(model: &str) -> Option<Self> {
        try_pricing_for_model(model)
    }
}

// ============================================================================
// Well-known rate cards (as of 2026-02)
// ============================================================================

/// Claude Sonnet 4/4.5/4.6 rate card.
pub const SONNET_4_RATES: RateCard = RateCard {
    input_per_million: 3.0,
    output_per_million: 15.0,
    cache_read_per_million: 0.30,
    cache_creation_per_million: 3.75, // 1.25× input
};

/// Claude Opus 4.5/4.6 rate card. Opus 4/4.1 was $15/$75/$1.50/$18.75.
pub const OPUS_4_RATES: RateCard = RateCard {
    input_per_million: 5.0,
    output_per_million: 25.0,
    cache_read_per_million: 0.50,
    cache_creation_per_million: 6.25, // 1.25× input
};

/// Claude Haiku 4.5 rate card. Haiku 3.5 was $0.80/$4.00/$0.08/$1.00.
pub const HAIKU_4_RATES: RateCard = RateCard {
    input_per_million: 1.0,
    output_per_million: 5.0,
    cache_read_per_million: 0.10,
    cache_creation_per_million: 1.25, // 1.25× input
};

/// Discriminated union of known model families.
///
/// Provides structured matching instead of fragile `contains()` heuristics.
/// Each variant maps to a well-known `RateCard`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModelFamily {
    /// Claude Opus 4 family (e.g. "claude-opus-4-20250514").
    Opus4,
    /// Claude Sonnet 4/4.5 family (e.g. "claude-sonnet-4-5-20250514").
    Sonnet4,
    /// Claude Haiku 4 family (e.g. "claude-haiku-4-5-20251001").
    Haiku4,
}

impl ModelFamily {
    /// Parse a model ID string into a known family.
    ///
    /// Uses prefix-based matching on the canonical `claude-{family}-` pattern
    /// rather than substring `contains()`, avoiding false positives on
    /// arbitrary model strings like "myopus-custom".
    ///
    /// Zero-allocation: uses `eq_ignore_ascii_case` on prefix slices instead
    /// of allocating a lowercased copy of the entire model string.
    ///
    /// Returns `None` for non-Claude or unrecognized models. Legacy `claude-3-*`
    /// models are not matched; callers should supply their own pricing data
    /// when the built-in family table is insufficient.
    pub fn from_model_id(model: &str) -> Option<Self> {
        let bytes = model.as_bytes();
        if bytes.len() >= 11 && bytes[..11].eq_ignore_ascii_case(b"claude-opus") {
            Some(Self::Opus4)
        } else if bytes.len() >= 13 && bytes[..13].eq_ignore_ascii_case(b"claude-sonnet") {
            Some(Self::Sonnet4)
        } else if bytes.len() >= 12 && bytes[..12].eq_ignore_ascii_case(b"claude-haiku") {
            Some(Self::Haiku4)
        } else {
            None
        }
    }

    /// The rate card for this model family.
    ///
    /// Total function — every variant has a well-defined rate card.
    pub fn rate_card(self) -> &'static RateCard {
        match self {
            Self::Opus4 => &OPUS_4_RATES,
            Self::Sonnet4 => &SONNET_4_RATES,
            Self::Haiku4 => &HAIKU_4_RATES,
        }
    }
}

impl std::fmt::Display for ModelFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Opus4 => write!(f, "opus-4"),
            Self::Sonnet4 => write!(f, "sonnet-4"),
            Self::Haiku4 => write!(f, "haiku-4"),
        }
    }
}

/// Look up pricing for a model string.
///
/// Returns `Some` if the model matches a known family, `None` otherwise.
/// Callers must decide their own fallback policy.
pub fn try_pricing_for_model(model: &str) -> Option<ModelPricing> {
    let family = ModelFamily::from_model_id(model)?;
    Some(ModelPricing {
        model: model.to_string(),
        rates: *family.rate_card(),
    })
}

/// Cache token counts for cost estimation.
#[derive(Debug, Clone, Copy, Default)]
pub struct CacheTokens {
    /// Tokens served from cache (cheaper).
    pub read: u64,
    /// Tokens written to cache (more expensive).
    pub creation: u64,
}

impl CacheTokens {
    pub fn read_only(read: u64) -> Self {
        Self { read, creation: 0 }
    }
}

/// Estimate the cost of a token usage given model pricing and cache info.
///
/// # Arguments
///
/// * `usage` — Token counts (prompt, completion).
/// * `cache` — Cache read/creation token counts.
/// * `pricing` — Per-model rate card.
///
/// # Returns
///
/// Estimated cost in USD.
pub fn estimate_cost(usage: &TokenUsage, cache: &CacheTokens, pricing: &ModelPricing) -> f64 {
    let r = &pricing.rates;
    let cached_total = cache.read.saturating_add(cache.creation);
    let uncached_input = usage.prompt_tokens.saturating_sub(cached_total);
    let input_cost = (uncached_input as f64) * r.input_per_million / 1_000_000.0;
    let output_cost = (usage.completion_tokens as f64) * r.output_per_million / 1_000_000.0;
    let cache_read_cost = (cache.read as f64) * r.cache_read_per_million / 1_000_000.0;
    let cache_create_cost = (cache.creation as f64) * r.cache_creation_per_million / 1_000_000.0;
    input_cost + output_cost + cache_read_cost + cache_create_cost
}

/// Estimate cost with no caching (all tokens are uncached input).
pub fn estimate_cost_simple(usage: &TokenUsage, pricing: &ModelPricing) -> f64 {
    estimate_cost(usage, &CacheTokens::default(), pricing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_cost_basic() {
        let usage = TokenUsage::simple(1_000_000, 500_000);
        let pricing = ModelPricing::new("test-model", 3.0, 15.0, 0.30);
        let cost = estimate_cost_simple(&usage, &pricing);
        // 1M input × $3/M + 500K output × $15/M = $3 + $7.5 = $10.5
        assert!((cost - 10.5).abs() < 0.001);
    }

    #[test]
    fn estimate_cost_with_cache_read() {
        let usage = TokenUsage::simple(1_000_000, 500_000);
        let cache = CacheTokens::read_only(800_000);
        let pricing = ModelPricing::new("test-model", 3.0, 15.0, 0.30);
        let cost = estimate_cost(&usage, &cache, &pricing);
        // Uncached: 200K × $3/M = $0.6
        // Cached read: 800K × $0.30/M = $0.24
        // Output: 500K × $15/M = $7.5
        // Total: $8.34
        assert!((cost - 8.34).abs() < 0.001);
    }

    #[test]
    fn estimate_cost_with_cache_creation() {
        let usage = TokenUsage::simple(1_000_000, 100_000);
        let cache = CacheTokens {
            read: 500_000,
            creation: 200_000,
        };
        let pricing = ModelPricing::with_cache_creation("test", 3.0, 15.0, 0.30, 3.75);
        let cost = estimate_cost(&usage, &cache, &pricing);
        // Uncached: (1M - 500K - 200K) = 300K × $3/M = $0.9
        // Cache read: 500K × $0.30/M = $0.15
        // Cache create: 200K × $3.75/M = $0.75
        // Output: 100K × $15/M = $1.5
        // Total: $3.3
        assert!((cost - 3.3).abs() < 0.001);
    }

    #[test]
    fn estimate_cost_zero_usage() {
        let usage = TokenUsage::ZERO;
        let pricing = try_pricing_for_model("claude-sonnet-4-5-20250514").unwrap();
        let cost = estimate_cost_simple(&usage, &pricing);
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn pricing_lookup_sonnet() {
        let p = try_pricing_for_model("claude-sonnet-4-5-20250514").unwrap();
        assert_eq!(p.rates.input_per_million, 3.0);
        assert_eq!(p.rates.output_per_million, 15.0);
    }

    #[test]
    fn pricing_lookup_opus() {
        let p = try_pricing_for_model("claude-opus-4-6-20250514").unwrap();
        assert_eq!(p.rates.input_per_million, 5.0);
        assert_eq!(p.rates.output_per_million, 25.0);
    }

    #[test]
    fn pricing_lookup_haiku() {
        let p = try_pricing_for_model("claude-haiku-4-5-20251001").unwrap();
        assert_eq!(p.rates.input_per_million, 1.0);
        assert_eq!(p.rates.output_per_million, 5.0);
    }

    #[test]
    fn try_pricing_returns_none_for_unknown() {
        assert!(try_pricing_for_model("gpt-4o-mini").is_none());
    }

    #[test]
    fn model_pricing_serializes() {
        let p = try_pricing_for_model("claude-sonnet-4-5-20250514").unwrap();
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"inputPerMillion\":3.0"));
        assert!(json.contains("\"cacheCreationPerMillion\":3.75"));
    }

    #[test]
    fn rate_card_is_copy() {
        let r = SONNET_4_RATES;
        let r2 = r; // Copy, not move
        assert_eq!(r.input_per_million, r2.input_per_million);
    }

    // --- ModelFamily ---

    #[test]
    fn model_family_parses_opus() {
        assert_eq!(
            ModelFamily::from_model_id("claude-opus-4-20250514"),
            Some(ModelFamily::Opus4),
        );
    }

    #[test]
    fn model_family_parses_sonnet() {
        assert_eq!(
            ModelFamily::from_model_id("claude-sonnet-4-5-20250514"),
            Some(ModelFamily::Sonnet4),
        );
    }

    #[test]
    fn model_family_parses_haiku() {
        assert_eq!(
            ModelFamily::from_model_id("claude-haiku-4-5-20251001"),
            Some(ModelFamily::Haiku4),
        );
    }

    #[test]
    fn model_family_rejects_non_claude() {
        assert_eq!(ModelFamily::from_model_id("gpt-4o-mini"), None);
    }

    #[test]
    fn model_family_rejects_substring_false_positive() {
        // "myopus-model" should NOT match — prefix-based matching prevents this
        assert_eq!(ModelFamily::from_model_id("myopus-model"), None);
        assert_eq!(ModelFamily::from_model_id("notclaude-sonnet-4"), None);
    }

    #[test]
    fn model_family_rejects_legacy_claude3() {
        // Legacy claude-3-* models have different pricing per sub-family
        // (opus=$15/$75, sonnet=$3/$15, haiku=$0.25/$1.25). Rather than
        // silently misclassifying, return None so callers use explicit fallback.
        assert_eq!(ModelFamily::from_model_id("claude-3-opus-20240229"), None);
        assert_eq!(ModelFamily::from_model_id("claude-3-sonnet-20240229"), None);
        assert_eq!(ModelFamily::from_model_id("claude-3-haiku-20240307"), None);
        assert_eq!(
            ModelFamily::from_model_id("claude-3-5-sonnet-20241022"),
            None
        );
        assert_eq!(
            ModelFamily::from_model_id("claude-3-5-haiku-20241022"),
            None
        );
    }

    #[test]
    fn model_family_rate_card_is_total() {
        // Every variant must produce a valid rate card (total function)
        assert_eq!(ModelFamily::Opus4.rate_card(), &OPUS_4_RATES);
        assert_eq!(ModelFamily::Sonnet4.rate_card(), &SONNET_4_RATES);
        assert_eq!(ModelFamily::Haiku4.rate_card(), &HAIKU_4_RATES);
    }

    #[test]
    fn model_family_display() {
        assert_eq!(ModelFamily::Opus4.to_string(), "opus-4");
        assert_eq!(ModelFamily::Sonnet4.to_string(), "sonnet-4");
        assert_eq!(ModelFamily::Haiku4.to_string(), "haiku-4");
    }

    #[test]
    fn model_family_case_insensitive() {
        assert_eq!(
            ModelFamily::from_model_id("Claude-Opus-4-20250514"),
            Some(ModelFamily::Opus4),
        );
        assert_eq!(
            ModelFamily::from_model_id("CLAUDE-SONNET-4-5"),
            Some(ModelFamily::Sonnet4),
        );
    }
}
