use std::fs;

use agent_fw_catalog::{CatalogEntry, CatalogKind};
use agent_fw_cli::catalog_export::{write_catalog_entries_json, MemoryCatalogWriter};

fn entry(id: &str, kind: CatalogKind, name: &str) -> CatalogEntry {
    CatalogEntry {
        id: id.to_string(),
        kind,
        name: name.to_string(),
        qualified_name: None,
        content: String::new(),
        tags: vec![],
        links: vec![],
        metadata: serde_json::json!({}),
    }
}

#[tokio::test]
async fn memory_writer_writes_sorted_json_array_and_creates_parent_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("nested/catalog.entries.json");
    let writer = MemoryCatalogWriter::default();

    writer
        .save_entries(vec![
            entry("enum:c", CatalogKind::Enum, "c"),
            entry("column:b", CatalogKind::Column, "b"),
            entry("table:a", CatalogKind::Table, "a"),
        ])
        .await
        .unwrap();

    let written_count = write_catalog_entries_json(&writer, &out).await.unwrap();

    let json = fs::read_to_string(out).unwrap();
    let entries: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert_eq!(written_count, 2);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["itemType"], "table");
    assert_eq!(entries[1]["itemType"], "column");
    assert!(!json.contains("\"itemType\": \"enum\""));
}
