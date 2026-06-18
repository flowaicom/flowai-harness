//! Monotonically sequenced event wrapper (functor).
//!
//! A pure data type (no async, no IO) that wraps any event `E` with a
//! monotonic sequence number. Lives in Layer 0 so any crate can use it
//! without pulling in tokio/dashmap.
//!
//! # Laws
//!
//! - **S1 (Monotonicity)**: `emit(a); emit(b)` implies `a.seq < b.seq`
//! - **S2 (Uniqueness)**: no two events share the same `seq` within a bus
//! - **S3 (Functor)**: `map(f, Sequenced { seq, event }) = Sequenced { seq, event: f(event) }`
//!
//! S1 and S2 are enforced by the emitting bus (e.g. `SequencedBus<E>`).
//! S3 is a structural property of `map`.

use serde::{Deserialize, Serialize};

/// Monotonically sequenced event wrapper.
///
/// Functor: `Sequenced<E>` wraps any `E` with a monotonic sequence number,
/// enabling stateless dedup. The sequence is unique per event bus instance
/// and strictly increasing (no gaps under normal operation; gaps possible
/// after lagged-receiver recovery).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sequenced<E> {
    /// Monotonic sequence number (0-based, unique per bus instance).
    pub seq: u64,
    /// The wrapped event payload.
    pub event: E,
}

impl<E> Sequenced<E> {
    /// Map over the inner event (functor law S3).
    pub fn map<F, B>(self, f: F) -> Sequenced<B>
    where
        F: FnOnce(E) -> B,
    {
        Sequenced {
            seq: self.seq,
            event: f(self.event),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn functor_preserves_seq() {
        let s = Sequenced {
            seq: 42,
            event: "hello",
        };
        let mapped = s.map(|e| e.len());
        assert_eq!(mapped.seq, 42);
        assert_eq!(mapped.event, 5);
    }

    #[test]
    fn functor_identity() {
        let s = Sequenced { seq: 7, event: 99 };
        let mapped = s.clone().map(|e| e);
        assert_eq!(s, mapped);
    }

    #[test]
    fn functor_composition() {
        let s = Sequenced { seq: 3, event: 10 };
        let f = |x: i32| x + 1;
        let g = |x: i32| x * 2;

        let composed = s.clone().map(|e| g(f(e)));
        let chained = s.map(f).map(g);
        assert_eq!(composed, chained);
    }

    #[test]
    fn serde_roundtrip() {
        let s = Sequenced {
            seq: 100,
            event: "test".to_string(),
        };
        let json = serde_json::to_string(&s).unwrap();
        let deserialized: Sequenced<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(s, deserialized);
    }
}
