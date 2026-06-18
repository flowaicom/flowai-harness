//! Set operations for combining stored sets.
//!
//! Provides sorted-array set algebra (union, intersect, diff) for combining
//! product sets and scope sets.

use serde::{Deserialize, Serialize};

/// Set operation for combining two sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SetOperation {
    Union,
    Intersect,
    Diff,
}

/// Union of two sorted slices (A ∪ B).
///
/// Returns a sorted, deduplicated vector containing all elements from both slices.
pub fn union_sorted<T: Ord + Clone>(a: &[T], b: &[T]) -> Vec<T> {
    let mut result = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);

    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => {
                result.push(a[i].clone());
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                result.push(b[j].clone());
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                result.push(a[i].clone());
                i += 1;
                j += 1;
            }
        }
    }

    result.extend_from_slice(&a[i..]);
    result.extend_from_slice(&b[j..]);
    result
}

/// Intersection of two sorted slices (A ∩ B).
///
/// Returns a sorted vector containing only elements present in both slices.
pub fn intersect_sorted<T: Ord + Clone>(a: &[T], b: &[T]) -> Vec<T> {
    let mut result = Vec::new();
    let (mut i, mut j) = (0, 0);

    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                result.push(a[i].clone());
                i += 1;
                j += 1;
            }
        }
    }

    result
}

/// Difference of two sorted slices (A \ B).
///
/// Returns a sorted vector containing elements in A that are not in B.
pub fn diff_sorted<T: Ord + Clone>(a: &[T], b: &[T]) -> Vec<T> {
    let mut result = Vec::new();
    let (mut i, mut j) = (0, 0);

    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => {
                result.push(a[i].clone());
                i += 1;
            }
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
        }
    }

    result.extend_from_slice(&a[i..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_basic() {
        assert_eq!(union_sorted(&[1, 3, 5], &[2, 3, 6]), vec![1, 2, 3, 5, 6]);
    }

    #[test]
    fn union_empty() {
        let empty: &[i32] = &[];
        assert_eq!(union_sorted(empty, &[1, 2]), vec![1, 2]);
        assert_eq!(union_sorted(&[1, 2], empty), vec![1, 2]);
    }

    #[test]
    fn intersect_basic() {
        assert_eq!(intersect_sorted(&[1, 2, 3, 4], &[2, 4, 5]), vec![2, 4]);
    }

    #[test]
    fn intersect_disjoint() {
        let result: Vec<i32> = intersect_sorted(&[1, 3], &[2, 4]);
        assert!(result.is_empty());
    }

    #[test]
    fn diff_basic() {
        assert_eq!(diff_sorted(&[1, 2, 3, 4], &[2, 4]), vec![1, 3]);
    }

    #[test]
    fn diff_no_overlap() {
        assert_eq!(diff_sorted(&[1, 3], &[2, 4]), vec![1, 3]);
    }

    #[test]
    fn diff_complete_overlap() {
        let result: Vec<i32> = diff_sorted(&[1, 2], &[1, 2, 3]);
        assert!(result.is_empty());
    }

    #[test]
    fn set_operation_serializes() {
        let op = SetOperation::Union;
        let json = serde_json::to_string(&op).unwrap();
        assert_eq!(json, "\"union\"");

        let parsed: SetOperation = serde_json::from_str("\"intersect\"").unwrap();
        assert_eq!(parsed, SetOperation::Intersect);
    }

    #[test]
    fn union_with_strings() {
        let a = vec!["apple".to_string(), "cherry".to_string()];
        let b = vec!["banana".to_string(), "cherry".to_string()];
        let result = union_sorted(&a, &b);
        assert_eq!(result, vec!["apple", "banana", "cherry"]);
    }
}
