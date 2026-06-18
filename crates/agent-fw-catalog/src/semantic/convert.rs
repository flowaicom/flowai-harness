use std::convert::TryFrom;

use super::entity::SemanticEntity;
use super::metadata::{
    decode_metadata, ColumnMetadata, DataQualityFindingMetadata, DocumentMetadata,
    EnumValueMetadata, KnowledgeMetadata, MetricMetadata, RelationshipMetadata, TableMetadata,
};
use crate::{CatalogEntry, CatalogError, CatalogKind};

impl TryFrom<CatalogEntry> for SemanticEntity {
    type Error = CatalogError;

    fn try_from(entry: CatalogEntry) -> Result<Self, Self::Error> {
        match entry.kind {
            CatalogKind::Table => {
                let metadata: TableMetadata = decode_metadata(&entry)?;
                ensure_nonblank_database_id(&entry, &metadata.database_id)?;
                Ok(Self::Table { entry, metadata })
            }
            CatalogKind::Column => {
                let metadata: ColumnMetadata = decode_metadata(&entry)?;
                ensure_nonblank_database_id(&entry, &metadata.database_id)?;
                Ok(Self::Column { entry, metadata })
            }
            CatalogKind::Relationship => {
                let metadata: RelationshipMetadata = decode_metadata(&entry)?;
                ensure_nonblank_database_id(&entry, &metadata.database_id)?;
                Ok(Self::Relationship { entry, metadata })
            }
            CatalogKind::Enum => {
                let metadata: EnumValueMetadata = decode_metadata(&entry)?;
                ensure_nonblank_database_id(&entry, &metadata.database_id)?;
                Ok(Self::EnumValue { entry, metadata })
            }
            CatalogKind::Metric => {
                let metadata: MetricMetadata = decode_metadata(&entry)?;
                Ok(Self::Metric { entry, metadata })
            }
            CatalogKind::Special => Ok(Self::Special { entry }),
            CatalogKind::Document => {
                let metadata: DocumentMetadata = decode_metadata(&entry)?;
                Ok(Self::Document { entry, metadata })
            }
            CatalogKind::Knowledge => {
                let metadata: KnowledgeMetadata = decode_metadata(&entry)?;
                Ok(Self::Knowledge { entry, metadata })
            }
            CatalogKind::DataQualityFinding => {
                let metadata: DataQualityFindingMetadata = decode_metadata(&entry)?;
                ensure_nonblank_database_id(&entry, &metadata.database_id)?;
                Ok(Self::DataQualityFinding { entry, metadata })
            }
        }
    }
}

impl TryFrom<&CatalogEntry> for SemanticEntity {
    type Error = CatalogError;

    fn try_from(entry: &CatalogEntry) -> Result<Self, Self::Error> {
        Self::try_from(entry.clone())
    }
}

fn ensure_nonblank_database_id(
    entry: &CatalogEntry,
    database_id: &str,
) -> Result<(), CatalogError> {
    if database_id.trim().is_empty() {
        return Err(CatalogError::InvalidQuery(format!(
            "invalid {} metadata for {}: databaseId must not be blank",
            entry.kind, entry.id
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn semantic_entry(kind: CatalogKind, metadata: serde_json::Value) -> CatalogEntry {
        CatalogEntry {
            id: format!("{kind}-id"),
            kind,
            name: kind.as_str().to_string(),
            qualified_name: None,
            content: String::new(),
            tags: Vec::new(),
            links: Vec::new(),
            metadata,
        }
    }

    #[test]
    fn semantic_conversion_decodes_table_metadata() {
        let entry = semantic_entry(
            CatalogKind::Table,
            json!({
                "databaseId": "warehouse",
                "schemaName": "public",
                "tableName": "fact_sales",
                "relationType": "base_table",
                "rowCount": 100,
                "columnCount": 3,
                "preferredQuerySurface": true,
                "source": {
                    "profilingRunId": "profile-1"
                }
            }),
        );

        let entity = SemanticEntity::try_from(entry).unwrap();
        assert_eq!(
            entity.kind(),
            super::super::entity::SemanticEntityKind::Table
        );
        match entity {
            SemanticEntity::Table { metadata, .. } => {
                assert_eq!(metadata.table_name, "fact_sales");
                assert!(metadata.preferred_query_surface);
            }
            _ => panic!("expected table entity"),
        }
    }

    #[test]
    fn semantic_conversion_rejects_blank_database_id() {
        let entry = semantic_entry(
            CatalogKind::Table,
            json!({
                "databaseId": " ",
                "schemaName": "public",
                "tableName": "fact_sales"
            }),
        );

        let error = SemanticEntity::try_from(entry).unwrap_err();

        assert!(error.to_string().contains("databaseId must not be blank"));
    }

    #[test]
    fn semantic_conversion_reports_invalid_metadata() {
        let entry = semantic_entry(CatalogKind::Column, json!({"databaseId": "warehouse"}));

        let error = SemanticEntity::try_from(entry).unwrap_err();
        assert!(error.to_string().contains("invalid column metadata"));
    }
}
