//! Generic action sequences and plan groups.
//!
//! # ActionSeq<A> — Non-Empty Action Sequence
//!
//! `ActionSeq<A>` is a newtype over [`NonEmpty<A>`], guaranteeing at least
//! one action via the type system. The structural invariant `len >= 1` is
//! enforced at compile time rather than via runtime assertions.
//!
//! ## Semigroup Structure
//!
//! `concat(a, b)` forms a semigroup (associativity holds):
//!
//! ```text
//! concat(concat(a, b), c) ≡ concat(a, concat(b, c))
//! ```
//!
//! Delegates to [`NonEmpty::concat`].
//!
//! There is no identity element (empty sequence is impossible by
//! construction), so this is a semigroup, not a monoid.
//!
//! ## Invariant
//!
//! **INV-1**: `seq.len() >= 1` — always holds by construction (via `NonEmpty`).
//!
//! ## Serde Compatibility
//!
//! Serializes as `{"head": A, "tail": [A...]}` (same wire format as the
//! original struct-based implementation) via custom Serialize/Deserialize.

use agent_fw_core::NonEmpty;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::context::PlanContext;

/// Non-empty action sequence. Newtype over [`NonEmpty<A>`].
///
/// Head is always present; tail may be empty. This guarantees that
/// every plan has at least one action.
///
/// ## Wire Format
///
/// Serializes as `{"head": A, "tail": [A...]}` for backward compatibility
/// with existing JSON data (e.g., persisted plans or FFI payloads).
#[derive(Clone, Debug)]
pub struct ActionSeq<A>(NonEmpty<A>);

impl<A> ActionSeq<A> {
    /// Total number of actions. Always >= 1.
    pub fn len(&self) -> usize {
        self.0.len().get()
    }

    /// Always returns false. Exists for API completeness (clippy).
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Reference to the first action.
    pub fn first(&self) -> &A {
        self.0.first()
    }

    /// Iterate over all actions in order.
    pub fn iter(&self) -> impl Iterator<Item = &A> {
        self.0.iter()
    }

    /// Transform each action, preserving the non-empty guarantee.
    ///
    /// This is the functor `map` for `ActionSeq`. The non-emptiness
    /// invariant is preserved at the type level throughout via `NonEmpty::map`.
    pub fn map<B>(&self, f: impl Fn(&A) -> B) -> ActionSeq<B> {
        // NonEmpty::map takes by value and FnMut(T) -> U.
        // We need by-ref map, so build manually.
        let head = f(self.0.first());
        let tail: Vec<B> = self.0.iter().skip(1).map(f).collect();
        ActionSeq(NonEmpty::new(head, tail))
    }

    /// Access the underlying `NonEmpty<A>`.
    pub fn as_non_empty(&self) -> &NonEmpty<A> {
        &self.0
    }

    /// Consume into the underlying `NonEmpty<A>`.
    pub fn into_non_empty(self) -> NonEmpty<A> {
        self.0
    }

    /// Construct from a `NonEmpty<A>`.
    pub fn from_non_empty(ne: NonEmpty<A>) -> Self {
        Self(ne)
    }
}

impl<A: Clone> ActionSeq<A> {
    /// Convert to a Vec, preserving order.
    pub fn to_vec(&self) -> Vec<A> {
        self.0.to_vec()
    }
}

impl<A: PartialEq> PartialEq for ActionSeq<A> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<A: Eq> Eq for ActionSeq<A> {}

// Serialize as {"head": A, "tail": [A...]} for backward compatibility
impl<A: Serialize> Serialize for ActionSeq<A> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("ActionSeq", 2)?;
        s.serialize_field("head", self.0.first())?;
        let tail: Vec<&A> = self.0.iter().skip(1).collect();
        s.serialize_field("tail", &tail)?;
        s.end()
    }
}

// Deserialize from {"head": A, "tail": [A...]}
impl<'de, A: Deserialize<'de>> Deserialize<'de> for ActionSeq<A> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{self, MapAccess, Visitor};
        use std::marker::PhantomData;

        struct ActionSeqVisitor<A>(PhantomData<A>);

        impl<'de, A: Deserialize<'de>> Visitor<'de> for ActionSeqVisitor<A> {
            type Value = ActionSeq<A>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an ActionSeq with head and tail fields")
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<ActionSeq<A>, M::Error> {
                let mut head: Option<A> = None;
                let mut tail: Option<Vec<A>> = None;

                // Use String keys (not &str) to support deserialization from
                // owned data like serde_json::Value (KV store roundtrip).
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "head" => head = Some(map.next_value()?),
                        "tail" => tail = Some(map.next_value()?),
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>()?;
                        }
                    }
                }

                let head = head.ok_or_else(|| de::Error::missing_field("head"))?;
                let tail = tail.unwrap_or_default();
                Ok(ActionSeq(NonEmpty::new(head, tail)))
            }
        }

        deserializer.deserialize_struct(
            "ActionSeq",
            &["head", "tail"],
            ActionSeqVisitor(PhantomData),
        )
    }
}

// ─── Smart Constructors ───────────────────────────────────────────────

/// Create a single-action sequence.
pub fn single_action<A>(a: A) -> ActionSeq<A> {
    ActionSeq(NonEmpty::singleton(a))
}

/// Create an `ActionSeq` from a non-empty vector.
///
/// Returns `None` if the vector is empty.
pub fn action_seq_from_vec<A>(actions: Vec<A>) -> Option<ActionSeq<A>> {
    NonEmpty::from_vec(actions).map(ActionSeq)
}

/// Append a single action to the end of a sequence.
pub fn append_action<A>(mut seq: ActionSeq<A>, a: A) -> ActionSeq<A> {
    seq.0.push(a);
    seq
}

/// Concatenate two action sequences (semigroup operation).
///
/// Delegates to [`NonEmpty::concat`].
///
/// # Law: Associativity
///
/// ```text
/// concat(concat(a, b), c) ≡ concat(a, concat(b, c))
/// ```
pub fn concat_actions<A>(a: ActionSeq<A>, b: ActionSeq<A>) -> ActionSeq<A> {
    ActionSeq(a.0.concat(b.0))
}

// ─── PlanGroup ────────────────────────────────────────────────────────

/// A group within a multi-group plan.
///
/// Each group targets a different set of entities with potentially
/// different actions. The `context` carries domain-specific targeting
/// information (e.g., entity filters, scope restrictions).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanGroup<A> {
    /// Human-readable label for this group.
    pub label: Option<String>,
    /// Actions to apply to the group's entities.
    pub actions: ActionSeq<A>,
    /// Domain-specific entity targeting context.
    pub context: PlanContext,
}

impl<A: PartialEq> PartialEq for PlanGroup<A> {
    fn eq(&self, other: &Self) -> bool {
        self.label == other.label && self.actions == other.actions && self.context == other.context
    }
}

impl<A: Eq> Eq for PlanGroup<A> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_action_has_len_one() {
        let seq = single_action(42);
        assert_eq!(seq.len(), 1);
        assert!(!seq.is_empty());
        assert_eq!(*seq.first(), 42);
    }

    #[test]
    fn from_vec_empty_returns_none() {
        let result = action_seq_from_vec::<i32>(vec![]);
        assert!(result.is_none());
    }

    #[test]
    fn from_vec_non_empty_returns_some() {
        let seq = action_seq_from_vec(vec![1, 2, 3]).unwrap();
        assert_eq!(seq.len(), 3);
        assert_eq!(*seq.first(), 1);
        assert_eq!(seq.to_vec(), vec![1, 2, 3]);
    }

    #[test]
    fn append_increases_len() {
        let seq = single_action(1);
        let seq = append_action(seq, 2);
        assert_eq!(seq.len(), 2);
        assert_eq!(seq.to_vec(), vec![1, 2]);
    }

    #[test]
    fn concat_combines_in_order() {
        let a = action_seq_from_vec(vec![1, 2]).unwrap();
        let b = action_seq_from_vec(vec![3, 4]).unwrap();
        let c = concat_actions(a, b);
        assert_eq!(c.len(), 4);
        assert_eq!(c.to_vec(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn to_vec_preserves_order() {
        let seq = action_seq_from_vec(vec![10, 20, 30]).unwrap();
        assert_eq!(seq.to_vec(), vec![10, 20, 30]);
    }

    #[test]
    fn iter_yields_all() {
        let seq = action_seq_from_vec(vec!["a", "b", "c"]).unwrap();
        let collected: Vec<_> = seq.iter().copied().collect();
        assert_eq!(collected, vec!["a", "b", "c"]);
    }

    #[test]
    fn map_preserves_non_empty() {
        let seq = single_action(1);
        let mapped = seq.map(|x| x * 2);
        assert_eq!(mapped.len(), 1);
        assert_eq!(*mapped.first(), 2);
    }

    #[test]
    fn map_transforms_all_elements() {
        let seq = action_seq_from_vec(vec![1, 2, 3]).unwrap();
        let mapped = seq.map(|x| x.to_string());
        assert_eq!(mapped.to_vec(), vec!["1", "2", "3"]);
        assert_eq!(mapped.len(), 3);
    }

    #[test]
    fn map_identity() {
        let seq = action_seq_from_vec(vec![10, 20, 30]).unwrap();
        let mapped = seq.map(|&x| x);
        assert_eq!(seq.to_vec(), mapped.to_vec());
    }

    #[test]
    fn plan_group_equality() {
        let g1 = PlanGroup {
            label: Some("g".into()),
            actions: single_action(1),
            context: PlanContext::new(),
        };
        let g2 = PlanGroup {
            label: Some("g".into()),
            actions: single_action(1),
            context: PlanContext::new(),
        };
        assert_eq!(g1, g2);
    }

    #[test]
    fn action_seq_serde_roundtrip() {
        let seq = action_seq_from_vec(vec![1, 2, 3]).unwrap();
        let json = serde_json::to_string(&seq).unwrap();
        let parsed: ActionSeq<i32> = serde_json::from_str(&json).unwrap();
        assert_eq!(seq, parsed);
    }

    #[test]
    fn action_seq_serde_format_backward_compat() {
        let seq = action_seq_from_vec(vec![1, 2, 3]).unwrap();
        let json = serde_json::to_string(&seq).unwrap();
        // Must serialize as {"head":1,"tail":[2,3]} (not as a flat array)
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["head"], 1);
        assert_eq!(v["tail"], serde_json::json!([2, 3]));
    }

    #[test]
    fn non_empty_roundtrip() {
        let ne = NonEmpty::new(1, vec![2, 3]);
        let seq = ActionSeq::from_non_empty(ne.clone());
        assert_eq!(seq.into_non_empty(), ne);
    }

    //=========================================================================
    // Property-Based Tests
    //=========================================================================

    use hegel::generators;

    #[hegel::test]
    fn non_empty_invariant(tc: hegel::TestCase) {
        let actions: Vec<i32> = tc.draw(
            generators::vecs(generators::integers::<i32>().min_value(0).max_value(99))
                .min_size(1)
                .max_size(19),
        );
        let seq = action_seq_from_vec(actions.clone()).unwrap();
        assert!(seq.len() >= 1, "INV-1: ActionSeq must be non-empty");
        assert_eq!(seq.len(), actions.len());
    }

    #[hegel::test]
    fn law_associativity(tc: hegel::TestCase) {
        let a: Vec<i32> = tc.draw(
            generators::vecs(generators::integers::<i32>().min_value(0).max_value(99))
                .min_size(1)
                .max_size(9),
        );
        let b: Vec<i32> = tc.draw(
            generators::vecs(generators::integers::<i32>().min_value(0).max_value(99))
                .min_size(1)
                .max_size(9),
        );
        let c: Vec<i32> = tc.draw(
            generators::vecs(generators::integers::<i32>().min_value(0).max_value(99))
                .min_size(1)
                .max_size(9),
        );

        let sa = action_seq_from_vec(a).unwrap();
        let sb = action_seq_from_vec(b).unwrap();
        let sc = action_seq_from_vec(c).unwrap();

        // concat(concat(a, b), c) == concat(a, concat(b, c))
        let left = concat_actions(concat_actions(sa.clone(), sb.clone()), sc.clone());
        let right = concat_actions(sa, concat_actions(sb, sc));
        assert_eq!(left.to_vec(), right.to_vec());
    }

    #[hegel::test]
    fn concat_length(tc: hegel::TestCase) {
        let a: Vec<i32> = tc.draw(
            generators::vecs(generators::integers::<i32>().min_value(0).max_value(99))
                .min_size(1)
                .max_size(9),
        );
        let b: Vec<i32> = tc.draw(
            generators::vecs(generators::integers::<i32>().min_value(0).max_value(99))
                .min_size(1)
                .max_size(9),
        );

        let sa = action_seq_from_vec(a.clone()).unwrap();
        let sb = action_seq_from_vec(b.clone()).unwrap();
        let combined = concat_actions(sa, sb);
        assert_eq!(combined.len(), a.len() + b.len());
    }
}
