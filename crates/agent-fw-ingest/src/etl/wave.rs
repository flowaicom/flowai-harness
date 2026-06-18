//! Wave-parallel loading — topological sort of FK dependency graph.
//!
//! Computes loading waves from foreign key edges: tables with no FK
//! dependencies are wave 0 (can load in parallel), tables that depend
//! only on wave-0 tables are wave 1, etc. Fact tables (many FKs) end
//! up in later waves.
//!
//! # Law — Dependency ordering
//!
//! ```text
//! ∀ edge (a → b) ∈ fk_edges:
//!     wave_of(a) > wave_of(b)
//! ```
//!
//! # Law — Minimality
//!
//! ```text
//! wave_of(t) = 0                         if t has no FK dependencies
//! wave_of(t) = 1 + max(wave_of(dep))     otherwise
//! ```

use std::collections::{HashMap, HashSet};

use agent_fw_catalog::ForeignKeyEdge;

/// A loading wave: all tables in this wave can be loaded in parallel
/// because their FK dependencies are satisfied by earlier waves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadingWave {
    /// Zero-based wave index.
    pub index: usize,
    /// Tables to load in this wave (sorted for determinism).
    pub tables: Vec<String>,
}

/// Result of wave computation.
#[derive(Debug, Clone)]
pub struct WavePlan {
    /// Waves in loading order (wave 0 first).
    pub waves: Vec<LoadingWave>,
    /// Tables that participate in FK cycles (cannot be wave-ordered).
    /// These need special handling (e.g., deferred FK constraints).
    pub cyclic_tables: Vec<String>,
    /// O(1) lookup: table name → wave index.
    wave_index: HashMap<String, usize>,
}

impl WavePlan {
    /// Total number of waves.
    pub fn wave_count(&self) -> usize {
        self.waves.len()
    }

    /// Lookup which wave a table belongs to. O(1).
    pub fn wave_of(&self, table: &str) -> Option<usize> {
        self.wave_index.get(table).copied()
    }

    /// All tables in loading order (dimensions first, facts last).
    pub fn tables_in_order(&self) -> Vec<&str> {
        self.waves
            .iter()
            .flat_map(|w| w.tables.iter().map(String::as_str))
            .collect()
    }
}

/// Compute loading waves from FK edges.
///
/// Pure function — topological layering of the FK dependency graph.
///
/// Each FK edge `(source_table → target_table)` means `source_table`
/// depends on `target_table` (it references the target's PK). So
/// `target_table` must be loaded before `source_table`.
///
/// Tables not mentioned in any FK edge are placed in wave 0.
pub fn compute_loading_waves(fk_edges: &[ForeignKeyEdge], all_tables: &[String]) -> WavePlan {
    // Build dependency graph: table → set of tables it depends on
    let mut dependencies: HashMap<&str, HashSet<&str>> = HashMap::new();
    let table_set: HashSet<&str> = all_tables.iter().map(String::as_str).collect();

    // Initialize all tables with empty dependency sets
    for t in &table_set {
        dependencies.entry(t).or_default();
    }

    // Add FK edges (source depends on target)
    for edge in fk_edges {
        if table_set.contains(edge.source_table.as_str())
            && table_set.contains(edge.target_table.as_str())
            && edge.source_table != edge.target_table
        {
            dependencies
                .entry(edge.source_table.as_str())
                .or_default()
                .insert(edge.target_table.as_str());
        }
    }

    // Kahn's algorithm — topological layering
    let mut waves = Vec::new();
    let mut wave_index = HashMap::new();
    let mut remaining: HashSet<&str> = table_set.clone();
    let mut assigned: HashSet<&str> = HashSet::new();

    loop {
        // Find all tables whose dependencies are fully satisfied
        let mut ready: Vec<&str> = remaining
            .iter()
            .filter(|&&t| {
                dependencies
                    .get(t)
                    .map(|deps| deps.iter().all(|d| assigned.contains(d)))
                    .unwrap_or(true)
            })
            .copied()
            .collect();

        if ready.is_empty() {
            break;
        }

        ready.sort(); // determinism
        let wave_idx = waves.len();

        for &t in &ready {
            wave_index.insert(t.to_string(), wave_idx);
        }

        let wave = LoadingWave {
            index: wave_idx,
            tables: ready.iter().map(|s| s.to_string()).collect(),
        };

        for t in &ready {
            remaining.remove(t);
            assigned.insert(t);
        }

        waves.push(wave);
    }

    // Any remaining tables are in cycles
    let mut cyclic_tables: Vec<String> = remaining.iter().map(|s| s.to_string()).collect();
    cyclic_tables.sort();

    WavePlan {
        waves,
        cyclic_tables,
        wave_index,
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(src: &str, src_col: &str, tgt: &str, tgt_col: &str) -> ForeignKeyEdge {
        ForeignKeyEdge {
            source_table: src.into(),
            source_column: src_col.into(),
            target_table: tgt.into(),
            target_column: tgt_col.into(),
        }
    }

    #[test]
    fn no_edges_single_wave() {
        let tables = vec!["a".into(), "b".into(), "c".into()];
        let plan = compute_loading_waves(&[], &tables);
        assert_eq!(plan.wave_count(), 1);
        assert_eq!(plan.waves[0].tables, vec!["a", "b", "c"]);
        assert!(plan.cyclic_tables.is_empty());
    }

    #[test]
    fn linear_chain() {
        // facts → orders → customers (facts depends on orders, orders depends on customers)
        let edges = vec![
            edge("facts", "order_id", "orders", "id"),
            edge("orders", "customer_id", "customers", "id"),
        ];
        let tables = vec!["facts".into(), "orders".into(), "customers".into()];
        let plan = compute_loading_waves(&edges, &tables);

        assert_eq!(plan.wave_count(), 3);
        assert_eq!(plan.wave_of("customers"), Some(0));
        assert_eq!(plan.wave_of("orders"), Some(1));
        assert_eq!(plan.wave_of("facts"), Some(2));
        assert!(plan.cyclic_tables.is_empty());
    }

    #[test]
    fn star_schema() {
        // fact_sales → dim_product, dim_customer, dim_time
        let edges = vec![
            edge("fact_sales", "product_id", "dim_product", "id"),
            edge("fact_sales", "customer_id", "dim_customer", "id"),
            edge("fact_sales", "time_id", "dim_time", "id"),
        ];
        let tables = vec![
            "fact_sales".into(),
            "dim_product".into(),
            "dim_customer".into(),
            "dim_time".into(),
        ];
        let plan = compute_loading_waves(&edges, &tables);

        assert_eq!(plan.wave_count(), 2);
        // All dims in wave 0, fact in wave 1
        assert_eq!(plan.wave_of("dim_product"), Some(0));
        assert_eq!(plan.wave_of("dim_customer"), Some(0));
        assert_eq!(plan.wave_of("dim_time"), Some(0));
        assert_eq!(plan.wave_of("fact_sales"), Some(1));
    }

    #[test]
    fn diamond_dependency() {
        // D depends on B and C, B and C depend on A
        let edges = vec![
            edge("D", "b_id", "B", "id"),
            edge("D", "c_id", "C", "id"),
            edge("B", "a_id", "A", "id"),
            edge("C", "a_id", "A", "id"),
        ];
        let tables = vec!["A".into(), "B".into(), "C".into(), "D".into()];
        let plan = compute_loading_waves(&edges, &tables);

        assert_eq!(plan.wave_count(), 3);
        assert_eq!(plan.wave_of("A"), Some(0));
        assert_eq!(plan.wave_of("B"), Some(1));
        assert_eq!(plan.wave_of("C"), Some(1));
        assert_eq!(plan.wave_of("D"), Some(2));
    }

    #[test]
    fn cycle_detection() {
        // A → B → A (cycle)
        let edges = vec![edge("A", "b_id", "B", "id"), edge("B", "a_id", "A", "id")];
        let tables = vec!["A".into(), "B".into(), "C".into()];
        let plan = compute_loading_waves(&edges, &tables);

        // C has no deps → wave 0
        assert_eq!(plan.wave_of("C"), Some(0));
        // A and B are cyclic
        assert_eq!(plan.cyclic_tables, vec!["A", "B"]);
    }

    #[test]
    fn self_reference_ignored() {
        let edges = vec![edge("tree", "parent_id", "tree", "id")];
        let tables = vec!["tree".into()];
        let plan = compute_loading_waves(&edges, &tables);

        assert_eq!(plan.wave_count(), 1);
        assert_eq!(plan.wave_of("tree"), Some(0));
        assert!(plan.cyclic_tables.is_empty());
    }

    #[test]
    fn tables_in_order_correctness() {
        let edges = vec![edge("B", "a_id", "A", "id")];
        let tables = vec!["A".into(), "B".into()];
        let plan = compute_loading_waves(&edges, &tables);

        let order = plan.tables_in_order();
        assert_eq!(order, vec!["A", "B"]);
    }

    #[test]
    fn law_dependency_ordering() {
        // For any FK edge (a → b), wave_of(a) > wave_of(b)
        let edges = vec![
            edge("fact", "dim1_id", "dim1", "id"),
            edge("fact", "dim2_id", "dim2", "id"),
            edge("dim2", "dim3_id", "dim3", "id"),
        ];
        let tables = vec!["fact".into(), "dim1".into(), "dim2".into(), "dim3".into()];
        let plan = compute_loading_waves(&edges, &tables);

        for e in &edges {
            let src_wave = plan.wave_of(&e.source_table).unwrap();
            let tgt_wave = plan.wave_of(&e.target_table).unwrap();
            assert!(
                src_wave > tgt_wave,
                "Dependency ordering violated: {} (wave {}) should be after {} (wave {})",
                e.source_table,
                src_wave,
                e.target_table,
                tgt_wave
            );
        }
    }

    #[test]
    fn wave_of_is_consistent_with_waves() {
        let edges = vec![
            edge("fact", "dim_id", "dim", "id"),
            edge("dim", "base_id", "base", "id"),
        ];
        let tables = vec!["fact".into(), "dim".into(), "base".into()];
        let plan = compute_loading_waves(&edges, &tables);

        // wave_of lookup must match wave.tables membership
        for wave in &plan.waves {
            for t in &wave.tables {
                assert_eq!(
                    plan.wave_of(t),
                    Some(wave.index),
                    "wave_of({}) should match its wave index",
                    t
                );
            }
        }
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use hegel::generators;

    fn draw_unique_tables(tc: &hegel::TestCase, max: usize) -> Vec<String> {
        let tables: std::collections::HashSet<String> = tc.draw(
            generators::hashsets(generators::from_regex(r"[a-z]{1,6}").fullmatch(true))
                .min_size(1)
                .max_size(max),
        );
        tables.into_iter().collect()
    }

    /// Law: ∀ non-cyclic edge (a -> b): wave_of(a) > wave_of(b)
    #[hegel::test]
    fn law_dependency_ordering_universal(tc: hegel::TestCase) {
        let tables = draw_unique_tables(&tc, 8);
        let n = tables.len();
        let plan_tables = tables.clone();

        // Create some random acyclic edges (just sequential deps to avoid cycles)
        let mut edges = vec![];
        for i in 1..n {
            edges.push(ForeignKeyEdge {
                source_table: tables[i].clone(),
                source_column: "fk_id".into(),
                target_table: tables[i - 1].clone(),
                target_column: "id".into(),
            });
        }

        let plan = compute_loading_waves(&edges, &plan_tables);
        assert!(
            plan.cyclic_tables.is_empty(),
            "Sequential chain should have no cycles"
        );

        for e in &edges {
            if let (Some(s), Some(t)) =
                (plan.wave_of(&e.source_table), plan.wave_of(&e.target_table))
            {
                assert!(
                    s > t,
                    "Dependency ordering violated: {} (wave {}) > {} (wave {})",
                    e.source_table,
                    s,
                    e.target_table,
                    t
                );
            }
        }
    }

    /// Law: wave_of lookup is consistent with wave membership
    #[hegel::test]
    fn law_wave_of_consistency(tc: hegel::TestCase) {
        let tables = draw_unique_tables(&tc, 10);
        let plan = compute_loading_waves(&[], &tables);
        for wave in &plan.waves {
            for t in &wave.tables {
                assert_eq!(plan.wave_of(t), Some(wave.index),);
            }
        }
    }

    /// Determinism: same inputs produce same output
    #[hegel::test]
    fn determinism(tc: hegel::TestCase) {
        let tables = draw_unique_tables(&tc, 8);
        let plan1 = compute_loading_waves(&[], &tables);
        let plan2 = compute_loading_waves(&[], &tables);
        assert_eq!(plan1.waves, plan2.waves);
        assert_eq!(plan1.cyclic_tables, plan2.cyclic_tables);
    }
}
