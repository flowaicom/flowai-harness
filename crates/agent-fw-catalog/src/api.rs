use serde::{Deserialize, Serialize};

/// Request to find a join path.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FindJoinPathRequest {
    pub from_table: String,
    pub to_table: String,
}

/// Request to profile a single table.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileTableRequest {
    pub source_id: String,
    pub schema_name: Option<String>,
    pub table_name: String,
    pub model_id: Option<String>,
    pub sample_size: Option<usize>,
}

/// Request to profile an entire database or a selected subset of tables.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileDatabaseRequest {
    pub source_id: String,
    pub schema_name: Option<String>,
    #[serde(default)]
    pub tables: Vec<String>,
    pub model_id: Option<String>,
    pub sample_size: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_join_path_request_rejects_unknown_fields() {
        let err = serde_json::from_value::<FindJoinPathRequest>(serde_json::json!({
            "fromTable": "products",
            "toTable": "stores",
            "extra": true
        }))
        .unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn profile_table_request_roundtrip() {
        let value = ProfileTableRequest {
            source_id: "src-1".to_string(),
            schema_name: Some("public".to_string()),
            table_name: "products".to_string(),
            model_id: Some("claude-haiku-4-5".to_string()),
            sample_size: Some(25),
        };
        let json = serde_json::to_string(&value).unwrap();
        let roundtrip: ProfileTableRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip.source_id, "src-1");
        assert_eq!(roundtrip.schema_name.as_deref(), Some("public"));
        assert_eq!(roundtrip.table_name, "products");
        assert_eq!(roundtrip.model_id.as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(roundtrip.sample_size, Some(25));
    }

    #[test]
    fn profile_database_request_roundtrip() {
        let value = ProfileDatabaseRequest {
            source_id: "src-1".to_string(),
            schema_name: None,
            tables: vec!["products".to_string(), "stores".to_string()],
            model_id: Some("claude-haiku-4-5".to_string()),
            sample_size: Some(50),
        };
        let json = serde_json::to_string(&value).unwrap();
        let roundtrip: ProfileDatabaseRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip.source_id, "src-1");
        assert_eq!(roundtrip.tables, vec!["products", "stores"]);
        assert_eq!(roundtrip.model_id.as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(roundtrip.sample_size, Some(50));
    }

    #[test]
    fn profile_requests_reject_unknown_fields() {
        let table_err = serde_json::from_value::<ProfileTableRequest>(serde_json::json!({
            "sourceId": "src-1",
            "tableName": "products",
            "extra": true
        }))
        .unwrap_err();
        assert!(table_err.to_string().contains("unknown field"));

        let database_err = serde_json::from_value::<ProfileDatabaseRequest>(serde_json::json!({
            "sourceId": "src-1",
            "tables": ["products"],
            "extra": true
        }))
        .unwrap_err();
        assert!(database_err.to_string().contains("unknown field"));
    }
}
