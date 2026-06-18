//! Sweep axis system and metric algebra.
//!
//! Provides the infrastructure for parameter sweeps: varying action parameters
//! across a range and measuring the impact via domain-specific metrics.
//!
//! # MetricPoint — Commutative Monoid
//!
//! `MetricPoint` records the baseline and modified values of a metric,
//! along with derived delta and percent change. It forms a commutative
//! monoid under `combine`:
//!
//! - **Identity**: `MetricPoint::ZERO` — all fields zero
//! - **Associativity**: `(a.combine(b)).combine(c) == a.combine(b.combine(c))`
//! - **Commutativity**: `a.combine(b) == b.combine(a)`
//!
//! The `combine` operation is additive on `baseline` and `modified`,
//! then recomputes `delta` and `percent_change` from the sums.
//!
//! # SweepRange — Validated Numeric Parameters
//!
//! Enforces:
//! - `from <= to`
//! - `step > 0`
//! - `point_count() <= MAX_SWEEP_POINTS`

use rust_decimal::prelude::*;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Maximum number of sweep points to prevent runaway computation.
pub const MAX_SWEEP_POINTS: usize = 100;

// ─── MetricPoint ──────────────────────────────────────────────────────

/// Impact of actions on a metric, forming a commutative monoid.
///
/// # Derived Fields
///
/// `delta` and `percent_change` are always consistent with `baseline`
/// and `modified`:
/// - `delta = modified - baseline`
/// - `percent_change = if baseline == 0 { 0 } else { delta / baseline * 100 }`
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricPoint {
    #[serde(skip_serializing)]
    pub baseline: Decimal,
    #[serde(skip_serializing)]
    pub modified: Decimal,
    #[serde(skip_serializing)]
    pub delta: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub percent_change: Decimal,
}

impl MetricPoint {
    /// Monoid identity element.
    pub const ZERO: Self = Self {
        baseline: Decimal::ZERO,
        modified: Decimal::ZERO,
        delta: Decimal::ZERO,
        percent_change: Decimal::ZERO,
    };

    /// Construct from baseline and modified values.
    /// Derives delta and percent_change automatically.
    pub fn from_baseline_modified(baseline: Decimal, modified: Decimal) -> Self {
        let delta = modified - baseline;
        let percent_change = if baseline.is_zero() {
            Decimal::ZERO
        } else {
            (delta / baseline) * Decimal::ONE_HUNDRED
        };
        Self {
            baseline,
            modified,
            delta,
            percent_change,
        }
    }

    /// Monoid combine: additive on baseline and modified.
    pub fn combine(&self, other: &Self) -> Self {
        Self::from_baseline_modified(
            self.baseline + other.baseline,
            self.modified + other.modified,
        )
    }
}

impl PartialEq for MetricPoint {
    fn eq(&self, other: &Self) -> bool {
        self.baseline == other.baseline
            && self.modified == other.modified
            && self.delta == other.delta
            && self.percent_change == other.percent_change
    }
}

impl Eq for MetricPoint {}

// ─── SweepRange ───────────────────────────────────────────────────────

/// Validated numeric sweep range.
///
/// Represents a discrete set of points: `from, from+step, from+2*step, ..., ≤ to`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SweepRange {
    #[serde(with = "rust_decimal::serde::str")]
    pub from: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub to: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub step: Decimal,
}

/// Error from sweep range validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SweepRangeError {
    pub axis_label: String,
    pub message: String,
}

impl fmt::Display for SweepRangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sweep axis '{}': {}", self.axis_label, self.message)
    }
}

impl std::error::Error for SweepRangeError {}

impl SweepRange {
    /// Number of discrete points in the range.
    pub fn point_count(&self) -> usize {
        if self.step.is_zero() || self.from > self.to {
            return 0;
        }
        let span = self.to - self.from;
        let count = (span / self.step).floor() + Decimal::ONE;
        count.to_usize().unwrap_or(0)
    }

    /// Validate range constraints.
    pub fn validate(&self, axis_label: &str) -> Result<(), SweepRangeError> {
        if self.step <= Decimal::ZERO {
            return Err(SweepRangeError {
                axis_label: axis_label.into(),
                message: format!("step must be > 0, got {}", self.step),
            });
        }
        if self.from > self.to {
            return Err(SweepRangeError {
                axis_label: axis_label.into(),
                message: format!("from ({}) must be <= to ({})", self.from, self.to),
            });
        }
        let count = self.point_count();
        if count > MAX_SWEEP_POINTS {
            return Err(SweepRangeError {
                axis_label: axis_label.into(),
                message: format!("too many points ({count}), maximum is {MAX_SWEEP_POINTS}"),
            });
        }
        Ok(())
    }

    /// Iterate over the discrete points in the range.
    pub fn points(&self) -> Vec<Decimal> {
        let count = self.point_count();
        (0..count)
            .map(|i| self.from + self.step * Decimal::from(i))
            .collect()
    }
}

// ─── SweepTarget ──────────────────────────────────────────────────────

/// Target specification for parameter sweeps.
///
/// Identifies which action in which group to vary, with an optional
/// multiplier for coupled parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepTarget {
    /// Which group (for multi-group plans). None = single-group.
    pub group_index: Option<usize>,
    /// Which action within the group's action sequence.
    pub action_index: usize,
    /// Multiplier applied to the sweep parameter for this target.
    /// Default: 1.0 (coupled parameter pattern).
    #[serde(default = "default_one", with = "rust_decimal::serde::str")]
    pub multiplier: Decimal,
}

fn default_one() -> Decimal {
    Decimal::ONE
}

// ─── SweepAxisType ────────────────────────────────────────────────────

/// Discriminant for sweep axis variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SweepAxisType {
    Parameter,
    Catalogue,
    Partition,
    Grid,
}

impl fmt::Display for SweepAxisType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            SweepAxisType::Parameter => "parameter",
            SweepAxisType::Catalogue => "catalogue",
            SweepAxisType::Partition => "partition",
            SweepAxisType::Grid => "grid",
        };
        write!(f, "{s}")
    }
}

impl Default for SweepAxisType {
    fn default() -> Self {
        Self::Parameter
    }
}

impl std::str::FromStr for SweepAxisType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "parameter" => Ok(SweepAxisType::Parameter),
            "catalogue" | "catalog" => Ok(SweepAxisType::Catalogue),
            "partition" => Ok(SweepAxisType::Partition),
            "grid" => Ok(SweepAxisType::Grid),
            _ => Err(format!("unknown sweep axis type: {s}")),
        }
    }
}

// ─── SweepAxis ────────────────────────────────────────────────────────

/// Sweep axis — defines how parameters are varied.
///
/// - **Parameter**: Vary action values across a numeric range
/// - **Catalogue**: Compare alternative entity sets (A/B testing)
/// - **Partition**: Split by column value (e.g., region, category)
/// - **Grid**: 2D sweep (parameter range × partition values)
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SweepAxis {
    Parameter {
        #[serde(flatten)]
        range: SweepRange,
        targets: Vec<SweepTarget>,
    },
    Catalogue {
        labels: Vec<String>,
    },
    Partition {
        column: String,
        values: Option<Vec<String>>,
    },
    Grid {
        #[serde(flatten)]
        range: SweepRange,
        target_action_index: usize,
        #[serde(default = "default_one", with = "rust_decimal::serde::str")]
        multiplier: Decimal,
        column: String,
        values: Option<Vec<String>>,
    },
}

impl SweepAxis {
    /// Extract the discriminant.
    pub fn axis_type(&self) -> SweepAxisType {
        match self {
            SweepAxis::Parameter { .. } => SweepAxisType::Parameter,
            SweepAxis::Catalogue { .. } => SweepAxisType::Catalogue,
            SweepAxis::Partition { .. } => SweepAxisType::Partition,
            SweepAxis::Grid { .. } => SweepAxisType::Grid,
        }
    }
}

// ─── SweepPoint & SweepSummary ────────────────────────────────────────

/// A single point in a sweep result.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepPoint {
    pub label: String,
    pub entity_count: usize,
    pub impact: MetricPoint,
}

/// Summary statistics for a completed sweep.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepSummary {
    pub point_count: usize,
    pub axis_type: SweepAxisType,
    pub max_delta_label: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub max_delta_percent: Decimal,
    pub min_delta_label: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub min_delta_percent: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub breakeven_label: Option<String>,
}

// ─── SweepMetric Trait ────────────────────────────────────────────────

/// Domain-specific metric computation for sweep analysis.
///
/// Users implement this trait for their entity type `E` and action type `A`.
/// The framework calls `compute` for each sweep point to measure impact.
///
/// # Example
///
/// ```ignore
/// impl SweepMetric<InventorySlice, AllocationAction> for ThroughputMetric {
///     fn compute(&self, entities: &[InventorySlice], actions: &[AllocationAction]) -> MetricPoint {
///         let baseline: Decimal = entities.iter().map(|item| item.capacity).sum();
///         let modified: Decimal = entities.iter().map(|item| apply_actions(item.capacity, actions)).sum();
///         MetricPoint::from_baseline_modified(baseline, modified)
///     }
///     fn name(&self) -> &'static str { "throughput" }
/// }
/// ```
pub trait SweepMetric<E, A>: Send + Sync {
    fn compute(&self, entities: &[E], actions: &[A]) -> MetricPoint;
    fn name(&self) -> &'static str;
}

// ─── ActionValue Trait ────────────────────────────────────────────────

/// Trait for actions with a single numeric parameter that can be varied.
///
/// Required for parameter sweeps — the framework needs to scale action
/// values by the sweep multiplier.
pub trait ActionValue {
    /// Get the numeric parameter value.
    fn value(&self) -> Decimal;
    /// Create a new action with the given value (immutable builder).
    fn with_value(&self, value: Decimal) -> Self;
}

// ─── Pure Computation Functions ───────────────────────────────────────

/// Find the breakeven point (where percent_change crosses zero).
///
/// Returns the label of the first point at or just after the sign change.
pub fn find_breakeven(points: &[SweepPoint]) -> Option<String> {
    if points.len() < 2 {
        return None;
    }
    // Check for exact zero
    for p in points {
        if p.impact.percent_change.is_zero() {
            return Some(p.label.clone());
        }
    }
    // Check for sign change
    for window in points.windows(2) {
        let a = window[0].impact.percent_change;
        let b = window[1].impact.percent_change;
        if (a.is_sign_negative() && b.is_sign_positive())
            || (a.is_sign_positive() && b.is_sign_negative())
        {
            // Return the point closer to zero
            if a.abs() <= b.abs() {
                return Some(window[0].label.clone());
            } else {
                return Some(window[1].label.clone());
            }
        }
    }
    None
}

/// Compute summary statistics from sweep points.
pub fn compute_sweep_summary(points: &[SweepPoint], axis_type: SweepAxisType) -> SweepSummary {
    let (max_label, max_pct) = points
        .iter()
        .max_by(|a, b| {
            a.impact
                .percent_change
                .partial_cmp(&b.impact.percent_change)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|p| (p.label.clone(), p.impact.percent_change))
        .unwrap_or_default();

    let (min_label, min_pct) = points
        .iter()
        .min_by(|a, b| {
            a.impact
                .percent_change
                .partial_cmp(&b.impact.percent_change)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|p| (p.label.clone(), p.impact.percent_change))
        .unwrap_or_default();

    SweepSummary {
        point_count: points.len(),
        axis_type,
        max_delta_label: max_label,
        max_delta_percent: max_pct,
        min_delta_label: min_label,
        min_delta_percent: min_pct,
        breakeven_label: find_breakeven(points),
    }
}

/// Build a human-readable description of sweep results.
pub fn build_description(
    summary: &SweepSummary,
    axis: &SweepAxis,
    total_entities: usize,
) -> String {
    let axis_desc = match axis {
        SweepAxis::Parameter { range, .. } => {
            format!("parameter sweep from {} to {}", range.from, range.to)
        }
        SweepAxis::Catalogue { labels } => {
            format!("catalogue comparison across {} variants", labels.len())
        }
        SweepAxis::Partition { column, .. } => {
            format!("partition by {column}")
        }
        SweepAxis::Grid { column, range, .. } => {
            format!("grid sweep from {} to {} × {column}", range.from, range.to)
        }
    };

    let impact_range = if summary.min_delta_percent == summary.max_delta_percent {
        format!("{:+.1}%", summary.max_delta_percent)
    } else {
        format!(
            "{:+.1}% to {:+.1}%",
            summary.min_delta_percent, summary.max_delta_percent
        )
    };

    let breakeven = summary
        .breakeven_label
        .as_ref()
        .map(|b| format!(", breakeven at {b}"))
        .unwrap_or_default();

    format!(
        "Analyzed {total_entities} entities across {} points ({axis_desc}). Impact: {impact_range}{breakeven}.",
        summary.point_count
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── MetricPoint Tests ────────────────────────────────────────

    #[test]
    fn metric_point_zero_is_identity() {
        let mp = MetricPoint::from_baseline_modified(Decimal::from(100), Decimal::from(110));
        assert_eq!(mp.combine(&MetricPoint::ZERO), mp);
        assert_eq!(MetricPoint::ZERO.combine(&mp), mp);
    }

    #[test]
    fn metric_point_combine_is_additive() {
        let a = MetricPoint::from_baseline_modified(Decimal::from(100), Decimal::from(110));
        let b = MetricPoint::from_baseline_modified(Decimal::from(200), Decimal::from(180));
        let combined = a.combine(&b);
        assert_eq!(combined.baseline, Decimal::from(300));
        assert_eq!(combined.modified, Decimal::from(290));
        assert_eq!(combined.delta, Decimal::from(-10));
    }

    #[test]
    fn metric_point_zero_baseline_safe() {
        let mp = MetricPoint::from_baseline_modified(Decimal::ZERO, Decimal::from(50));
        assert_eq!(mp.percent_change, Decimal::ZERO);
    }

    #[test]
    fn metric_point_derived_fields() {
        let mp = MetricPoint::from_baseline_modified(Decimal::from(200), Decimal::from(220));
        assert_eq!(mp.delta, Decimal::from(20));
        assert_eq!(mp.percent_change, Decimal::from(10)); // 20/200 * 100 = 10
    }

    // ─── SweepRange Tests ─────────────────────────────────────────

    #[test]
    fn range_point_count() {
        let r = SweepRange {
            from: Decimal::ZERO,
            to: Decimal::from(10),
            step: Decimal::from(2),
        };
        assert_eq!(r.point_count(), 6); // 0, 2, 4, 6, 8, 10
    }

    #[test]
    fn range_points_list() {
        let r = SweepRange {
            from: Decimal::from(-5),
            to: Decimal::from(5),
            step: Decimal::from(5),
        };
        assert_eq!(
            r.points(),
            vec![Decimal::from(-5), Decimal::ZERO, Decimal::from(5),]
        );
    }

    #[test]
    fn range_validates_step_zero() {
        let r = SweepRange {
            from: Decimal::ZERO,
            to: Decimal::from(10),
            step: Decimal::ZERO,
        };
        assert!(r.validate("test").is_err());
    }

    #[test]
    fn range_validates_from_greater_than_to() {
        let r = SweepRange {
            from: Decimal::from(10),
            to: Decimal::ZERO,
            step: Decimal::ONE,
        };
        assert!(r.validate("test").is_err());
    }

    #[test]
    fn range_validates_too_many_points() {
        let r = SweepRange {
            from: Decimal::ZERO,
            to: Decimal::from(1000),
            step: Decimal::ONE,
        };
        assert!(r.validate("test").is_err());
    }

    #[test]
    fn range_valid_ok() {
        let r = SweepRange {
            from: Decimal::from(-10),
            to: Decimal::from(10),
            step: Decimal::from(5),
        };
        assert!(r.validate("test").is_ok());
    }

    // ─── SweepAxisType Tests ──────────────────────────────────────

    #[test]
    fn axis_type_display_roundtrip() {
        for t in [
            SweepAxisType::Parameter,
            SweepAxisType::Catalogue,
            SweepAxisType::Partition,
            SweepAxisType::Grid,
        ] {
            let s = t.to_string();
            let parsed: SweepAxisType = s.parse().unwrap();
            assert_eq!(parsed, t);
        }
    }

    #[test]
    fn axis_type_accepts_catalog_spelling() {
        let parsed: SweepAxisType = "catalog".parse().unwrap();
        assert_eq!(parsed, SweepAxisType::Catalogue);
    }

    // ─── SweepAxis Tests ──────────────────────────────────────────

    #[test]
    fn sweep_axis_discriminant() {
        let param = SweepAxis::Parameter {
            range: SweepRange {
                from: Decimal::ZERO,
                to: Decimal::from(10),
                step: Decimal::ONE,
            },
            targets: vec![],
        };
        assert_eq!(param.axis_type(), SweepAxisType::Parameter);

        let cat = SweepAxis::Catalogue {
            labels: vec!["a".into()],
        };
        assert_eq!(cat.axis_type(), SweepAxisType::Catalogue);
    }

    // ─── find_breakeven Tests ─────────────────────────────────────

    fn make_point(label: &str, pct: i64) -> SweepPoint {
        SweepPoint {
            label: label.into(),
            entity_count: 10,
            impact: MetricPoint::from_baseline_modified(
                Decimal::from(100),
                Decimal::from(100 + pct),
            ),
        }
    }

    #[test]
    fn breakeven_exact_zero() {
        let points = vec![
            make_point("-5%", -5),
            make_point("0%", 0),
            make_point("+5%", 5),
        ];
        assert_eq!(find_breakeven(&points), Some("0%".into()));
    }

    #[test]
    fn breakeven_sign_change() {
        let points = vec![
            make_point("-10%", -10),
            make_point("-2%", -2),
            make_point("+3%", 3),
            make_point("+10%", 10),
        ];
        // -2% is closer to zero than +3%
        assert_eq!(find_breakeven(&points), Some("-2%".into()));
    }

    #[test]
    fn breakeven_none_when_all_positive() {
        let points = vec![
            make_point("+1%", 1),
            make_point("+5%", 5),
            make_point("+10%", 10),
        ];
        assert_eq!(find_breakeven(&points), None);
    }

    #[test]
    fn breakeven_none_when_single_point() {
        let points = vec![make_point("x", 5)];
        assert_eq!(find_breakeven(&points), None);
    }

    // ─── compute_sweep_summary Tests ──────────────────────────────

    #[test]
    fn summary_finds_extremes() {
        let points = vec![
            make_point("-10%", -10),
            make_point("0%", 0),
            make_point("+15%", 15),
        ];
        let summary = compute_sweep_summary(&points, SweepAxisType::Parameter);
        assert_eq!(summary.max_delta_label, "+15%");
        assert_eq!(summary.min_delta_label, "-10%");
        assert_eq!(summary.point_count, 3);
    }

    // ─── build_description Tests ──────────────────────────────────

    #[test]
    fn description_parameter() {
        let points = vec![
            make_point("-5%", -5),
            make_point("0%", 0),
            make_point("+5%", 5),
        ];
        let summary = compute_sweep_summary(&points, SweepAxisType::Parameter);
        let axis = SweepAxis::Parameter {
            range: SweepRange {
                from: Decimal::from(-5),
                to: Decimal::from(5),
                step: Decimal::from(5),
            },
            targets: vec![],
        };
        let desc = build_description(&summary, &axis, 100);
        assert!(desc.contains("100 entities"));
        assert!(desc.contains("3 points"));
        assert!(desc.contains("parameter sweep"));
    }

    // ─── Property-Based Tests ─────────────────────────────────────

    use hegel::generators;

    #[hegel::test]
    fn metric_point_identity(tc: hegel::TestCase) {
        let baseline = tc.draw(
            generators::integers::<i64>()
                .min_value(-10000)
                .max_value(9999),
        );
        let modified = tc.draw(
            generators::integers::<i64>()
                .min_value(-10000)
                .max_value(9999),
        );
        let mp =
            MetricPoint::from_baseline_modified(Decimal::from(baseline), Decimal::from(modified));
        // combine(ZERO, x) == x
        assert_eq!(
            MetricPoint::ZERO.combine(&mp),
            mp.clone(),
            "left identity failed"
        );
        // combine(x, ZERO) == x
        assert_eq!(mp.combine(&MetricPoint::ZERO), mp, "right identity failed");
    }

    #[hegel::test]
    fn metric_point_commutativity(tc: hegel::TestCase) {
        let b1 = tc.draw(
            generators::integers::<i64>()
                .min_value(-5000)
                .max_value(4999),
        );
        let m1 = tc.draw(
            generators::integers::<i64>()
                .min_value(-5000)
                .max_value(4999),
        );
        let b2 = tc.draw(
            generators::integers::<i64>()
                .min_value(-5000)
                .max_value(4999),
        );
        let m2 = tc.draw(
            generators::integers::<i64>()
                .min_value(-5000)
                .max_value(4999),
        );
        let a = MetricPoint::from_baseline_modified(Decimal::from(b1), Decimal::from(m1));
        let b = MetricPoint::from_baseline_modified(Decimal::from(b2), Decimal::from(m2));
        assert_eq!(a.combine(&b), b.combine(&a));
    }

    #[hegel::test]
    fn metric_point_associativity(tc: hegel::TestCase) {
        let b1 = tc.draw(
            generators::integers::<i64>()
                .min_value(-3000)
                .max_value(2999),
        );
        let m1 = tc.draw(
            generators::integers::<i64>()
                .min_value(-3000)
                .max_value(2999),
        );
        let b2 = tc.draw(
            generators::integers::<i64>()
                .min_value(-3000)
                .max_value(2999),
        );
        let m2 = tc.draw(
            generators::integers::<i64>()
                .min_value(-3000)
                .max_value(2999),
        );
        let b3 = tc.draw(
            generators::integers::<i64>()
                .min_value(-3000)
                .max_value(2999),
        );
        let m3 = tc.draw(
            generators::integers::<i64>()
                .min_value(-3000)
                .max_value(2999),
        );
        let a = MetricPoint::from_baseline_modified(Decimal::from(b1), Decimal::from(m1));
        let b = MetricPoint::from_baseline_modified(Decimal::from(b2), Decimal::from(m2));
        let c = MetricPoint::from_baseline_modified(Decimal::from(b3), Decimal::from(m3));
        assert_eq!((a.combine(&b)).combine(&c), a.combine(&(b.combine(&c))));
    }

    #[hegel::test]
    fn metric_point_delta_consistent(tc: hegel::TestCase) {
        let baseline = tc.draw(
            generators::integers::<i64>()
                .min_value(-10000)
                .max_value(9999),
        );
        let modified = tc.draw(
            generators::integers::<i64>()
                .min_value(-10000)
                .max_value(9999),
        );
        let mp =
            MetricPoint::from_baseline_modified(Decimal::from(baseline), Decimal::from(modified));
        assert_eq!(mp.delta, mp.modified - mp.baseline);
    }

    #[hegel::test]
    fn sweep_range_point_count_bounded(tc: hegel::TestCase) {
        let from = tc.draw(generators::integers::<i64>().min_value(-100).max_value(99));
        let to = tc.draw(generators::integers::<i64>().min_value(-100).max_value(99));
        let step = tc.draw(generators::integers::<i64>().min_value(1).max_value(49));
        let r = SweepRange {
            from: Decimal::from(from),
            to: Decimal::from(to),
            step: Decimal::from(step),
        };
        let count = r.point_count();
        if from <= to {
            assert!(count >= 1, "at least one point when from <= to");
        } else {
            assert_eq!(count, 0, "zero points when from > to");
        }
    }
}
