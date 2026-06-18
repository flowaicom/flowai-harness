//! Thread file metadata and stored file content.
//!
//! These types are generic workspace/runtime shapes:
//! - a thread-scoped file index
//! - file metadata for UI listing
//! - stored file payload + filename for download responses

use serde::{Deserialize, Serialize};

/// Metadata for a file attached to a thread.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadFileEntry {
    pub file_id: String,
    pub filename: String,
    pub thread_id: String,
    pub created_at: String,
}

/// Index of files for a thread, typically stored as one KV/blob value.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadFileIndex {
    pub files: Vec<ThreadFileEntry>,
}

/// File content + metadata stored under a file key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredFile {
    pub content: String,
    pub filename: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_file_entry_serializes_camel_case() {
        let entry = ThreadFileEntry {
            file_id: "f-1".to_string(),
            filename: "report.csv".to_string(),
            thread_id: "t-1".to_string(),
            created_at: "2026-03-09T10:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"fileId\""));
        assert!(json.contains("\"threadId\""));
        assert!(json.contains("\"createdAt\""));
        assert!(!json.contains("\"file_id\""));
    }

    #[test]
    fn thread_file_index_default_is_empty() {
        assert!(ThreadFileIndex::default().files.is_empty());
    }

    #[test]
    fn stored_file_roundtrips() {
        let stored = StoredFile {
            content: "a,b\n1,2\n".to_string(),
            filename: "data.csv".to_string(),
        };
        let json = serde_json::to_string(&stored).unwrap();
        let roundtrip: StoredFile = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip, stored);
    }
}
