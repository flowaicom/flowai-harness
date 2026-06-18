//! `NonEmpty<T>` — a Vec guaranteed to have at least one element.
//!
//! Makes `len == 0` unrepresentable by construction. The internal
//! representation is `head: T` + `tail: Vec<T>`, so `len >= 1` is
//! a structural invariant rather than a runtime check.
//!
//! # Serde
//!
//! Serializes as a JSON array. Deserialization rejects empty arrays
//! with a clear error message.

use std::num::NonZeroUsize;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A non-empty collection. `len() >= 1` by construction.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NonEmpty<T> {
    head: T,
    tail: Vec<T>,
}

impl<T> NonEmpty<T> {
    /// Infallible constructor: one head element, zero or more tail elements.
    pub fn new(head: T, tail: Vec<T>) -> Self {
        Self { head, tail }
    }

    /// Singleton (exactly one element).
    pub fn singleton(value: T) -> Self {
        Self {
            head: value,
            tail: Vec::new(),
        }
    }

    /// Try to construct from a `Vec`. Returns `None` if empty.
    pub fn from_vec(v: Vec<T>) -> Option<Self> {
        let mut iter = v.into_iter();
        let head = iter.next()?;
        let tail: Vec<T> = iter.collect();
        Some(Self { head, tail })
    }

    /// Number of elements (always >= 1).
    pub fn len(&self) -> NonZeroUsize {
        // Safety: 1 + tail.len() >= 1
        unsafe { NonZeroUsize::new_unchecked(1 + self.tail.len()) }
    }

    /// Reference to the first element (always exists).
    pub fn first(&self) -> &T {
        &self.head
    }

    /// Mutable reference to the first element.
    pub fn first_mut(&mut self) -> &mut T {
        &mut self.head
    }

    /// Return a contiguous slice of all elements.
    ///
    /// This allocates a temporary `Vec` on each call. For iteration,
    /// prefer `iter()` which is allocation-free.
    pub fn to_vec(&self) -> Vec<T>
    where
        T: Clone,
    {
        let mut v = Vec::with_capacity(1 + self.tail.len());
        v.push(self.head.clone());
        v.extend(self.tail.iter().cloned());
        v
    }

    /// Consume into a `Vec`.
    pub fn into_vec(self) -> Vec<T> {
        let mut v = Vec::with_capacity(1 + self.tail.len());
        v.push(self.head);
        v.extend(self.tail);
        v
    }

    /// Iterate over all elements.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        std::iter::once(&self.head).chain(self.tail.iter())
    }

    /// Push an element to the end.
    pub fn push(&mut self, value: T) {
        self.tail.push(value);
    }

    /// Map a function over all elements, preserving non-emptiness.
    pub fn map<U, F: FnMut(T) -> U>(self, mut f: F) -> NonEmpty<U> {
        NonEmpty {
            head: f(self.head),
            tail: self.tail.into_iter().map(f).collect(),
        }
    }

    /// Reference to the last element (always exists).
    pub fn last(&self) -> &T {
        self.tail.last().unwrap_or(&self.head)
    }

    /// Semigroup binary operation: concatenate two non-empty collections.
    ///
    /// # Law: Associativity
    ///
    /// ```text
    /// concat(concat(a, b), c).to_vec() == concat(a, concat(b, c)).to_vec()
    /// ```
    ///
    /// # Law: Length
    ///
    /// ```text
    /// concat(a, b).len() == a.len() + b.len()
    /// ```
    pub fn concat(mut self, other: NonEmpty<T>) -> NonEmpty<T> {
        self.tail.push(other.head);
        self.tail.extend(other.tail);
        self
    }
}

impl<T: Clone> NonEmpty<T> {
    /// Return a slice-like view. Allocates a Vec internally.
    #[deprecated(
        note = "Use iter() for allocation-free iteration or to_vec() for an explicit allocation. \
                as_slice() allocates despite its name because NonEmpty uses head+tail layout."
    )]
    pub fn as_slice(&self) -> Vec<T> {
        self.to_vec()
    }
}

impl<T> Extend<T> for NonEmpty<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        self.tail.extend(iter);
    }
}

impl<T> IntoIterator for NonEmpty<T> {
    type Item = T;
    type IntoIter = std::iter::Chain<std::iter::Once<T>, std::vec::IntoIter<T>>;

    fn into_iter(self) -> Self::IntoIter {
        std::iter::once(self.head).chain(self.tail.into_iter())
    }
}

impl<'a, T> IntoIterator for &'a NonEmpty<T> {
    type Item = &'a T;
    type IntoIter = std::iter::Chain<std::iter::Once<&'a T>, std::slice::Iter<'a, T>>;

    fn into_iter(self) -> Self::IntoIter {
        std::iter::once(&self.head).chain(self.tail.iter())
    }
}

// Serialize as a flat JSON array
impl<T: Serialize> Serialize for NonEmpty<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(1 + self.tail.len()))?;
        seq.serialize_element(&self.head)?;
        for item in &self.tail {
            seq.serialize_element(item)?;
        }
        seq.end()
    }
}

// Deserialize from a JSON array, rejecting empty arrays
impl<'de, T: Deserialize<'de>> Deserialize<'de> for NonEmpty<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let vec = Vec::<T>::deserialize(deserializer)?;
        NonEmpty::from_vec(vec).ok_or_else(|| {
            serde::de::Error::custom("NonEmpty requires at least one element, got empty array")
        })
    }
}

impl<T> std::ops::Index<usize> for NonEmpty<T> {
    type Output = T;

    fn index(&self, index: usize) -> &T {
        if index == 0 {
            &self.head
        } else {
            &self.tail[index - 1]
        }
    }
}

impl<T> From<NonEmpty<T>> for Vec<T> {
    fn from(ne: NonEmpty<T>) -> Vec<T> {
        ne.into_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_vec_empty_returns_none() {
        let result = NonEmpty::<i32>::from_vec(vec![]);
        assert!(result.is_none());
    }

    #[test]
    fn from_vec_nonempty() {
        let ne = NonEmpty::from_vec(vec![1, 2, 3]).unwrap();
        assert_eq!(ne.len().get(), 3);
        assert_eq!(ne.first(), &1);
        assert_eq!(ne.last(), &3);
    }

    #[test]
    fn singleton_first() {
        let ne = NonEmpty::singleton(42);
        assert_eq!(ne.first(), &42);
        assert_eq!(ne.last(), &42);
        assert_eq!(ne.len().get(), 1);
    }

    #[test]
    fn new_head_tail() {
        let ne = NonEmpty::new(10, vec![20, 30]);
        assert_eq!(ne.len().get(), 3);
        assert_eq!(ne.first(), &10);
        assert_eq!(ne.last(), &30);
    }

    #[test]
    fn iter_yields_all_elements() {
        let ne = NonEmpty::new(1, vec![2, 3]);
        let collected: Vec<_> = ne.iter().copied().collect();
        assert_eq!(collected, vec![1, 2, 3]);
    }

    #[test]
    fn into_iter_owned() {
        let ne = NonEmpty::new("a".to_string(), vec!["b".to_string()]);
        let collected: Vec<_> = ne.into_iter().collect();
        assert_eq!(collected, vec!["a", "b"]);
    }

    #[test]
    fn into_vec_roundtrip() {
        let ne = NonEmpty::new(1, vec![2, 3]);
        let v = ne.into_vec();
        assert_eq!(v, vec![1, 2, 3]);
    }

    #[test]
    fn to_vec_clones() {
        let ne = NonEmpty::new(1, vec![2]);
        let v = ne.to_vec();
        assert_eq!(v, vec![1, 2]);
        // Original still usable
        assert_eq!(ne.first(), &1);
    }

    #[test]
    fn push_extends() {
        let mut ne = NonEmpty::singleton(1);
        ne.push(2);
        ne.push(3);
        assert_eq!(ne.len().get(), 3);
    }

    #[test]
    fn extend_trait() {
        let mut ne = NonEmpty::singleton(1);
        ne.extend(vec![2, 3, 4]);
        assert_eq!(ne.len().get(), 4);
    }

    #[test]
    fn map_preserves_nonempty() {
        let ne = NonEmpty::new(1, vec![2, 3]);
        let mapped = ne.map(|x| x * 10);
        assert_eq!(mapped.first(), &10);
        assert_eq!(mapped.len().get(), 3);
    }

    // Serde tests

    #[test]
    fn serde_roundtrip() {
        let ne = NonEmpty::new(1, vec![2, 3]);
        let json = serde_json::to_string(&ne).unwrap();
        assert_eq!(json, "[1,2,3]");
        let parsed: NonEmpty<i32> = serde_json::from_str(&json).unwrap();
        assert_eq!(ne, parsed);
    }

    #[test]
    fn serde_singleton_roundtrip() {
        let ne = NonEmpty::singleton("hello".to_string());
        let json = serde_json::to_string(&ne).unwrap();
        assert_eq!(json, r#"["hello"]"#);
        let parsed: NonEmpty<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(ne, parsed);
    }

    #[test]
    fn serde_empty_array_fails() {
        let result = serde_json::from_str::<NonEmpty<i32>>("[]");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("at least one element"),
            "error should mention non-empty constraint: {err}"
        );
    }

    #[test]
    fn from_nonempty_into_vec() {
        let ne = NonEmpty::new(1, vec![2]);
        let v: Vec<i32> = ne.into();
        assert_eq!(v, vec![1, 2]);
    }

    // Concat (semigroup operation)

    #[test]
    fn concat_two_singletons() {
        let a = NonEmpty::singleton(1);
        let b = NonEmpty::singleton(2);
        let c = a.concat(b);
        assert_eq!(c.to_vec(), vec![1, 2]);
    }

    #[test]
    fn concat_preserves_order() {
        let a = NonEmpty::new(1, vec![2]);
        let b = NonEmpty::new(3, vec![4]);
        let c = a.concat(b);
        assert_eq!(c.to_vec(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn concat_length_additive() {
        let a = NonEmpty::new(1, vec![2, 3]);
        let b = NonEmpty::new(4, vec![5]);
        let len_a = a.len().get();
        let len_b = b.len().get();
        let c = a.concat(b);
        assert_eq!(c.len().get(), len_a + len_b);
    }

    #[test]
    fn concat_associativity() {
        let a = NonEmpty::new(1, vec![2]);
        let b = NonEmpty::singleton(3);
        let c = NonEmpty::new(4, vec![5]);

        let left = a.clone().concat(b.clone()).concat(c.clone());
        let right = a.concat(b.concat(c));
        assert_eq!(left.to_vec(), right.to_vec());
    }

    // =========================================================================
    // Property-Based Tests (Hegel)
    // =========================================================================

    use hegel::generators;

    fn draw_non_empty_i32(tc: &hegel::TestCase) -> NonEmpty<i32> {
        let head = tc.draw(generators::integers::<i32>());
        let tail: Vec<i32> = tc.draw(generators::vecs(generators::integers::<i32>()));
        NonEmpty::new(head, tail)
    }

    // --- Semigroup law: associativity ---

    #[hegel::test]
    fn concat_associativity_law(tc: hegel::TestCase) {
        let a = draw_non_empty_i32(&tc);
        let b = draw_non_empty_i32(&tc);
        let c = draw_non_empty_i32(&tc);
        let left = a.clone().concat(b.clone()).concat(c.clone());
        let right = a.concat(b.concat(c));
        assert_eq!(left.to_vec(), right.to_vec());
    }

    // --- Length additivity: |a ++ b| = |a| + |b| ---

    #[hegel::test]
    fn concat_length_additive_law(tc: hegel::TestCase) {
        let a = draw_non_empty_i32(&tc);
        let b = draw_non_empty_i32(&tc);
        let la = a.len().get();
        let lb = b.len().get();
        assert_eq!(a.concat(b).len().get(), la + lb);
    }

    // --- Structural invariant: len >= 1 always ---

    #[hegel::test]
    fn len_always_positive(tc: hegel::TestCase) {
        let ne = draw_non_empty_i32(&tc);
        assert!(ne.len().get() >= 1);
    }

    // --- first() always returns the head ---

    #[hegel::test]
    fn first_is_head(tc: hegel::TestCase) {
        let ne = draw_non_empty_i32(&tc);
        assert_eq!(ne.first(), &ne.to_vec()[0]);
    }

    // --- to_vec / from_vec roundtrip ---

    #[hegel::test]
    fn to_vec_from_vec_roundtrip(tc: hegel::TestCase) {
        let ne = draw_non_empty_i32(&tc);
        let v = ne.to_vec();
        let recovered = NonEmpty::from_vec(v).unwrap();
        assert_eq!(ne, recovered);
    }

    // --- Serde roundtrip ---

    #[hegel::test]
    fn serde_roundtrip_prop(tc: hegel::TestCase) {
        let ne = draw_non_empty_i32(&tc);
        let json = serde_json::to_string(&ne).unwrap();
        let parsed: NonEmpty<i32> = serde_json::from_str(&json).unwrap();
        assert_eq!(ne, parsed);
    }

    // --- map preserves length ---

    #[hegel::test]
    fn map_preserves_length(tc: hegel::TestCase) {
        let ne = draw_non_empty_i32(&tc);
        let original_len = ne.len();
        let mapped = ne.map(|x| x.wrapping_mul(2));
        assert_eq!(mapped.len(), original_len);
    }

    // --- concat preserves element order ---

    #[hegel::test]
    fn concat_preserves_element_order(tc: hegel::TestCase) {
        let a = draw_non_empty_i32(&tc);
        let b = draw_non_empty_i32(&tc);
        let av = a.to_vec();
        let bv = b.to_vec();
        let cv = a.concat(b).to_vec();
        // cv should be av ++ bv
        assert_eq!(&cv[..av.len()], &av[..]);
        assert_eq!(&cv[av.len()..], &bv[..]);
    }

    // --- from_vec(empty) is None ---

    #[hegel::test]
    fn from_vec_nonempty_is_some(tc: hegel::TestCase) {
        let v: Vec<i32> = tc.draw(generators::vecs(generators::integers::<i32>()));
        match NonEmpty::from_vec(v.clone()) {
            None => assert!(v.is_empty()),
            Some(ne) => {
                assert!(!v.is_empty());
                assert_eq!(ne.to_vec(), v);
            }
        }
    }

    // --- Index consistency ---

    #[hegel::test]
    fn index_agrees_with_to_vec(tc: hegel::TestCase) {
        let ne = draw_non_empty_i32(&tc);
        let v = ne.to_vec();
        let idx = tc.draw(generators::integers::<usize>().max_value(v.len() - 1));
        assert_eq!(ne[idx], v[idx]);
    }
}
