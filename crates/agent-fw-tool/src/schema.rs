//! ToolSchema trait for JSON schema generation.
//!
//! Tools implement this trait to provide their input schema.
//! The `#[derive(ToolSchema)]` proc macro generates implementations automatically.

/// Trait for types that can produce a JSON schema describing their structure.
///
/// Used by the tool framework to generate tool definitions for LLM function calling.
///
/// # Example
///
/// ```ignore
/// use agent_fw_tool::DeriveToolSchema;
///
/// #[derive(serde::Deserialize, DeriveToolSchema)]
/// pub struct SearchInput {
///     /// Search query text
///     pub query: String,
///
///     /// Max results to return
///     pub limit: Option<usize>,
/// }
///
/// let schema = SearchInput::json_schema();
/// ```
pub trait ToolSchema {
    /// Generate the JSON schema for this type.
    fn json_schema() -> serde_json::Value;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DeriveToolSchema;
    use serde::Deserialize;

    // =========================================================================
    // Basic derive tests
    // =========================================================================

    #[derive(Deserialize, DeriveToolSchema)]
    struct BasicInput {
        /// Search query text
        query: String,
        /// Max results
        limit: Option<usize>,
    }

    #[test]
    fn basic_schema_generation() {
        let schema = BasicInput::json_schema();
        assert_eq!(schema["type"], "object");

        let props = &schema["properties"];
        assert_eq!(props["query"]["type"], "string");
        assert_eq!(props["query"]["description"], "Search query text");
        assert_eq!(props["limit"]["type"], "integer");
        assert_eq!(props["limit"]["description"], "Max results");

        // query is required (non-Option), limit is not (Option)
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("query")));
        assert!(!required.contains(&serde_json::json!("limit")));
    }

    // =========================================================================
    // All primitive types
    // =========================================================================

    #[derive(Deserialize, DeriveToolSchema)]
    struct AllTypes {
        s: String,
        i: i64,
        u: u32,
        f: f64,
        b: bool,
        v: Vec<String>,
    }

    #[test]
    fn all_primitive_types() {
        let schema = AllTypes::json_schema();
        let props = &schema["properties"];

        assert_eq!(props["s"]["type"], "string");
        assert_eq!(props["i"]["type"], "integer");
        assert_eq!(props["u"]["type"], "integer");
        assert_eq!(props["f"]["type"], "number");
        assert_eq!(props["b"]["type"], "boolean");
        assert_eq!(props["v"]["type"], "array");
        assert_eq!(props["v"]["items"]["type"], "string");
    }

    // =========================================================================
    // serde rename_all
    // =========================================================================

    #[derive(Deserialize, DeriveToolSchema)]
    #[serde(rename_all = "camelCase")]
    struct CamelCaseInput {
        /// Filter by type
        product_type: String,
        /// Max count
        max_results: Option<u32>,
    }

    #[test]
    fn serde_rename_all_camel_case() {
        let schema = CamelCaseInput::json_schema();
        let props = schema["properties"].as_object().unwrap();

        // Field names should be camelCase
        assert!(props.contains_key("productType"));
        assert!(props.contains_key("maxResults"));
        assert!(!props.contains_key("product_type"));

        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("productType")));
    }

    // =========================================================================
    // serde rename on field
    // =========================================================================

    #[derive(Deserialize, DeriveToolSchema)]
    struct RenameFieldInput {
        #[serde(rename = "queryText")]
        query: String,
    }

    #[test]
    fn serde_rename_field() {
        let schema = RenameFieldInput::json_schema();
        let props = schema["properties"].as_object().unwrap();

        assert!(props.contains_key("queryText"));
        assert!(!props.contains_key("query"));
    }

    // =========================================================================
    // schema attributes
    // =========================================================================

    #[derive(Deserialize, DeriveToolSchema)]
    struct SchemaAttrsInput {
        /// Default description from doc
        #[schema(required)]
        maybe_required: Option<String>,

        #[schema(description = "Overridden description")]
        with_desc: String,

        #[schema(enum_values = ["electronics", "clothing", "food"])]
        category: Option<String>,
    }

    #[test]
    fn schema_required_on_option() {
        let schema = SchemaAttrsInput::json_schema();
        let required = schema["required"].as_array().unwrap();

        // Option field with #[schema(required)] should be required
        assert!(required.contains(&serde_json::json!("maybe_required")));
        assert!(required.contains(&serde_json::json!("with_desc")));
    }

    #[test]
    fn schema_description_override() {
        let schema = SchemaAttrsInput::json_schema();
        let props = &schema["properties"];

        assert_eq!(props["with_desc"]["description"], "Overridden description");
    }

    #[test]
    fn schema_enum_values() {
        let schema = SchemaAttrsInput::json_schema();
        let props = &schema["properties"];

        let enum_vals = props["category"]["enum"].as_array().unwrap();
        assert_eq!(enum_vals.len(), 3);
        assert!(enum_vals.contains(&serde_json::json!("electronics")));
        assert!(enum_vals.contains(&serde_json::json!("clothing")));
        assert!(enum_vals.contains(&serde_json::json!("food")));
    }

    // =========================================================================
    // serde_json::Value field
    // =========================================================================

    #[derive(Deserialize, DeriveToolSchema)]
    struct JsonValueInput {
        /// Arbitrary JSON payload
        payload: serde_json::Value,
    }

    #[test]
    fn json_value_produces_empty_schema() {
        let schema = JsonValueInput::json_schema();
        let props = &schema["properties"];

        // serde_json::Value should produce {} (no type constraint)
        let payload_schema = props["payload"].as_object().unwrap();
        assert!(!payload_schema.contains_key("type"));
    }

    // =========================================================================
    // Round-trip deserialization tests (proves schema matches actual struct)
    // =========================================================================

    #[test]
    fn basic_input_roundtrip() {
        let json = r#"{"query": "test", "limit": 5}"#;
        let input: BasicInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.query, "test");
        assert_eq!(input.limit, Some(5));

        let json_no_limit = r#"{"query": "hello"}"#;
        let input2: BasicInput = serde_json::from_str(json_no_limit).unwrap();
        assert_eq!(input2.query, "hello");
        assert_eq!(input2.limit, None);
    }

    #[test]
    fn camel_case_input_roundtrip() {
        let json = r#"{"productType": "electronics", "maxResults": 10}"#;
        let input: CamelCaseInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.product_type, "electronics");
        assert_eq!(input.max_results, Some(10));
    }

    #[test]
    fn schema_attrs_input_roundtrip() {
        let json = r#"{"maybe_required": "yes", "with_desc": "hello", "category": "food"}"#;
        let input: SchemaAttrsInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.maybe_required, Some("yes".to_string()));
        assert_eq!(input.with_desc, "hello");
        assert_eq!(input.category, Some("food".to_string()));
    }

    #[test]
    fn json_value_input_roundtrip() {
        let json = r#"{"payload": {"nested": true, "count": 42}}"#;
        let input: JsonValueInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.payload["nested"], true);
        assert_eq!(input.payload["count"], 42);
    }

    #[test]
    fn rename_field_input_roundtrip() {
        let json = r#"{"queryText": "search term"}"#;
        let input: RenameFieldInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.query, "search term");
    }

    #[test]
    fn all_types_roundtrip() {
        let json = r#"{"s": "hello", "i": -42, "u": 7, "f": 3.14, "b": true, "v": ["a", "b"]}"#;
        let input: AllTypes = serde_json::from_str(json).unwrap();
        assert_eq!(input.s, "hello");
        assert_eq!(input.i, -42);
        assert_eq!(input.u, 7);
        assert!((input.f - 3.14).abs() < f64::EPSILON);
        assert!(input.b);
        assert_eq!(input.v, vec!["a", "b"]);
    }
}
