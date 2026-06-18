//! Cost estimation for eval runs — cache-aware pricing models.
//!
//! # Design
//!
//! Cost estimation is a pure function from token usage + model pricing → USD amount.
//! No IO, no side effects. The `CostEstimate` type is a commutative monoid
//! (identity + associative + commutative combine).
//!
//! # Invariant
//!
//! All four fields are **finite non-negative f64**. This is enforced by:
//! - `from_usage`: inputs are `u64` tokens and non-negative pricing rates →
//!   all intermediate products are finite non-negative (u64 → f64 is lossless
//!   for values < 2^53, and pricing rates are small constants).
//! - `combine`: finite + finite = finite (no overflow for practical USD amounts).
//! - `ZERO`: all fields are 0.0 (trivially finite non-negative).
//!
//! # Laws
//!
//! - **L1 (Monoid identity)**: `CostEstimate::ZERO.combine(x) == x`
//! - **L2 (Associativity)**: `(a.combine(b)).combine(c) == a.combine(b.combine(c))`
//! - **L3 (Commutativity)**: `a.combine(b) == b.combine(a)`
//! - **L4 (Non-negative)**: `estimate.total_usd >= 0.0`
//! - **L5 (Finite)**: all fields are finite (not NaN, not Infinity)

use crate::types::TokenUsageSummary;
pub use agent_fw_core::ModelPricing;
use agent_fw_core::{estimate_cost, CacheTokens, TokenUsage};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Resolve pricing for a model at runtime.
///
/// The framework deliberately keeps pricing as injected application data.
/// Hardcoded model tables are useful for tests and demos, but production code
/// should wire a resolver from the outside so model costs can be refreshed
/// independently of framework releases.
pub trait ModelPricingResolver: Send + Sync {
    fn resolve_model_pricing(&self, model_id: &str) -> Option<ModelPricing>;
}

/// Exact-match pricing resolver backed by externally supplied data.
#[derive(Debug, Clone, Default)]
pub struct StaticPricingResolver {
    entries: BTreeMap<String, ModelPricing>,
}

impl StaticPricingResolver {
    pub fn new(entries: impl IntoIterator<Item = ModelPricing>) -> Self {
        let mut resolver = Self::default();
        for pricing in entries {
            resolver.insert(pricing);
        }
        resolver
    }

    pub fn insert(&mut self, pricing: ModelPricing) {
        self.entries.insert(pricing.model.clone(), pricing);
    }
}

impl ModelPricingResolver for StaticPricingResolver {
    fn resolve_model_pricing(&self, model_id: &str) -> Option<ModelPricing> {
        self.entries.get(model_id).cloned()
    }
}

/// Cost estimate (commutative monoid).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostEstimate {
    pub input_cost_usd: f64,
    pub output_cost_usd: f64,
    pub cached_cost_usd: f64,
    pub total_usd: f64,
}

impl CostEstimate {
    /// Monoid identity.
    pub const ZERO: Self = Self {
        input_cost_usd: 0.0,
        output_cost_usd: 0.0,
        cached_cost_usd: 0.0,
        total_usd: 0.0,
    };

    /// Check that all fields are finite non-negative (L4 + L5).
    pub fn is_finite(&self) -> bool {
        [
            self.input_cost_usd,
            self.output_cost_usd,
            self.cached_cost_usd,
            self.total_usd,
        ]
        .iter()
        .all(|&v| v.is_finite() && v >= 0.0)
    }

    /// Compute cost from token usage and pricing.
    ///
    /// # Panics (debug only)
    ///
    /// Debug-asserts that the result is finite. This cannot fail when
    /// pricing rates are finite non-negative (the built-in rates satisfy
    /// this) and token counts are `u64` (always finite when cast to `f64`).
    pub fn from_usage(usage: &TokenUsageSummary, pricing: &ModelPricing) -> Self {
        debug_assert!(
            pricing.is_valid(),
            "ModelPricing rates must be finite non-negative"
        );
        let core_usage = TokenUsage::new(
            usage.input_tokens(),
            usage.output_tokens(),
            usage.cached_tokens(),
            usage.cache_creation_tokens(),
        );
        let cache = CacheTokens {
            read: usage.cached_tokens(),
            creation: usage.cache_creation_tokens(),
        };
        let uncached_input = usage.uncached_input_tokens();
        let input_cost = (uncached_input as f64 / 1_000_000.0) * pricing.input_per_million();
        let output_cost =
            (usage.output_tokens() as f64 / 1_000_000.0) * pricing.output_per_million();
        let cached_cost = (usage.cached_tokens() as f64 / 1_000_000.0)
            * pricing.cache_read_per_million()
            + (usage.cache_creation_tokens() as f64 / 1_000_000.0)
                * pricing.cache_creation_per_million();
        let total = estimate_cost(&core_usage, &cache, pricing);
        let result = Self {
            input_cost_usd: input_cost,
            output_cost_usd: output_cost,
            cached_cost_usd: cached_cost,
            total_usd: total,
        };
        debug_assert!(
            result.is_finite(),
            "CostEstimate::from_usage produced non-finite result: {result:?}"
        );
        result
    }

    /// Monoid combine (field-wise addition).
    ///
    /// Preserves the finite non-negative invariant when both operands
    /// satisfy it: `finite + finite = finite` for practical USD amounts
    /// (overflow to `Infinity` requires ~1.8e308 USD which exceeds world GDP).
    pub fn combine(&self, other: &Self) -> Self {
        let result = Self {
            input_cost_usd: self.input_cost_usd + other.input_cost_usd,
            output_cost_usd: self.output_cost_usd + other.output_cost_usd,
            cached_cost_usd: self.cached_cost_usd + other.cached_cost_usd,
            total_usd: self.total_usd + other.total_usd,
        };
        debug_assert!(
            result.is_finite(),
            "CostEstimate::combine produced non-finite result: {result:?}"
        );
        result
    }
}

/// Estimate the cost of an eval run before execution.
///
/// Pure function: takes config params, returns estimated cost.
pub fn estimate_eval_cost(
    test_case_count: u32,
    samples_per_case: u32,
    avg_input_tokens: u64,
    avg_output_tokens: u64,
    cache_hit_rate: f64,
    pricing: &ModelPricing,
) -> CostEstimate {
    let total_samples = test_case_count as u64 * samples_per_case as u64;
    let total_input = total_samples * avg_input_tokens;
    let total_output = total_samples * avg_output_tokens;
    let total_cached = (total_input as f64 * cache_hit_rate) as u64;

    let usage = TokenUsageSummary::new(total_input, total_output, total_cached, 0);
    CostEstimate::from_usage(&usage, pricing)
}

/// Estimate the cost of profiling a schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilingCostEstimate {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    #[serde(default)]
    pub cache_creation_tokens: u64,
    pub cost: CostEstimate,
}

const ENRICHMENT_PROMPT_OVERHEAD: u64 = 300;
const TOKENS_PER_COLUMN: u64 = 25;
const TOKENS_PER_SAMPLE_ROW: u64 = 120;
const SAMPLE_ROWS: u64 = 3;
const OUTPUT_BASE_TOKENS: u64 = 120;
const OUTPUT_PER_COLUMN_TOKENS: u64 = 35;

/// Estimate the cost of profiling a schema.
///
/// The estimate models one enrichment call per table with:
/// - fixed prompt overhead
/// - per-column schema tokens
/// - three sample rows per table
/// - fixed and per-column output tokens
///
/// If cache-read pricing is available, repeated prompt overhead after the first
/// table is charged at the cache-read rate.
pub fn estimate_profiling_cost(
    table_count: u32,
    total_columns: u32,
    pricing: &ModelPricing,
) -> ProfilingCostEstimate {
    if table_count == 0 {
        return ProfilingCostEstimate {
            input_tokens: 0,
            output_tokens: 0,
            cached_tokens: 0,
            cache_creation_tokens: 0,
            cost: CostEstimate::ZERO,
        };
    }

    let table_count = table_count as u64;
    let avg_columns = (total_columns as u64).checked_div(table_count).unwrap_or(0);

    let input_per_table = ENRICHMENT_PROMPT_OVERHEAD
        + avg_columns * TOKENS_PER_COLUMN
        + SAMPLE_ROWS * TOKENS_PER_SAMPLE_ROW;
    let output_per_table = OUTPUT_BASE_TOKENS + avg_columns * OUTPUT_PER_COLUMN_TOKENS;

    let input_tokens = table_count * input_per_table;
    let output_tokens = table_count * output_per_table;

    let cached_tokens = match pricing.cache_read_per_million() {
        cache_rate if cache_rate > 0.0 && table_count > 1 => {
            ENRICHMENT_PROMPT_OVERHEAD * (table_count - 1)
        }
        _ => 0,
    };

    let usage = TokenUsageSummary::new(input_tokens, output_tokens, cached_tokens, 0);

    ProfilingCostEstimate {
        input_tokens,
        output_tokens,
        cached_tokens,
        cache_creation_tokens: 0,
        cost: CostEstimate::from_usage(&usage, pricing),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sonnet_pricing() -> ModelPricing {
        ModelPricing::for_model("claude-sonnet-4-6").unwrap()
    }

    fn usage(input: u64, output: u64, cached: u64, cache_creation: u64) -> TokenUsageSummary {
        TokenUsageSummary::new(input, output, cached, cache_creation)
    }

    // =========================================================================
    // ModelPricing validation
    // =========================================================================

    #[test]
    fn builtin_pricing_is_valid() {
        for model in ["claude-opus-4-6", "claude-sonnet-4-5", "claude-haiku-4-5"] {
            let p = ModelPricing::for_model(model).unwrap();
            assert!(p.is_valid(), "built-in pricing for {model} must be valid");
        }
    }

    #[test]
    fn invalid_pricing_detected() {
        let p = ModelPricing::new("bad", f64::NAN, 1.0, 0.0);
        assert!(!p.is_valid());

        let p2 = ModelPricing::new("bad", -1.0, 1.0, 0.0);
        assert!(!p2.is_valid());
    }

    #[test]
    fn pricing_for_known_models() {
        assert!(ModelPricing::for_model("claude-opus-4-6").is_some());
        assert!(ModelPricing::for_model("claude-sonnet-4-5").is_some());
        assert!(ModelPricing::for_model("claude-haiku-4-5").is_some());
        assert!(ModelPricing::for_model("unknown-model").is_none());
    }

    #[test]
    fn static_pricing_resolver_requires_exact_external_entry() {
        let resolver =
            StaticPricingResolver::new([ModelPricing::new("custom-model", 9.0, 27.0, 1.0)]);

        let resolved = resolver.resolve_model_pricing("custom-model").unwrap();
        assert_eq!(resolved.input_per_million(), 9.0);
        assert!(resolver.resolve_model_pricing("missing-model").is_none());
    }

    // =========================================================================
    // CostEstimate invariant
    // =========================================================================

    #[test]
    fn zero_is_finite() {
        assert!(CostEstimate::ZERO.is_finite());
    }

    #[test]
    fn from_usage_is_finite() {
        let usage = usage(u64::MAX, u64::MAX, 0, 0);
        let est = CostEstimate::from_usage(&usage, &sonnet_pricing());
        assert!(est.is_finite());
    }

    // =========================================================================
    // Monoid laws (exact equality — f64 addition is deterministic)
    // =========================================================================

    #[test]
    fn cost_estimate_zero_is_identity() {
        let usage = usage(100_000, 50_000, 25_000, 0);
        let est = CostEstimate::from_usage(&usage, &sonnet_pricing());
        assert_eq!(CostEstimate::ZERO.combine(&est), est, "L1: left identity");
        assert_eq!(est.combine(&CostEstimate::ZERO), est, "L1: right identity");
    }

    /// Helper: assert two CostEstimates are equal within f64 tolerance.
    ///
    /// f64 addition is NOT associative (IEEE 754 rounding), so monoid law
    /// tests must use epsilon comparison. This is an inherent limitation of
    /// f64 — the monoid laws hold "up to floating-point precision."
    fn assert_cost_approx_eq(a: &CostEstimate, b: &CostEstimate, label: &str) {
        let eps = 1e-10;
        assert!(
            (a.input_cost_usd - b.input_cost_usd).abs() < eps,
            "{label}: input_cost_usd {:.15} vs {:.15}",
            a.input_cost_usd,
            b.input_cost_usd
        );
        assert!(
            (a.output_cost_usd - b.output_cost_usd).abs() < eps,
            "{label}: output_cost_usd"
        );
        assert!(
            (a.cached_cost_usd - b.cached_cost_usd).abs() < eps,
            "{label}: cached_cost_usd"
        );
        assert!(
            (a.total_usd - b.total_usd).abs() < eps,
            "{label}: total_usd {:.15} vs {:.15}",
            a.total_usd,
            b.total_usd
        );
    }

    #[test]
    fn cost_estimate_associativity() {
        let pricing = sonnet_pricing();
        let a = CostEstimate::from_usage(&usage(100, 50, 10, 0), &pricing);
        let b = CostEstimate::from_usage(&usage(200, 100, 20, 0), &pricing);
        let c = CostEstimate::from_usage(&usage(300, 150, 30, 0), &pricing);
        assert_cost_approx_eq(
            &a.combine(&b).combine(&c),
            &a.combine(&b.combine(&c)),
            "L2: associativity",
        );
    }

    #[test]
    fn cost_estimate_commutativity() {
        let pricing = sonnet_pricing();
        let a = CostEstimate::from_usage(&usage(100_000, 50_000, 10_000, 0), &pricing);
        let b = CostEstimate::from_usage(&usage(200_000, 100_000, 20_000, 0), &pricing);
        assert_cost_approx_eq(&a.combine(&b), &b.combine(&a), "L3: commutativity");
    }

    #[test]
    fn cost_estimate_non_negative() {
        let pricing = sonnet_pricing();
        let est = CostEstimate::from_usage(&usage(0, 0, 0, 0), &pricing);
        assert!(est.total_usd >= 0.0, "L4: non-negative");
        assert!(est.is_finite(), "L5: finite");
    }

    #[test]
    fn cache_creation_tokens_are_costed_separately() {
        let pricing = sonnet_pricing();
        let est = CostEstimate::from_usage(&usage(10_000, 2_000, 1_000, 2_500), &pricing);
        assert!(est.cached_cost_usd > 0.0);
        assert!(est.input_cost_usd > 0.0);
    }

    // =========================================================================
    // Estimation functions
    // =========================================================================

    #[test]
    fn estimate_eval_cost_reasonable() {
        let pricing = sonnet_pricing();
        let est = estimate_eval_cost(10, 3, 5000, 2000, 0.5, &pricing);
        assert!(est.total_usd > 0.0);
        assert!(est.total_usd < 10.0); // Should be well under $10 for 30 samples
        assert!(est.is_finite());
    }

    #[test]
    fn estimate_profiling_cost_reasonable() {
        let pricing = sonnet_pricing();
        let est = estimate_profiling_cost(20, 15, &pricing);
        assert!(est.cost.total_usd > 0.0);
        assert!(est.cost.total_usd < 1.0); // Should be well under $1 for 20 tables
        assert!(est.cost.is_finite());
    }

    #[test]
    fn estimate_profiling_cost_zero_tables() {
        let est = estimate_profiling_cost(0, 0, &sonnet_pricing());
        assert_eq!(est.input_tokens, 0);
        assert_eq!(est.output_tokens, 0);
        assert_eq!(est.cached_tokens, 0);
        assert_eq!(est.cost, CostEstimate::ZERO);
    }

    #[test]
    fn estimate_profiling_cost_uses_cache_read_rate_for_repeated_overhead() {
        let pricing = ModelPricing::new("custom-model", 5.0, 25.0, 0.5);
        let est = estimate_profiling_cost(10, 100, &pricing);
        assert_eq!(est.cached_tokens, ENRICHMENT_PROMPT_OVERHEAD * 9);
        assert!(est.cost.total_usd > 0.0);
    }

    // =========================================================================
    // Hegel: monoid laws with arbitrary token counts
    // =========================================================================

    use hegel::generators;

    fn draw_token_quad(tc: &hegel::TestCase, max: u64) -> (u64, u64, u64, u64) {
        let input = tc.draw(generators::integers::<u64>().min_value(0).max_value(max));
        let output = tc.draw(generators::integers::<u64>().min_value(0).max_value(max));
        let cached_raw = tc.draw(generators::integers::<u64>().min_value(0).max_value(max));
        let cache_creation_raw = tc.draw(generators::integers::<u64>().min_value(0).max_value(max));
        let cached = cached_raw.min(input);
        let cache_creation = cache_creation_raw.min(input.saturating_sub(cached));
        (input, output, cached, cache_creation)
    }

    /// L1 (Identity) with arbitrary token counts.
    #[hegel::test]
    fn law_identity(tc: hegel::TestCase) {
        let (input, output, cached, cache_creation) = draw_token_quad(&tc, 10_000_000);
        let pricing = sonnet_pricing();
        let usage = usage(input, output, cached, cache_creation);
        let est = CostEstimate::from_usage(&usage, &pricing);
        assert_eq!(
            CostEstimate::ZERO.combine(&est),
            est.clone(),
            "left identity"
        );
        assert_eq!(est.combine(&CostEstimate::ZERO), est, "right identity");
    }

    /// L2 (Associativity) with arbitrary token counts.
    ///
    /// Uses epsilon comparison because f64 addition is not associative
    /// under IEEE 754 — `(a+b)+c` and `a+(b+c)` may differ by 1 ULP.
    #[hegel::test]
    fn law_associativity(tc: hegel::TestCase) {
        let pricing = sonnet_pricing();
        let (ai, ao, ac, acw) = draw_token_quad(&tc, 1_000_000);
        let (bi, bo, bc, bcw) = draw_token_quad(&tc, 1_000_000);
        let (ci, co, cc, ccw) = draw_token_quad(&tc, 1_000_000);
        let a = CostEstimate::from_usage(&usage(ai, ao, ac, acw), &pricing);
        let b = CostEstimate::from_usage(&usage(bi, bo, bc, bcw), &pricing);
        let c = CostEstimate::from_usage(&usage(ci, co, cc, ccw), &pricing);
        let ab_c = a.combine(&b).combine(&c);
        let a_bc = a.combine(&b.combine(&c));
        assert!(
            (ab_c.total_usd - a_bc.total_usd).abs() < 1e-10,
            "associativity: {} vs {}",
            ab_c.total_usd,
            a_bc.total_usd
        );
    }

    /// L3 (Commutativity) with arbitrary token counts.
    ///
    /// Exact equality: `a + b == b + a` IS guaranteed by IEEE 754
    /// (addition is commutative even with rounding).
    #[hegel::test]
    fn law_commutativity(tc: hegel::TestCase) {
        let pricing = sonnet_pricing();
        let (ai, ao, ac, acw) = draw_token_quad(&tc, 10_000_000);
        let (bi, bo, bc, bcw) = draw_token_quad(&tc, 10_000_000);
        let a = CostEstimate::from_usage(&usage(ai, ao, ac, acw), &pricing);
        let b = CostEstimate::from_usage(&usage(bi, bo, bc, bcw), &pricing);
        assert_eq!(a.combine(&b), b.combine(&a), "commutativity");
    }

    /// L4 + L5 (Non-negative + Finite) with arbitrary token counts.
    #[hegel::test]
    fn law_non_negative_finite(tc: hegel::TestCase) {
        let (input, output, cached, cache_creation) = draw_token_quad(&tc, u64::MAX);
        let pricing = sonnet_pricing();
        let usage = usage(input, output, cached, cache_creation);
        let est = CostEstimate::from_usage(&usage, &pricing);
        assert!(est.total_usd >= 0.0, "L4: non-negative");
        assert!(est.is_finite(), "L5: finite");
    }
}
