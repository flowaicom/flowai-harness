//! Compact summary of a resolved entity set.
//!
//! Designed for LLM context efficiency: provides key information about an
//! entity set without including all the data.
//!
//! # Laws
//!
//! - **Faithfulness**: glimpse represents the entity's key characteristics.
//! - **Boundedness**: `sample_labels.len() <= MAX_SAMPLE` (default 5).

use serde::{Deserialize, Serialize};

/// Default maximum number of sample labels in a glimpse.
pub const MAX_SAMPLE: usize = 5;

/// Compact summary of a resolved entity set.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Glimpse {
    /// Total number of entities in the set.
    pub total_count: usize,
    /// Sample of entity labels (up to MAX_SAMPLE).
    pub sample_labels: Vec<String>,
    /// Facet distributions (e.g., category breakdown).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facets: Vec<GlimpseFacet>,
    /// Domain-specific extension data (structured samples, distributions, etc.).
    /// Opaque to the framework; applications define the shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<serde_json::Value>,
}

/// A single facet distribution in a glimpse.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GlimpseFacet {
    /// Facet name (e.g., "product_type", "region").
    pub name: String,
    /// Distribution: (value, count) pairs.
    pub distribution: Vec<(String, usize)>,
}

impl Glimpse {
    /// Create an empty glimpse (no entities matched).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Create a glimpse from a total count and sample labels.
    ///
    /// Labels are truncated to `MAX_SAMPLE` entries.
    pub fn from_labels(total: usize, labels: Vec<String>) -> Self {
        let sample_labels = if labels.len() > MAX_SAMPLE {
            labels.into_iter().take(MAX_SAMPLE).collect()
        } else {
            labels
        };
        Self {
            total_count: total,
            sample_labels,
            facets: Vec::new(),
            extensions: None,
        }
    }

    /// Add domain-specific extension data (builder pattern).
    pub fn with_extensions(mut self, ext: serde_json::Value) -> Self {
        self.extensions = Some(ext);
        self
    }

    /// Add a facet distribution (builder pattern).
    pub fn with_facet(mut self, name: &str, dist: Vec<(String, usize)>) -> Self {
        self.facets.push(GlimpseFacet {
            name: name.to_string(),
            distribution: dist,
        });
        self
    }

    /// Monoidal fold over a collection of glimpses.
    ///
    /// # Laws
    /// - L1 (Homomorphism): `concat(xs) == xs.fold(empty(), merge)`
    /// - L2 (Singleton):    `concat([g]) == g`
    /// - L3 (Empty):        `concat([]) == Glimpse::empty()`
    pub fn concat(glimpses: impl IntoIterator<Item = Glimpse>) -> Glimpse {
        glimpses.into_iter().fold(Glimpse::empty(), Glimpse::merge)
    }

    /// Monoidal merge: combine two Glimpses.
    ///
    /// # Laws
    ///
    /// - **Identity**: `g.merge(Glimpse::empty()) == g == Glimpse::empty().merge(g)`
    /// - **Associativity**: `a.merge(b).merge(c) == a.merge(b.merge(c))`
    ///   (holds for `total_count` and `sample_labels`; facet distribution
    ///    order may differ but values are equivalent)
    ///
    /// # Merge semantics
    ///
    /// - `total_count`: sum
    /// - `sample_labels`: concatenate, truncate to `MAX_SAMPLE`
    /// - `facets`: union by name; matching facets sum their distributions
    /// - `extensions`: JSON object merge (right-biased on key conflicts)
    pub fn merge(self, other: Self) -> Self {
        let total_count = self.total_count + other.total_count;

        let mut sample_labels = self.sample_labels;
        sample_labels.extend(other.sample_labels);
        sample_labels.truncate(MAX_SAMPLE);

        let facets = merge_facets(self.facets, other.facets);

        let extensions = match (self.extensions, other.extensions) {
            (None, None) => None,
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (Some(serde_json::Value::Object(mut a)), Some(serde_json::Value::Object(b))) => {
                for (k, v) in b {
                    a.insert(k, v);
                }
                Some(serde_json::Value::Object(a))
            }
            (_, Some(b)) => Some(b), // non-object: prefer RHS
        };

        Self {
            total_count,
            sample_labels,
            facets,
            extensions,
        }
    }
}

/// Merge two facet lists by name.
///
/// Facets with the same name get their distributions merged (sum counts
/// for matching values, append new values). Facets unique to either side
/// are included as-is.
fn merge_facets(mut left: Vec<GlimpseFacet>, right: Vec<GlimpseFacet>) -> Vec<GlimpseFacet> {
    for rf in right {
        if let Some(lf) = left.iter_mut().find(|f| f.name == rf.name) {
            for (val, count) in rf.distribution {
                if let Some(entry) = lf.distribution.iter_mut().find(|(v, _)| *v == val) {
                    entry.1 += count;
                } else {
                    lf.distribution.push((val, count));
                }
            }
        } else {
            left.push(rf);
        }
    }
    left
}

impl PartialEq for Glimpse {
    fn eq(&self, other: &Self) -> bool {
        self.total_count == other.total_count
            && self.sample_labels == other.sample_labels
            && self.extensions == other.extensions
    }
}

impl Eq for Glimpse {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_glimpse() {
        let g = Glimpse::empty();
        assert_eq!(g.total_count, 0);
        assert!(g.sample_labels.is_empty());
        assert!(g.facets.is_empty());
    }

    #[test]
    fn from_labels_within_limit() {
        let labels = vec!["a".into(), "b".into(), "c".into()];
        let g = Glimpse::from_labels(100, labels);
        assert_eq!(g.total_count, 100);
        assert_eq!(g.sample_labels.len(), 3);
    }

    #[test]
    fn from_labels_truncates_to_max() {
        let labels: Vec<String> = (0..10).map(|i| format!("item-{i}")).collect();
        let g = Glimpse::from_labels(10, labels);
        assert_eq!(g.sample_labels.len(), MAX_SAMPLE);
    }

    #[test]
    fn with_facet_adds_facet() {
        let g = Glimpse::from_labels(5, vec!["x".into()])
            .with_facet("type", vec![("A".into(), 3), ("B".into(), 2)]);
        assert_eq!(g.facets.len(), 1);
        assert_eq!(g.facets[0].name, "type");
        assert_eq!(g.facets[0].distribution.len(), 2);
    }

    #[test]
    fn serde_roundtrip() {
        let g = Glimpse::from_labels(42, vec!["foo".into(), "bar".into()])
            .with_facet("cat", vec![("X".into(), 10)]);
        let json = serde_json::to_string(&g).unwrap();
        let parsed: Glimpse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.total_count, 42);
        assert_eq!(parsed.sample_labels, vec!["foo", "bar"]);
        assert_eq!(parsed.facets.len(), 1);
    }

    #[test]
    fn empty_facets_skipped_in_json() {
        let g = Glimpse::from_labels(1, vec!["a".into()]);
        let json = serde_json::to_string(&g).unwrap();
        assert!(!json.contains("facets"));
    }

    // ─── Merge (monoid) tests ────────────────────────────────────────

    #[test]
    fn merge_identity_right() {
        let g = Glimpse::from_labels(10, vec!["a".into(), "b".into()]);
        let merged = g.clone().merge(Glimpse::empty());
        assert_eq!(merged, g);
    }

    #[test]
    fn merge_identity_left() {
        let g = Glimpse::from_labels(10, vec!["a".into(), "b".into()]);
        let merged = Glimpse::empty().merge(g.clone());
        assert_eq!(merged, g);
    }

    #[test]
    fn merge_sums_total_count() {
        let a = Glimpse::from_labels(10, vec!["x".into()]);
        let b = Glimpse::from_labels(20, vec!["y".into()]);
        let merged = a.merge(b);
        assert_eq!(merged.total_count, 30);
    }

    #[test]
    fn merge_concatenates_and_truncates_labels() {
        let a = Glimpse::from_labels(3, vec!["a".into(), "b".into(), "c".into()]);
        let b = Glimpse::from_labels(3, vec!["d".into(), "e".into(), "f".into()]);
        let merged = a.merge(b);
        assert_eq!(merged.sample_labels.len(), MAX_SAMPLE);
        assert_eq!(merged.sample_labels[0], "a");
    }

    #[test]
    fn merge_facets_by_name() {
        let a = Glimpse::from_labels(5, vec![]).with_facet("type", vec![("A".into(), 3)]);
        let b = Glimpse::from_labels(5, vec![])
            .with_facet("type", vec![("A".into(), 2), ("B".into(), 1)]);
        let merged = a.merge(b);
        assert_eq!(merged.facets.len(), 1);
        let dist = &merged.facets[0].distribution;
        // A: 3 + 2 = 5
        assert_eq!(dist.iter().find(|(v, _)| v == "A").unwrap().1, 5);
        // B: 1 (new)
        assert_eq!(dist.iter().find(|(v, _)| v == "B").unwrap().1, 1);
    }

    #[test]
    fn merge_disjoint_facets() {
        let a = Glimpse::from_labels(5, vec![]).with_facet("type", vec![("A".into(), 3)]);
        let b = Glimpse::from_labels(5, vec![]).with_facet("region", vec![("US".into(), 5)]);
        let merged = a.merge(b);
        assert_eq!(merged.facets.len(), 2);
    }

    #[test]
    fn merge_extensions_object() {
        let a = Glimpse::from_labels(1, vec![]).with_extensions(serde_json::json!({"k1": "v1"}));
        let b = Glimpse::from_labels(1, vec![]).with_extensions(serde_json::json!({"k2": "v2"}));
        let merged = a.merge(b);
        let ext = merged.extensions.unwrap();
        assert_eq!(ext["k1"], "v1");
        assert_eq!(ext["k2"], "v2");
    }

    #[test]
    fn merge_extensions_right_bias() {
        let a = Glimpse::from_labels(1, vec![]).with_extensions(serde_json::json!({"k": "left"}));
        let b = Glimpse::from_labels(1, vec![]).with_extensions(serde_json::json!({"k": "right"}));
        let merged = a.merge(b);
        assert_eq!(merged.extensions.unwrap()["k"], "right");
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use hegel::generators;

    fn draw_glimpse(tc: &hegel::TestCase) -> Glimpse {
        let count: usize = tc.draw(generators::integers::<usize>().min_value(0).max_value(999));
        let labels: Vec<String> = tc.draw(
            generators::vecs(generators::from_regex("[a-z]{1,5}").fullmatch(true))
                .max_size(MAX_SAMPLE),
        );
        Glimpse::from_labels(count, labels)
    }

    /// Monoid identity (right): g.merge(empty) == g
    #[hegel::test]
    fn merge_right_identity(tc: hegel::TestCase) {
        let g = draw_glimpse(&tc);
        let merged = g.clone().merge(Glimpse::empty());
        assert_eq!(merged.total_count, g.total_count);
        assert_eq!(merged.sample_labels, g.sample_labels);
    }

    /// Monoid identity (left): empty.merge(g) == g
    #[hegel::test]
    fn merge_left_identity(tc: hegel::TestCase) {
        let g = draw_glimpse(&tc);
        let merged = Glimpse::empty().merge(g.clone());
        assert_eq!(merged.total_count, g.total_count);
        assert_eq!(merged.sample_labels, g.sample_labels);
    }

    /// Monoid associativity: (a.merge(b)).merge(c).total_count == a.merge(b.merge(c)).total_count
    #[hegel::test]
    fn merge_associativity_count(tc: hegel::TestCase) {
        let a = draw_glimpse(&tc);
        let b = draw_glimpse(&tc);
        let c = draw_glimpse(&tc);
        let left = a.clone().merge(b.clone()).merge(c.clone());
        let right = a.merge(b.merge(c));
        assert_eq!(left.total_count, right.total_count);
    }

    // ─── Glimpse::concat laws ──────────────────────────────────

    /// L1 (Homomorphism): concat(xs) == xs.fold(empty(), merge)
    #[hegel::test]
    fn concat_homomorphism(tc: hegel::TestCase) {
        let n: usize = tc.draw(generators::integers::<usize>().min_value(0).max_value(5));
        let gs: Vec<Glimpse> = (0..n).map(|_| draw_glimpse(&tc)).collect();
        let via_concat = Glimpse::concat(gs.clone());
        let via_fold = gs.into_iter().fold(Glimpse::empty(), Glimpse::merge);
        assert_eq!(via_concat.total_count, via_fold.total_count);
        assert_eq!(via_concat.sample_labels, via_fold.sample_labels);
    }

    /// L2 (Singleton): concat([g]) == g
    #[hegel::test]
    fn concat_singleton(tc: hegel::TestCase) {
        let g = draw_glimpse(&tc);
        let result = Glimpse::concat(vec![g.clone()]);
        assert_eq!(result.total_count, g.total_count);
        assert_eq!(result.sample_labels, g.sample_labels);
    }

    /// L3 (Empty): concat([]) == Glimpse::empty()
    #[hegel::test]
    fn concat_empty(_tc: hegel::TestCase) {
        let result = Glimpse::concat(Vec::<Glimpse>::new());
        assert_eq!(result.total_count, 0);
        assert!(result.sample_labels.is_empty());
    }
}
