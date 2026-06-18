//! runtime query assembly C4: `RuntimeSpec` JSON round-trip parity.
//!
//! The fixture at `tests/fixtures/runtime_spec.json` is the canonical
//! shape the Python (Python adapter) and TypeScript `flowai-harness` libraries
//! will use for cross-language parity tests. Keeping this file in lockstep
//! with the spec struct is how we catch silent schema drift.

use flowai_runtime::RuntimeSpec;

const FIXTURE: &str = include_str!("fixtures/runtime_spec.json");

#[test]
fn fixture_parses_and_round_trips_through_runtime_spec() {
    let parsed: RuntimeSpec = serde_json::from_str(FIXTURE).expect("fixture parses");
    let reserialized =
        serde_json::to_value(&parsed).expect("RuntimeSpec re-serialises to JSON value");
    let fixture_value: serde_json::Value =
        serde_json::from_str(FIXTURE).expect("fixture is valid JSON");

    assert_eq!(
        reserialized, fixture_value,
        "RuntimeSpec must round-trip the fixture byte-for-byte (modulo key order)",
    );
}

#[test]
fn fixture_uses_camel_case_top_level_keys() {
    // The library facades (Pydantic / Zod) serialise to camelCase; the Rust
    // crate must accept that shape unchanged so SDK-emitted specs deserialise
    // here without conversion.
    let value: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture is JSON");
    let obj = value.as_object().expect("top-level object");
    assert!(obj.contains_key("approvalPolicies"));
    assert!(obj.contains_key("storageFactories"));
    assert!(!obj.contains_key("approval_policies"));
    assert!(!obj.contains_key("storage_factories"));
}
