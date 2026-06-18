//! Proptest law harness for `PlanContext` typed accessors.
//!
//! # Laws
//!
//! - **R1 (Roundtrip)**: `insert(k, &v); extract::<T>(k) == Ok(v)` for i64, String, Vec<String>
//! - **R2 (MissingKey)**: `extract::<T>(absent_key)` returns `Err(MissingKey)`
//! - **R3 (TypeMismatch)**: `set(k, json!("str")); extract::<i64>(k)` returns `Err(TypeMismatch)`
//! - **R4 (Purity)**: same context, same key → same result
//! - **R5 (Insert commutativity)**: different-key insert order is irrelevant for extract
//! - **R6 (Last-write-wins)**: same key, second value wins

use agent_fw_plan::context::ContextError;
use agent_fw_plan::PlanContext;
use hegel::generators;

/// R1 (Roundtrip — i64): `insert` then `extract` for i64.
pub fn r1_i64_roundtrip() {
    hegel::Hegel::new(|tc: hegel::TestCase| {
        let val = tc.draw(
            generators::integers::<i64>()
                .min_value(-10000)
                .max_value(9999),
        );
        let mut ctx = PlanContext::new();
        ctx.insert("key", &val).unwrap();
        let extracted: i64 = ctx.extract("key").unwrap();
        assert_eq!(extracted, val, "R1: i64 roundtrip");
    })
    .settings(hegel::Settings::new().test_cases(200))
    .run();
}

/// R1 (Roundtrip — String): `insert` then `extract` for String.
pub fn r1_string_roundtrip() {
    hegel::Hegel::new(|tc: hegel::TestCase| {
        let val: String = tc.draw(generators::text().max_size(50));
        let mut ctx = PlanContext::new();
        ctx.insert("key", &val).unwrap();
        let extracted: String = ctx.extract("key").unwrap();
        assert_eq!(extracted, val, "R1: String roundtrip");
    })
    .settings(hegel::Settings::new().test_cases(200))
    .run();
}

/// R1 (Roundtrip — Vec<String>): `insert` then `extract` for Vec<String>.
pub fn r1_vec_roundtrip() {
    hegel::Hegel::new(|tc: hegel::TestCase| {
        let vals: Vec<String> =
            tc.draw(generators::vecs(generators::text().min_size(1).max_size(10)).max_size(4));
        let mut ctx = PlanContext::new();
        ctx.insert("tags", &vals).unwrap();
        let extracted: Vec<String> = ctx.extract("tags").unwrap();
        assert_eq!(extracted, vals, "R1: Vec<String> roundtrip");
    })
    .run();
}

/// R2 (MissingKey): extracting an absent key returns `MissingKey`.
pub fn r2_missing_key() {
    hegel::Hegel::new(|tc: hegel::TestCase| {
        let key: String = tc.draw(generators::text().min_size(1).max_size(20));
        let ctx = PlanContext::new();
        let result = ctx.extract::<String>(&key);
        assert!(
            matches!(result, Err(ContextError::MissingKey { .. })),
            "R2: expected MissingKey, got {:?}",
            result,
        );
    })
    .run();
}

/// R3 (TypeMismatch): string value extracted as i64 yields `TypeMismatch`.
pub fn r3_type_mismatch() {
    hegel::Hegel::new(|tc: hegel::TestCase| {
        let val: String = tc.draw(generators::text().min_size(1).max_size(20));
        let mut ctx = PlanContext::new();
        ctx.set("key", serde_json::json!(val));
        let result = ctx.extract::<i64>("key");
        assert!(
            matches!(result, Err(ContextError::TypeMismatch { .. })),
            "R3: expected TypeMismatch, got {:?}",
            result,
        );
    })
    .run();
}

/// R4 (Purity): same context, same key → same result.
pub fn r4_purity() {
    hegel::Hegel::new(|tc: hegel::TestCase| {
        let val = tc.draw(
            generators::integers::<i64>()
                .min_value(-10000)
                .max_value(9999),
        );
        let mut ctx = PlanContext::new();
        ctx.insert("key", &val).unwrap();
        let a: i64 = ctx.extract("key").unwrap();
        let b: i64 = ctx.extract("key").unwrap();
        assert_eq!(a, b, "R4: purity");
    })
    .settings(hegel::Settings::new().test_cases(200))
    .run();
}

/// R5 (Insert commutativity): different-key insert order is irrelevant.
pub fn r5_insert_commutativity() {
    hegel::Hegel::new(|tc: hegel::TestCase| {
        let int_val = tc.draw(
            generators::integers::<i64>()
                .min_value(-10000)
                .max_value(9999),
        );
        let str_val: String = tc.draw(generators::text().min_size(1).max_size(20));

        // Order A: int first, str second
        let mut ctx_a = PlanContext::new();
        ctx_a.insert("int", &int_val).unwrap();
        ctx_a.insert("str", &str_val).unwrap();

        // Order B: str first, int second
        let mut ctx_b = PlanContext::new();
        ctx_b.insert("str", &str_val).unwrap();
        ctx_b.insert("int", &int_val).unwrap();

        let a_int: i64 = ctx_a.extract("int").unwrap();
        let b_int: i64 = ctx_b.extract("int").unwrap();
        let a_str: String = ctx_a.extract("str").unwrap();
        let b_str: String = ctx_b.extract("str").unwrap();

        assert_eq!(a_int, b_int, "R5: int key commutes");
        assert_eq!(a_str, b_str, "R5: str key commutes");
    })
    .settings(hegel::Settings::new().test_cases(200))
    .run();
}

/// R6 (Last-write-wins): same key, second value wins.
pub fn r6_last_write_wins() {
    hegel::Hegel::new(|tc: hegel::TestCase| {
        let first = tc.draw(
            generators::integers::<i64>()
                .min_value(-10000)
                .max_value(9999),
        );
        let second = tc.draw(
            generators::integers::<i64>()
                .min_value(-10000)
                .max_value(9999),
        );
        let mut ctx = PlanContext::new();
        ctx.insert("key", &first).unwrap();
        ctx.insert("key", &second).unwrap();
        let extracted: i64 = ctx.extract("key").unwrap();
        assert_eq!(extracted, second, "R6: last write wins");
    })
    .settings(hegel::Settings::new().test_cases(200))
    .run();
}

/// Run all plan context laws.
pub fn test_all() {
    r1_i64_roundtrip();
    r1_string_roundtrip();
    r1_vec_roundtrip();
    r2_missing_key();
    r3_type_mismatch();
    r4_purity();
    r5_insert_commutativity();
    r6_last_write_wins();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_context_laws() {
        test_all();
    }
}
