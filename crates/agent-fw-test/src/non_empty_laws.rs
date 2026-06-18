//! NonEmpty<T> algebraic law test harnesses.
//!
//! # Laws
//!
//! - L1 (Roundtrip): `from_vec(ne.to_vec()) == Some(ne)`
//! - L2 (Length): `ne.len().get() == ne.to_vec().len()`
//! - L3 (Semigroup Associativity): `concat(concat(a,b),c).to_vec() == concat(a,concat(b,c)).to_vec()`
//! - L4 (Semigroup Length): `concat(a,b).len() == a.len() + b.len()`
//! - L5 (Serde Roundtrip): `deserialize(serialize(ne)) == ne`
//! - L6 (Empty Rejection): `from_str::<NonEmpty<i32>>("[]")` fails
//!
//! # Usage
//!
//! ```ignore
//! #[test]
//! fn non_empty_satisfies_laws() {
//!     agent_fw_test::non_empty_laws::test_all();
//! }
//! ```

use agent_fw_core::NonEmpty;

/// Run all NonEmpty laws.
pub fn test_all() {
    law_roundtrip();
    law_length();
    law_semigroup_associativity();
    law_semigroup_length();
    law_serde_roundtrip();
    law_empty_rejection();
    hegel_laws();
}

/// L1: from_vec(ne.to_vec()) == Some(ne)
pub fn law_roundtrip() {
    let ne = NonEmpty::new(1, vec![2, 3]);
    let roundtripped = NonEmpty::from_vec(ne.to_vec());
    assert_eq!(
        roundtripped,
        Some(ne),
        "L1: from_vec(to_vec()) must roundtrip"
    );
}

/// L2: ne.len().get() == ne.to_vec().len()
pub fn law_length() {
    let ne = NonEmpty::new(10, vec![20, 30, 40]);
    assert_eq!(
        ne.len().get(),
        ne.to_vec().len(),
        "L2: len().get() must equal to_vec().len()"
    );

    let singleton = NonEmpty::singleton(42);
    assert_eq!(singleton.len().get(), 1, "L2: singleton len is 1");
}

/// L3: concat(concat(a,b),c).to_vec() == concat(a,concat(b,c)).to_vec()
pub fn law_semigroup_associativity() {
    let a = NonEmpty::new(1, vec![2]);
    let b = NonEmpty::singleton(3);
    let c = NonEmpty::new(4, vec![5]);

    let left = a.clone().concat(b.clone()).concat(c.clone());
    let right = a.concat(b.concat(c));
    assert_eq!(
        left.to_vec(),
        right.to_vec(),
        "L3: concat must be associative"
    );
}

/// L4: concat(a,b).len() == a.len() + b.len()
pub fn law_semigroup_length() {
    let a = NonEmpty::new(1, vec![2, 3]);
    let b = NonEmpty::new(4, vec![5]);
    let len_a = a.len().get();
    let len_b = b.len().get();
    let combined = a.concat(b);
    assert_eq!(
        combined.len().get(),
        len_a + len_b,
        "L4: concat length must be additive"
    );
}

/// L5: deserialize(serialize(ne)) == ne
pub fn law_serde_roundtrip() {
    let ne = NonEmpty::new(1, vec![2, 3]);
    let json = serde_json::to_string(&ne).expect("serialize should succeed");
    let parsed: NonEmpty<i32> = serde_json::from_str(&json).expect("deserialize should succeed");
    assert_eq!(ne, parsed, "L5: serde roundtrip must preserve value");
}

/// L6: deserializing an empty array fails
pub fn law_empty_rejection() {
    let result = serde_json::from_str::<NonEmpty<i32>>("[]");
    assert!(
        result.is_err(),
        "L6: empty array must be rejected by NonEmpty deserializer"
    );
}

/// Property-based law verification via hegel.
fn hegel_laws() {
    use hegel::generators;

    // L1+L2: roundtrip + length
    hegel::Hegel::new(|tc: hegel::TestCase| {
        let values: Vec<i32> = tc.draw(
            generators::vecs(generators::integers::<i32>())
                .min_size(1)
                .max_size(19),
        );
        let ne = NonEmpty::from_vec(values).unwrap();
        // L1
        assert_eq!(
            NonEmpty::from_vec(ne.to_vec()),
            Some(ne.clone()),
            "L1 hegel: roundtrip"
        );
        // L2
        assert_eq!(ne.len().get(), ne.to_vec().len(), "L2 hegel: length");
    })
    .run();

    // L3+L4: semigroup associativity + length
    hegel::Hegel::new(|tc: hegel::TestCase| {
        let va: Vec<i32> = tc.draw(
            generators::vecs(generators::integers::<i32>())
                .min_size(1)
                .max_size(9),
        );
        let vb: Vec<i32> = tc.draw(
            generators::vecs(generators::integers::<i32>())
                .min_size(1)
                .max_size(9),
        );
        let vc: Vec<i32> = tc.draw(
            generators::vecs(generators::integers::<i32>())
                .min_size(1)
                .max_size(9),
        );
        let a = NonEmpty::from_vec(va).unwrap();
        let b = NonEmpty::from_vec(vb).unwrap();
        let c = NonEmpty::from_vec(vc).unwrap();

        // L3: associativity
        let left = a.clone().concat(b.clone()).concat(c.clone());
        let right = a.clone().concat(b.clone().concat(c.clone()));
        assert_eq!(left.to_vec(), right.to_vec(), "L3 hegel: associativity");

        // L4: length additivity
        let combined = a.clone().concat(b.clone());
        assert_eq!(
            combined.len().get(),
            a.len().get() + b.len().get(),
            "L4 hegel: length"
        );
    })
    .run();

    // L5: serde roundtrip
    hegel::Hegel::new(|tc: hegel::TestCase| {
        let values: Vec<i32> = tc.draw(
            generators::vecs(generators::integers::<i32>())
                .min_size(1)
                .max_size(19),
        );
        let ne = NonEmpty::from_vec(values).unwrap();
        let json = serde_json::to_string(&ne).unwrap();
        let parsed: NonEmpty<i32> = serde_json::from_str(&json).unwrap();
        assert_eq!(ne, parsed, "L5 hegel: serde roundtrip");
    })
    .run();
}
