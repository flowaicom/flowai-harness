//! Product attribute discovery and schema generation for dynamic star schemas.

use super::parquet_reader::ProductRow;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

/// Stable 64-bit hash (FNV-1a). Deterministic across Rust versions and platforms.
///
/// Used for content-addressed column name generation. These hashes are persisted
/// as database column names, so they must NOT depend on `DefaultHasher` (which
/// is explicitly not guaranteed stable across Rust releases).
#[inline]
fn stable_hash(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in s.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Discovered product attributes (key -> value).
pub type ProductAttributes = BTreeMap<String, String>;

/// SQL column type for dynamic attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SqlColumnType {
    Text,
    Integer,
    Real,
    Boolean,
}

impl SqlColumnType {
    pub fn as_sql(&self) -> &'static str {
        match self {
            Self::Text => "TEXT",
            Self::Integer => "INTEGER",
            Self::Real => "REAL",
            Self::Boolean => "BOOLEAN",
        }
    }
}

/// A sanitized column name (safe for SQL identifiers).
///
/// # Invariant (enforced by construction)
///
/// The inner string matches `[a-z0-9][a-z0-9_]*[a-z0-9]` or is a single `[a-z0-9]`.
/// This guarantees it is safe to interpolate inside double-quoted SQL identifiers.
///
/// # Two construction paths
///
/// - [`SanitizedName::from_raw`] — lossy: maps arbitrary input to a valid identifier.
///   Returns the sanitized name *and* a bool indicating whether the input was already valid.
/// - [`Deserialize`] — strict: rejects anything that doesn't already satisfy the invariant.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct SanitizedName(String);

/// Maximum identifier length (PostgreSQL limit).
const MAX_IDENTIFIER_LEN: usize = 63;

impl SanitizedName {
    /// Validate that a string satisfies the SanitizedName invariant.
    fn validate(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= MAX_IDENTIFIER_LEN
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            && !s.starts_with('_')
            && !s.ends_with('_')
    }

    /// Lossy construction from arbitrary user input.
    ///
    /// Returns `(sanitized, was_already_valid)`. The second element lets callers
    /// detect when the mapping was lossy (for diagnostics / collision tracking).
    pub fn from_raw(raw: &str) -> (Self, bool) {
        // Only ASCII alphanumeric and underscore are allowed (non-ASCII → underscore).
        let sanitized: String = raw
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect();
        let trimmed = sanitized.trim_matches('_');

        // Truncate to MAX_IDENTIFIER_LEN chars (safe: all chars are ASCII after sanitization)
        let truncated = if trimmed.len() > MAX_IDENTIFIER_LEN {
            trimmed[..MAX_IDENTIFIER_LEN].trim_end_matches('_')
        } else {
            trimmed
        };

        if truncated.is_empty() {
            // Content-addressed fallback: hash the raw input so different
            // all-special-char inputs don't silently collide on "col".
            let hash = stable_hash(raw);
            (Self(format!("col_{:x}", hash)), false)
        } else {
            let was_valid = truncated == raw;
            (Self(truncated.to_string()), was_valid)
        }
    }

    /// Convenience wrapper that discards the `was_already_valid` flag.
    pub fn new(raw: &str) -> Self {
        Self::from_raw(raw).0
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> serde::Deserialize<'de> for SanitizedName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if !SanitizedName::validate(&s) {
            return Err(serde::de::Error::custom(format!(
                "invalid SanitizedName: '{}' must match [a-z0-9_], not start/end with _, max {} chars",
                s, MAX_IDENTIFIER_LEN
            )));
        }
        Ok(SanitizedName(s))
    }
}

impl std::fmt::Display for SanitizedName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A discovered column from product attributes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredColumn {
    pub original_name: String,
    pub sanitized_name: SanitizedName,
    pub sql_type: SqlColumnType,
    pub display_name: String,
    pub sample_values: Vec<String>,
    pub distinct_count: usize,
}

/// Schema discovered from product attributes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductSchema {
    pub columns: Vec<DiscoveredColumn>,
    pub total_products: usize,
}

/// Parse all unique attribute keys from a set of product rows.
pub fn parse_all_attributes(products: &[ProductRow]) -> Vec<ProductAttributes> {
    products.iter().map(|p| p.attributes.clone()).collect()
}

/// Discover the product schema from a set of product rows.
pub fn discover_product_schema(products: &[ProductRow]) -> ProductSchema {
    let mut column_values: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for product in products {
        for (key, value) in &product.attributes {
            column_values
                .entry(key.clone())
                .or_default()
                .push(value.clone());
        }
    }

    let mut used_names: HashSet<String> = HashSet::new();
    let columns: Vec<DiscoveredColumn> = column_values
        .into_iter()
        .map(|(name, values)| {
            let sql_type = infer_column_type(&values);
            let mut distinct: Vec<String> = values.clone();
            distinct.sort();
            distinct.dedup();
            let distinct_count = distinct.len();
            let sample_values: Vec<String> = distinct.into_iter().take(5).collect();
            let display_name = generate_display_name_adaptive(&name);

            let (mut sanitized, _was_valid) = SanitizedName::from_raw(&name);
            if used_names.contains(sanitized.as_str()) {
                // Content-addressed dedup: hash the *original* name so the
                // suffix is deterministic and won't collide with real columns.
                let hash = stable_hash(&name);
                let base = sanitized.as_str().to_string();
                sanitized = SanitizedName::new(&format!("{}_{:x}", base, hash & 0xFFFF));
                // If *still* colliding (astronomically unlikely), counter fallback
                let mut counter = 1u32;
                while used_names.contains(sanitized.as_str()) {
                    sanitized =
                        SanitizedName::new(&format!("{}_{}_{:x}", base, counter, hash & 0xFFFF));
                    counter += 1;
                }
            }
            used_names.insert(sanitized.as_str().to_string());

            DiscoveredColumn {
                original_name: name.clone(),
                sanitized_name: sanitized,
                sql_type,
                display_name,
                sample_values,
                distinct_count,
            }
        })
        .collect();

    ProductSchema {
        columns,
        total_products: products.len(),
    }
}

/// Generate a human-readable display name from a snake_case or camelCase column name.
pub fn generate_display_name_adaptive(name: &str) -> String {
    // Split on underscores or camelCase boundaries
    let mut words = Vec::new();
    let mut current = String::new();

    for c in name.chars() {
        if c == '_' || c == '-' {
            if !current.is_empty() {
                words.push(current.clone());
                current.clear();
            }
        } else if c.is_uppercase() && !current.is_empty() {
            words.push(current.clone());
            current.clear();
            current.push(c);
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        words.push(current);
    }

    words
        .iter()
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().to_string() + &chars.as_str().to_lowercase(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Infer SQL column type from sample values.
fn infer_column_type(values: &[String]) -> SqlColumnType {
    let non_empty: Vec<&str> = values
        .iter()
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .collect();
    if non_empty.is_empty() {
        return SqlColumnType::Text;
    }

    // Check if all values are boolean-like
    if non_empty.iter().all(|v| {
        matches!(
            v.to_lowercase().as_str(),
            "true" | "false" | "0" | "1" | "yes" | "no"
        )
    }) {
        return SqlColumnType::Boolean;
    }

    // Check if all values are integers
    if non_empty.iter().all(|v| v.parse::<i64>().is_ok()) {
        return SqlColumnType::Integer;
    }

    // Check if all values are floats
    if non_empty.iter().all(|v| v.parse::<f64>().is_ok()) {
        return SqlColumnType::Real;
    }

    SqlColumnType::Text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitized_name_basic() {
        assert_eq!(SanitizedName::new("Hello World").as_str(), "hello_world");
        assert_eq!(SanitizedName::new("foo-bar").as_str(), "foo_bar");
    }

    #[test]
    fn sanitized_name_empty_inputs_get_distinct_hashes() {
        // Different all-special-char inputs must NOT collapse to the same name
        let a = SanitizedName::new("___");
        let b = SanitizedName::new("!!!");
        let c = SanitizedName::new("...");
        assert!(a.as_str().starts_with("col_"), "got: {}", a.as_str());
        assert!(b.as_str().starts_with("col_"), "got: {}", b.as_str());
        assert_ne!(a, b, "different inputs must produce different hashed names");
        assert_ne!(b, c);
        assert_ne!(a, c);
    }

    #[test]
    fn sanitized_name_from_raw_reports_validity() {
        let (_, valid) = SanitizedName::from_raw("hello_world");
        assert!(valid);
        let (_, valid) = SanitizedName::from_raw("Hello World");
        assert!(!valid);
    }

    #[test]
    fn display_name_generation() {
        assert_eq!(
            generate_display_name_adaptive("product_name"),
            "Product Name"
        );
        assert_eq!(
            generate_display_name_adaptive("productName"),
            "Product Name"
        );
        assert_eq!(generate_display_name_adaptive("ABC"), "A B C");
    }

    #[test]
    fn infer_types() {
        assert_eq!(
            infer_column_type(&["1".into(), "2".into(), "3".into()]),
            SqlColumnType::Integer
        );
        assert_eq!(
            infer_column_type(&["1.5".into(), "2.3".into()]),
            SqlColumnType::Real
        );
        assert_eq!(
            infer_column_type(&["true".into(), "false".into()]),
            SqlColumnType::Boolean
        );
        assert_eq!(
            infer_column_type(&["hello".into(), "world".into()]),
            SqlColumnType::Text
        );
    }

    #[test]
    fn deserialize_sanitized_name_rejects_sql_injection() {
        // Attempt to deserialize SQL injection payloads
        let injection = "\"foo; DROP TABLE users--\"";
        let result: Result<SanitizedName, _> = serde_json::from_str(injection);
        assert!(result.is_err(), "SQL injection content must be rejected");

        let injection2 = "\"hello world\"";
        let result2: Result<SanitizedName, _> = serde_json::from_str(injection2);
        assert!(result2.is_err(), "spaces must be rejected");

        let injection3 = "\"Robert'); DROP TABLE students;--\"";
        let result3: Result<SanitizedName, _> = serde_json::from_str(injection3);
        assert!(result3.is_err(), "Bobby Tables must be rejected");

        let empty = "\"\"";
        let result4: Result<SanitizedName, _> = serde_json::from_str(empty);
        assert!(result4.is_err(), "empty string must be rejected");

        let leading_underscore = "\"_foo\"";
        let result5: Result<SanitizedName, _> = serde_json::from_str(leading_underscore);
        assert!(result5.is_err(), "leading underscore must be rejected");

        let trailing_underscore = "\"foo_\"";
        let result6: Result<SanitizedName, _> = serde_json::from_str(trailing_underscore);
        assert!(result6.is_err(), "trailing underscore must be rejected");

        let uppercase = "\"FooBar\"";
        let result7: Result<SanitizedName, _> = serde_json::from_str(uppercase);
        assert!(result7.is_err(), "uppercase letters must be rejected");
    }

    #[test]
    fn deserialize_sanitized_name_accepts_valid() {
        let valid: SanitizedName = serde_json::from_str("\"hello_world\"").unwrap();
        assert_eq!(valid.as_str(), "hello_world");

        let simple: SanitizedName = serde_json::from_str("\"foo\"").unwrap();
        assert_eq!(simple.as_str(), "foo");

        let with_digits: SanitizedName = serde_json::from_str("\"col_1\"").unwrap();
        assert_eq!(with_digits.as_str(), "col_1");

        let digits_only: SanitizedName = serde_json::from_str("\"123\"").unwrap();
        assert_eq!(digits_only.as_str(), "123");
    }

    // =========================================================================
    // SanitizedName proptest laws
    // =========================================================================

    mod hegel_laws {
        use super::*;
        use hegel::generators;

        /// L1 (Totality): from_raw never panics for any &str.
        #[hegel::test]
        fn l1_totality(tc: hegel::TestCase) {
            let s: String = tc.draw(generators::text());
            let (_name, _valid) = SanitizedName::from_raw(&s);
            // If we get here, it didn't panic — that's the law.
        }

        /// L2 (Idempotency): applying from_raw to its own output is a fixpoint.
        #[hegel::test]
        fn l2_idempotency(tc: hegel::TestCase) {
            let s: String = tc.draw(generators::text());
            let (first, _) = SanitizedName::from_raw(&s);
            let (second, was_valid) = SanitizedName::from_raw(first.as_str());
            assert_eq!(
                first.as_str(),
                second.as_str(),
                "from_raw must be idempotent: first={}, second={}",
                first.as_str(),
                second.as_str()
            );
            assert!(
                was_valid,
                "output of from_raw must already be valid: {}",
                first.as_str()
            );
        }

        /// L3 (SQL-Safety): output matches [a-z0-9][a-z0-9_]*[a-z0-9] | [a-z0-9], max 63 chars.
        #[hegel::test]
        fn l3_sql_safety(tc: hegel::TestCase) {
            let s: String = tc.draw(generators::text());
            let (name, _) = SanitizedName::from_raw(&s);
            let v = name.as_str();

            // Non-empty
            assert!(!v.is_empty(), "SanitizedName must not be empty");
            // Max length
            assert!(v.len() <= 63, "len {} > 63 for input {:?}", v.len(), s);
            // Character allowlist
            assert!(
                v.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "invalid char in {:?}",
                v
            );
            // No leading/trailing underscore
            assert!(!v.starts_with('_'), "starts with _ : {:?}", v);
            assert!(!v.ends_with('_'), "ends with _ : {:?}", v);
            // No SQL injection characters
            for bad in &[';', '\'', '"', '-', '(', ')', ' ', '\n', '\t'] {
                assert!(!v.contains(*bad), "contains {:?} in {:?}", bad, v);
            }
        }
    }

    /// Golden test: pin stable_hash values so changes to the algorithm
    /// are caught immediately (hashes are persisted as database column names).
    #[test]
    fn stable_hash_golden_values() {
        assert_eq!(stable_hash("___"), 0xb8c3741a0c665b74);
        assert_eq!(stable_hash("!!!"), 0xbbe43c17ca866be2);
        assert_eq!(stable_hash("..."), 0xf7d93e17ec4b1219);
        assert_eq!(stable_hash("hello"), 0xa430d84680aabd0b);
        // Verify distinct inputs produce distinct hashes
        assert_ne!(stable_hash("___"), stable_hash("!!!"));
        assert_ne!(stable_hash("foo"), stable_hash("bar"));
    }

    #[test]
    fn collision_detection_distinct_sanitized_names() {
        // "Foo Bar" -> foo_bar, "foo-bar" -> foo_bar (collision!)
        let products = vec![ProductRow {
            product_id: "p1".into(),
            segment: "s".into(),
            subsegment: "ss".into(),
            brand: "b".into(),
            sub_brand: "sb".into(),
            attributes: BTreeMap::from([
                ("Foo Bar".into(), "val1".into()),
                ("foo-bar".into(), "val2".into()),
            ]),
        }];
        let schema = discover_product_schema(&products);
        let names: Vec<&str> = schema
            .columns
            .iter()
            .map(|c| c.sanitized_name.as_str())
            .collect();
        // All sanitized names must be unique
        let unique_names: HashSet<&str> = names.iter().copied().collect();
        assert_eq!(
            names.len(),
            unique_names.len(),
            "sanitized names must be unique, got: {:?}",
            names
        );
        // Both original columns must be present
        assert_eq!(schema.columns.len(), 2);
    }
}
