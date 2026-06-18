//! Sweep orchestration: pure computation functions for parameter, comparison, and grid sweeps.
//!
//! These functions are the reusable core of sweep analysis. They operate on
//! pre-resolved data (items + actions) and produce `SweepPoint` sequences.
//!
//! # Sweep Variants
//!
//! - **Parameter**: Vary action values across a numeric range. Monoid fold across groups.
//! - **Comparison**: Compute metric per slice (catalogue or partition) with shared/per-slice actions.
//! - **Grid**: Parameter × Partition cross-product.
//!
//! # Laws
//!
//! - Parameter sweep: monoid fold via `MetricPoint::combine()` is associative + commutative,
//!   so group ordering doesn't affect the result.
//! - Comparison sweep: each slice is independent — order of slices = order of output points.
//! - Grid sweep: Cartesian product of slices × parameter values.

use rust_decimal::Decimal;

use crate::sweep::{ActionValue, MetricPoint, SweepMetric, SweepPoint, SweepRange, SweepTarget};

/// Observer for sweep point emission.
///
/// Enables real-time streaming of sweep results via EventSink or other
/// channels. The framework calls `on_point` for each computed point.
///
/// # Laws
///
/// - **L1 (Non-blocking)**: `on_point` must not block the computation thread.
/// - **L2 (Order preservation)**: Points are emitted in computation order.
pub trait SweepObserver: Send + Sync {
    /// Called when a sweep point is computed.
    ///
    /// `index` is zero-based, `total` is the expected total point count
    /// (may be approximate for grid sweeps).
    fn on_point(&self, point: &SweepPoint, index: usize, total: usize);
}

/// No-op observer — monoid identity for sweep observation.
pub struct NullSweepObserver;

impl SweepObserver for NullSweepObserver {
    fn on_point(&self, _: &SweepPoint, _: usize, _: usize) {}
}

/// A resolved group of items + actions for sweep computation.
///
/// Used by `parameter_sweep` when a plan has multiple groups
/// (e.g., different entity sets with different actions).
pub struct SweepGroup<I, A> {
    pub items: Vec<I>,
    pub actions: Vec<A>,
}

/// A resolved slice for comparison and grid sweeps.
///
/// Each slice represents a named partition or catalogue variant
/// with its own items and optional action overrides.
pub struct SweepSlice<I, A> {
    pub label: String,
    pub items: Vec<I>,
    /// Per-slice action overrides. If `None`, uses the default actions.
    pub actions: Option<Vec<A>>,
}

/// Parameter sweep: vary action values across a numeric range.
///
/// For each point in the range:
/// 1. Apply sweep targets (set action values to `point * multiplier`)
/// 2. Compute metric for each group
/// 3. Combine group metrics via `MetricPoint::combine()` (monoid fold)
///
/// The `ActionValue` trait provides `value()` / `with_value()` for
/// immutable action mutation. Actions are cloned once per group,
/// then mutated per point via replacement.
pub fn parameter_sweep<E, A, M>(
    groups: &[SweepGroup<E, A>],
    range: &SweepRange,
    targets: &[SweepTarget],
    metric: &M,
) -> Vec<SweepPoint>
where
    A: ActionValue + Clone,
    M: SweepMetric<E, A>,
{
    let points = range.points();
    let mut result = Vec::with_capacity(points.len());

    for p in &points {
        // Build per-group action buffers with targets applied
        let combined = groups
            .iter()
            .enumerate()
            .map(|(gi, group)| {
                let mut actions = group.actions.clone();
                for target in targets {
                    // Apply target if it matches this group (or if group_index is None = all groups)
                    if target.group_index.is_none() || target.group_index == Some(gi) {
                        if let Some(action) = actions.get_mut(target.action_index) {
                            let new_val = *p * target.multiplier;
                            *action = action.with_value(new_val);
                        }
                    }
                }
                metric.compute(&group.items, &actions)
            })
            .fold(MetricPoint::ZERO, |acc, mp| acc.combine(&mp));

        let total_items: usize = groups.iter().map(|g| g.items.len()).sum();
        let label = normalize_label(*p);

        result.push(SweepPoint {
            label,
            entity_count: total_items,
            impact: combined,
        });
    }

    result
}

/// Comparison sweep: compute metric per slice with shared/per-slice actions.
///
/// Used by both Catalogue (per-variant actions) and Partition (shared actions) axes.
/// Each slice produces one `SweepPoint`. If a slice has no action overrides,
/// `default_actions` are used.
pub fn comparison_sweep<E, A, M>(
    slices: &[SweepSlice<E, A>],
    default_actions: &[A],
    metric: &M,
) -> Vec<SweepPoint>
where
    M: SweepMetric<E, A>,
{
    slices
        .iter()
        .map(|slice| {
            let actions = slice.actions.as_deref().unwrap_or(default_actions);
            let impact = metric.compute(&slice.items, actions);
            SweepPoint {
                label: slice.label.clone(),
                entity_count: slice.items.len(),
                impact,
            }
        })
        .collect()
}

/// Grid sweep: Parameter × Partition cross-product.
///
/// For each slice (partition), runs a parameter sweep across the range.
/// Labels are formatted as `"{slice_label} @ {parameter_value}"`.
///
/// `target_action_index` and `multiplier` identify which action to vary
/// within each slice's action buffer.
pub fn grid_sweep<E, A, M>(
    slices: &[SweepSlice<E, A>],
    default_actions: &[A],
    range: &SweepRange,
    target_action_index: usize,
    multiplier: Decimal,
    metric: &M,
) -> Vec<SweepPoint>
where
    A: ActionValue + Clone,
    M: SweepMetric<E, A>,
{
    let param_points = range.points();
    let mut result = Vec::with_capacity(slices.len() * param_points.len());

    for slice in slices {
        // Clone actions once per slice
        let base_actions: Vec<A> = slice
            .actions
            .clone()
            .unwrap_or_else(|| default_actions.to_vec());

        for p in &param_points {
            let mut actions = base_actions.clone();
            if let Some(action) = actions.get_mut(target_action_index) {
                let new_val = *p * multiplier;
                *action = action.with_value(new_val);
            }

            let impact = metric.compute(&slice.items, &actions);
            let label = format!("{} @ {}", slice.label, normalize_label(*p));

            result.push(SweepPoint {
                label,
                entity_count: slice.items.len(),
                impact,
            });
        }
    }

    result
}

/// Streaming parameter sweep: same as [`parameter_sweep`] but emits each point
/// to `observer` as it is computed.
///
/// Returns the collected points as well, preserving the same contract as the
/// non-streaming variant.
pub fn parameter_sweep_streaming<E, A, M>(
    groups: &[SweepGroup<E, A>],
    range: &SweepRange,
    targets: &[SweepTarget],
    metric: &M,
    observer: &dyn SweepObserver,
) -> Vec<SweepPoint>
where
    A: ActionValue + Clone,
    M: SweepMetric<E, A>,
{
    let points = range.points();
    let total = points.len();
    let mut result = Vec::with_capacity(total);

    for (idx, p) in points.iter().enumerate() {
        let combined = groups
            .iter()
            .enumerate()
            .map(|(gi, group)| {
                let mut actions = group.actions.clone();
                for target in targets {
                    if target.group_index.is_none() || target.group_index == Some(gi) {
                        if let Some(action) = actions.get_mut(target.action_index) {
                            let new_val = *p * target.multiplier;
                            *action = action.with_value(new_val);
                        }
                    }
                }
                metric.compute(&group.items, &actions)
            })
            .fold(MetricPoint::ZERO, |acc, mp| acc.combine(&mp));

        let total_items: usize = groups.iter().map(|g| g.items.len()).sum();
        let label = normalize_label(*p);

        let sweep_point = SweepPoint {
            label,
            entity_count: total_items,
            impact: combined,
        };

        observer.on_point(&sweep_point, idx, total);
        result.push(sweep_point);
    }

    result
}

/// Streaming comparison sweep: same as [`comparison_sweep`] but emits each point
/// to `observer` as it is computed.
///
/// Returns the collected points as well, preserving the same contract as the
/// non-streaming variant.
pub fn comparison_sweep_streaming<E, A, M>(
    slices: &[SweepSlice<E, A>],
    default_actions: &[A],
    metric: &M,
    observer: &dyn SweepObserver,
) -> Vec<SweepPoint>
where
    M: SweepMetric<E, A>,
{
    let total = slices.len();
    let mut result = Vec::with_capacity(total);

    for (idx, slice) in slices.iter().enumerate() {
        let actions = slice.actions.as_deref().unwrap_or(default_actions);
        let impact = metric.compute(&slice.items, actions);
        let sweep_point = SweepPoint {
            label: slice.label.clone(),
            entity_count: slice.items.len(),
            impact,
        };

        observer.on_point(&sweep_point, idx, total);
        result.push(sweep_point);
    }

    result
}

/// Streaming grid sweep: same as [`grid_sweep`] but emits each point
/// to `observer` as it is computed.
///
/// Returns the collected points as well, preserving the same contract as the
/// non-streaming variant.
///
/// `total` passed to the observer is `slices.len() * range.points().len()`.
pub fn grid_sweep_streaming<E, A, M>(
    slices: &[SweepSlice<E, A>],
    default_actions: &[A],
    range: &SweepRange,
    target_action_index: usize,
    multiplier: Decimal,
    metric: &M,
    observer: &dyn SweepObserver,
) -> Vec<SweepPoint>
where
    A: ActionValue + Clone,
    M: SweepMetric<E, A>,
{
    let param_points = range.points();
    let total = slices.len() * param_points.len();
    let mut result = Vec::with_capacity(total);
    let mut idx = 0;

    for slice in slices {
        let base_actions: Vec<A> = slice
            .actions
            .clone()
            .unwrap_or_else(|| default_actions.to_vec());

        for p in &param_points {
            let mut actions = base_actions.clone();
            if let Some(action) = actions.get_mut(target_action_index) {
                let new_val = *p * multiplier;
                *action = action.with_value(new_val);
            }

            let impact = metric.compute(&slice.items, &actions);
            let label = format!("{} @ {}", slice.label, normalize_label(*p));

            let sweep_point = SweepPoint {
                label,
                entity_count: slice.items.len(),
                impact,
            };

            observer.on_point(&sweep_point, idx, total);
            result.push(sweep_point);
            idx += 1;
        }
    }

    result
}

/// Normalize a decimal value for use as a sweep point label.
/// Removes trailing zeros (e.g., "1.50" → "1.5", "2.00" → "2").
fn normalize_label(value: Decimal) -> String {
    let s = value.normalize().to_string();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Test Domain ─────────────────────────────────────────────────

    #[derive(Clone)]
    struct Item {
        baseline_value: Decimal,
        quantity: Decimal,
    }

    #[derive(Clone)]
    struct Action {
        value: Decimal,
    }

    impl ActionValue for Action {
        fn value(&self) -> Decimal {
            self.value
        }
        fn with_value(&self, value: Decimal) -> Self {
            Self { value }
        }
    }

    /// Revenue metric: sum(quantity * value) for modified, sum(quantity * baseline) for baseline.
    struct RevenueMetric;

    impl SweepMetric<Item, Action> for RevenueMetric {
        fn compute(&self, items: &[Item], actions: &[Action]) -> MetricPoint {
            let action_value = actions.first().map(|a| a.value).unwrap_or(Decimal::ZERO);
            let baseline: Decimal = items.iter().map(|i| i.baseline_value * i.quantity).sum();
            let modified: Decimal = items.iter().map(|i| action_value * i.quantity).sum();
            MetricPoint::from_baseline_modified(baseline, modified)
        }

        fn name(&self) -> &'static str {
            "revenue"
        }
    }

    fn make_items(prices: &[(i64, i64)]) -> Vec<Item> {
        prices
            .iter()
            .map(|(price, qty)| Item {
                baseline_value: Decimal::from(*price),
                quantity: Decimal::from(*qty),
            })
            .collect()
    }

    fn make_action(value: i64) -> Action {
        Action {
            value: Decimal::from(value),
        }
    }

    // ─── Parameter Sweep Tests ───────────────────────────────────────

    #[test]
    fn parameter_sweep_single_group() {
        let groups = vec![SweepGroup {
            items: make_items(&[(10, 1)]),
            actions: vec![make_action(10)],
        }];
        let range = SweepRange {
            from: Decimal::from(8),
            to: Decimal::from(12),
            step: Decimal::from(2),
        };
        let targets = vec![SweepTarget {
            group_index: None,
            action_index: 0,
            multiplier: Decimal::ONE,
        }];

        let points = parameter_sweep(&groups, &range, &targets, &RevenueMetric);

        assert_eq!(points.len(), 3); // 8, 10, 12
        assert_eq!(points[0].label, "8");
        assert_eq!(points[1].label, "10");
        assert_eq!(points[2].label, "12");

        // At value=8: baseline=10, modified=8 → delta=-2 → -20%
        assert_eq!(points[0].impact.percent_change, Decimal::from(-20));
        // At value=10: baseline=10, modified=10 → 0%
        assert_eq!(points[1].impact.percent_change, Decimal::ZERO);
        // At value=12: baseline=10, modified=12 → +20%
        assert_eq!(points[2].impact.percent_change, Decimal::from(20));
    }

    #[test]
    fn parameter_sweep_multi_group_combines() {
        let groups = vec![
            SweepGroup {
                items: make_items(&[(10, 1)]),
                actions: vec![make_action(10)],
            },
            SweepGroup {
                items: make_items(&[(20, 1)]),
                actions: vec![make_action(20)],
            },
        ];
        let range = SweepRange {
            from: Decimal::from(15),
            to: Decimal::from(15),
            step: Decimal::ONE,
        };
        let targets = vec![SweepTarget {
            group_index: None,
            action_index: 0,
            multiplier: Decimal::ONE,
        }];

        let points = parameter_sweep(&groups, &range, &targets, &RevenueMetric);

        assert_eq!(points.len(), 1);
        // Group 1: baseline=10, modified=15
        // Group 2: baseline=20, modified=15
        // Combined: baseline=30, modified=30 → 0%
        assert_eq!(points[0].impact.baseline, Decimal::from(30));
        assert_eq!(points[0].impact.modified, Decimal::from(30));
        assert_eq!(points[0].impact.percent_change, Decimal::ZERO);
        assert_eq!(points[0].entity_count, 2);
    }

    #[test]
    fn parameter_sweep_with_multiplier() {
        let groups = vec![SweepGroup {
            items: make_items(&[(10, 1)]),
            actions: vec![make_action(10)],
        }];
        let range = SweepRange {
            from: Decimal::from(10),
            to: Decimal::from(10),
            step: Decimal::ONE,
        };
        let targets = vec![SweepTarget {
            group_index: None,
            action_index: 0,
            multiplier: Decimal::from(2), // 10 * 2 = 20
        }];

        let points = parameter_sweep(&groups, &range, &targets, &RevenueMetric);

        assert_eq!(points.len(), 1);
        // Action value = 10 * 2 = 20, baseline = 10 → +100%
        assert_eq!(points[0].impact.modified, Decimal::from(20));
        assert_eq!(points[0].impact.percent_change, Decimal::from(100));
    }

    #[test]
    fn parameter_sweep_group_specific_target() {
        let groups = vec![
            SweepGroup {
                items: make_items(&[(10, 1)]),
                actions: vec![make_action(10)],
            },
            SweepGroup {
                items: make_items(&[(20, 1)]),
                actions: vec![make_action(20)],
            },
        ];
        let range = SweepRange {
            from: Decimal::from(15),
            to: Decimal::from(15),
            step: Decimal::ONE,
        };
        // Only target group 0
        let targets = vec![SweepTarget {
            group_index: Some(0),
            action_index: 0,
            multiplier: Decimal::ONE,
        }];

        let points = parameter_sweep(&groups, &range, &targets, &RevenueMetric);

        assert_eq!(points.len(), 1);
        // Group 0: baseline=10, modified=15 (targeted)
        // Group 1: baseline=20, modified=20 (untouched)
        // Combined: baseline=30, modified=35
        assert_eq!(points[0].impact.baseline, Decimal::from(30));
        assert_eq!(points[0].impact.modified, Decimal::from(35));
    }

    #[test]
    fn parameter_sweep_empty_range() {
        let groups = vec![SweepGroup {
            items: make_items(&[(10, 1)]),
            actions: vec![make_action(10)],
        }];
        let range = SweepRange {
            from: Decimal::from(10),
            to: Decimal::from(5),
            step: Decimal::ONE,
        };
        let targets = vec![];

        let points = parameter_sweep(&groups, &range, &targets, &RevenueMetric);
        assert!(points.is_empty());
    }

    // ─── Comparison Sweep Tests ──────────────────────────────────────

    #[test]
    fn comparison_sweep_with_default_actions() {
        let slices = vec![
            SweepSlice {
                label: "Region A".into(),
                items: make_items(&[(10, 2)]),
                actions: None,
            },
            SweepSlice {
                label: "Region B".into(),
                items: make_items(&[(10, 3)]),
                actions: None,
            },
        ];
        let default_actions = vec![make_action(12)];

        let points = comparison_sweep(&slices, &default_actions, &RevenueMetric);

        assert_eq!(points.len(), 2);
        assert_eq!(points[0].label, "Region A");
        assert_eq!(points[0].entity_count, 1);
        // baseline=20 (10*2), modified=24 (12*2) → +20%
        assert_eq!(points[0].impact.percent_change, Decimal::from(20));

        assert_eq!(points[1].label, "Region B");
        assert_eq!(points[1].entity_count, 1);
        // baseline=30 (10*3), modified=36 (12*3) → +20%
        assert_eq!(points[1].impact.percent_change, Decimal::from(20));
    }

    #[test]
    fn comparison_sweep_with_per_slice_actions() {
        let slices = vec![
            SweepSlice {
                label: "Variant A".into(),
                items: make_items(&[(10, 1)]),
                actions: Some(vec![make_action(12)]),
            },
            SweepSlice {
                label: "Variant B".into(),
                items: make_items(&[(10, 1)]),
                actions: Some(vec![make_action(8)]),
            },
        ];
        let default_actions = vec![make_action(99)]; // should not be used

        let points = comparison_sweep(&slices, &default_actions, &RevenueMetric);

        assert_eq!(points.len(), 2);
        // Variant A: 10→12, +20%
        assert_eq!(points[0].impact.percent_change, Decimal::from(20));
        // Variant B: 10→8, -20%
        assert_eq!(points[1].impact.percent_change, Decimal::from(-20));
    }

    #[test]
    fn comparison_sweep_empty_slices() {
        let points = comparison_sweep::<Item, Action, _>(&[], &[], &RevenueMetric);
        assert!(points.is_empty());
    }

    // ─── Grid Sweep Tests ────────────────────────────────────────────

    #[test]
    fn grid_sweep_basic() {
        let slices = vec![
            SweepSlice {
                label: "East".into(),
                items: make_items(&[(10, 1)]),
                actions: None,
            },
            SweepSlice {
                label: "West".into(),
                items: make_items(&[(20, 1)]),
                actions: None,
            },
        ];
        let default_actions = vec![make_action(10)];
        let range = SweepRange {
            from: Decimal::from(8),
            to: Decimal::from(12),
            step: Decimal::from(2),
        };

        let points = grid_sweep(
            &slices,
            &default_actions,
            &range,
            0,
            Decimal::ONE,
            &RevenueMetric,
        );

        // 2 slices × 3 parameter points = 6 points
        assert_eq!(points.len(), 6);

        // Check labels
        assert_eq!(points[0].label, "East @ 8");
        assert_eq!(points[1].label, "East @ 10");
        assert_eq!(points[2].label, "East @ 12");
        assert_eq!(points[3].label, "West @ 8");
        assert_eq!(points[4].label, "West @ 10");
        assert_eq!(points[5].label, "West @ 12");

        // East @ 8: baseline=10, modified=8 → -20%
        assert_eq!(points[0].impact.percent_change, Decimal::from(-20));
        // West @ 12: baseline=20, modified=12 → -40%
        assert_eq!(points[5].impact.percent_change, Decimal::from(-40));
    }

    #[test]
    fn grid_sweep_with_multiplier() {
        let slices = vec![SweepSlice {
            label: "All".into(),
            items: make_items(&[(10, 1)]),
            actions: None,
        }];
        let default_actions = vec![make_action(10)];
        let range = SweepRange {
            from: Decimal::from(5),
            to: Decimal::from(5),
            step: Decimal::ONE,
        };

        let points = grid_sweep(
            &slices,
            &default_actions,
            &range,
            0,
            Decimal::from(2), // 5 * 2 = 10
            &RevenueMetric,
        );

        assert_eq!(points.len(), 1);
        // value = 5 * 2 = 10, baseline = 10, modified = 10 → 0%
        assert_eq!(points[0].impact.percent_change, Decimal::ZERO);
    }

    #[test]
    fn grid_sweep_per_slice_actions() {
        let slices = vec![SweepSlice {
            label: "Custom".into(),
            items: make_items(&[(10, 1)]),
            actions: Some(vec![make_action(15)]),
        }];
        let default_actions = vec![make_action(99)]; // should not be used
        let range = SweepRange {
            from: Decimal::from(12),
            to: Decimal::from(12),
            step: Decimal::ONE,
        };

        let points = grid_sweep(
            &slices,
            &default_actions,
            &range,
            0,
            Decimal::ONE,
            &RevenueMetric,
        );

        assert_eq!(points.len(), 1);
        // baseline = 10, modified = 12 → +20%
        assert_eq!(points[0].impact.percent_change, Decimal::from(20));
    }

    #[test]
    fn grid_sweep_empty_slices() {
        let range = SweepRange {
            from: Decimal::ZERO,
            to: Decimal::from(10),
            step: Decimal::ONE,
        };
        let points =
            grid_sweep::<Item, Action, _>(&[], &[], &range, 0, Decimal::ONE, &RevenueMetric);
        assert!(points.is_empty());
    }

    // ─── Label Normalization Tests ───────────────────────────────────

    #[test]
    fn normalize_label_strips_trailing_zeros() {
        assert_eq!(normalize_label(Decimal::new(150, 2)), "1.5");
        assert_eq!(normalize_label(Decimal::new(200, 2)), "2");
        assert_eq!(normalize_label(Decimal::from(10)), "10");
    }

    #[test]
    fn normalize_label_negative() {
        assert_eq!(normalize_label(Decimal::from(-5)), "-5");
    }
}
