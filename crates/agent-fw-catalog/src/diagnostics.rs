use std::collections::{BTreeMap, BTreeSet};

use crate::{
    decode_metadata, relation_kind, CatalogEntry, CatalogKind, ColumnMetadata,
    DataQualityFindingMetadata, EnumValueMetadata, RelationshipMetadata, TableMetadata,
};

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogRelationDiagnostics {
    pub total_relations: usize,
    pub orphaned_relations: usize,
    pub database_mismatched_relations: usize,
    pub relation_counts_by_kind: BTreeMap<String, usize>,
    pub orphaned_counts_by_kind: BTreeMap<String, usize>,
    pub database_mismatch_counts_by_kind: BTreeMap<String, usize>,
    pub samples: Vec<CatalogRelationDiagnostic>,
}

impl CatalogRelationDiagnostics {
    pub fn is_clean(&self) -> bool {
        self.orphaned_relations == 0 && self.database_mismatched_relations == 0
    }

    pub fn record_missing_source_relation(
        &mut self,
        source_id: String,
        target_id: String,
        relation_kind: String,
    ) {
        self.total_relations += 1;
        self.orphaned_relations += 1;
        increment(&mut self.relation_counts_by_kind, &relation_kind);
        increment(&mut self.orphaned_counts_by_kind, &relation_kind);
        push_sample(
            self,
            source_id,
            target_id,
            relation_kind,
            CatalogRelationIssue::MissingSource,
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogRelationDiagnostic {
    pub source_id: String,
    pub target_id: String,
    pub relation_kind: String,
    pub issue: CatalogRelationIssue,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CatalogRelationIssue {
    MissingSource,
    MissingTarget,
    InvalidRelationshipMetadata(String),
    RelationshipTargetDatabaseMismatch {
        #[serde(rename = "expectedDatabaseId")]
        expected_database_id: String,
        #[serde(rename = "actualDatabaseId")]
        actual_database_id: Option<String>,
    },
    RelationEndpointDatabaseMismatch {
        #[serde(rename = "sourceDatabaseId")]
        source_database_id: String,
        #[serde(rename = "targetDatabaseId")]
        target_database_id: String,
    },
}

pub fn diagnose_catalog_relations(entries: &[CatalogEntry]) -> CatalogRelationDiagnostics {
    let ids = entries
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<BTreeSet<_>>();
    let entries_by_id = entries
        .iter()
        .map(|entry| (entry.id.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut diagnostics = CatalogRelationDiagnostics::default();

    for entry in entries {
        for link in &entry.links {
            diagnostics.total_relations += 1;
            increment(&mut diagnostics.relation_counts_by_kind, &link.kind);
            if !ids.contains(link.target_id.as_str()) {
                diagnostics.orphaned_relations += 1;
                increment(&mut diagnostics.orphaned_counts_by_kind, &link.kind);
                push_sample(
                    &mut diagnostics,
                    entry.id.clone(),
                    link.target_id.clone(),
                    link.kind.clone(),
                    CatalogRelationIssue::MissingTarget,
                );
                continue;
            }

            diagnose_link_database_scope(entry, link, &entries_by_id, &mut diagnostics);
        }

        if entry.kind == CatalogKind::Relationship {
            diagnose_relationship_targets(entry, &entries_by_id, &mut diagnostics);
        }
    }

    diagnostics
}

fn diagnose_link_database_scope(
    entry: &CatalogEntry,
    link: &crate::CatalogRelation,
    entries_by_id: &BTreeMap<&str, &CatalogEntry>,
    diagnostics: &mut CatalogRelationDiagnostics,
) {
    if !requires_same_database(&link.kind) {
        return;
    }

    let Some(target) = entries_by_id.get(link.target_id.as_str()) else {
        return;
    };
    let Some(source_database_id) = entry_database_id(entry) else {
        return;
    };
    let Some(target_database_id) = entry_database_id(target) else {
        return;
    };

    if source_database_id != target_database_id {
        diagnostics.database_mismatched_relations += 1;
        increment(
            &mut diagnostics.database_mismatch_counts_by_kind,
            &link.kind,
        );
        push_sample(
            diagnostics,
            entry.id.clone(),
            link.target_id.clone(),
            link.kind.clone(),
            CatalogRelationIssue::RelationEndpointDatabaseMismatch {
                source_database_id,
                target_database_id,
            },
        );
    }
}

fn requires_same_database(relation_kind: &str) -> bool {
    matches!(
        relation_kind,
        relation_kind::HAS_COLUMN
            | relation_kind::BELONGS_TO
            | relation_kind::REFERENCES
            | relation_kind::REFERENCED_BY
            | relation_kind::REFERENCES_TABLE
            | relation_kind::REFERENCED_BY_TABLE
            | relation_kind::RELATIONSHIP_SOURCE_TABLE
            | relation_kind::RELATIONSHIP_TARGET_TABLE
            | relation_kind::ENUM_VALUE_OF
            | relation_kind::HAS_ENUM_VALUE
            | relation_kind::KNOWLEDGE_APPLIES_TO
            | relation_kind::DATA_QUALITY_FINDING_APPLIES_TO
            | relation_kind::APPLIES_TO
    )
}

fn diagnose_relationship_targets(
    entry: &CatalogEntry,
    entries_by_id: &BTreeMap<&str, &CatalogEntry>,
    diagnostics: &mut CatalogRelationDiagnostics,
) {
    let metadata = match decode_metadata::<RelationshipMetadata>(entry) {
        Ok(metadata) => metadata,
        Err(error) => {
            push_sample(
                diagnostics,
                entry.id.clone(),
                entry.id.clone(),
                relation_kind::RELATIONSHIP_SOURCE_TABLE.to_string(),
                CatalogRelationIssue::InvalidRelationshipMetadata(error.to_string()),
            );
            return;
        }
    };

    diagnose_relationship_table_target(
        entry,
        &metadata.source_table_id,
        relation_kind::RELATIONSHIP_SOURCE_TABLE,
        &metadata.database_id,
        !has_link(
            entry,
            &metadata.source_table_id,
            relation_kind::RELATIONSHIP_SOURCE_TABLE,
        ),
        entries_by_id,
        diagnostics,
    );
    diagnose_relationship_table_target(
        entry,
        &metadata.target_table_id,
        relation_kind::RELATIONSHIP_TARGET_TABLE,
        &metadata.database_id,
        !has_link(
            entry,
            &metadata.target_table_id,
            relation_kind::RELATIONSHIP_TARGET_TABLE,
        ),
        entries_by_id,
        diagnostics,
    );
}

fn diagnose_relationship_table_target(
    entry: &CatalogEntry,
    table_id: &str,
    relation_kind: &str,
    expected_database_id: &str,
    count_relation: bool,
    entries_by_id: &BTreeMap<&str, &CatalogEntry>,
    diagnostics: &mut CatalogRelationDiagnostics,
) {
    if count_relation {
        diagnostics.total_relations += 1;
        increment(&mut diagnostics.relation_counts_by_kind, relation_kind);
    }

    let Some(table) = entries_by_id.get(table_id) else {
        if count_relation {
            diagnostics.orphaned_relations += 1;
            increment(&mut diagnostics.orphaned_counts_by_kind, relation_kind);
            push_sample(
                diagnostics,
                entry.id.clone(),
                table_id.to_string(),
                relation_kind.to_string(),
                CatalogRelationIssue::MissingTarget,
            );
        }
        return;
    };

    let actual_database_id = table_database_id(table);
    if count_relation && actual_database_id.as_deref() != Some(expected_database_id) {
        diagnostics.database_mismatched_relations += 1;
        increment(
            &mut diagnostics.database_mismatch_counts_by_kind,
            relation_kind,
        );
        push_sample(
            diagnostics,
            entry.id.clone(),
            table_id.to_string(),
            relation_kind.to_string(),
            CatalogRelationIssue::RelationshipTargetDatabaseMismatch {
                expected_database_id: expected_database_id.to_string(),
                actual_database_id,
            },
        );
    }
}

fn has_link(entry: &CatalogEntry, target_id: &str, relation_kind: &str) -> bool {
    entry
        .links
        .iter()
        .any(|link| link.target_id == target_id && link.kind == relation_kind)
}

fn table_database_id(entry: &CatalogEntry) -> Option<String> {
    match entry_database_id(entry) {
        Some(database_id) if entry.kind == CatalogKind::Table => Some(database_id),
        _ => None,
    }
}

fn entry_database_id(entry: &CatalogEntry) -> Option<String> {
    let database_id = match entry.kind {
        CatalogKind::Table => decode_metadata::<TableMetadata>(entry).ok()?.database_id,
        CatalogKind::Column => decode_metadata::<ColumnMetadata>(entry).ok()?.database_id,
        CatalogKind::Relationship => {
            decode_metadata::<RelationshipMetadata>(entry)
                .ok()?
                .database_id
        }
        CatalogKind::Enum => {
            decode_metadata::<EnumValueMetadata>(entry)
                .ok()?
                .database_id
        }
        CatalogKind::DataQualityFinding => {
            decode_metadata::<DataQualityFindingMetadata>(entry)
                .ok()?
                .database_id
        }
        CatalogKind::Metric
        | CatalogKind::Knowledge
        | CatalogKind::Document
        | CatalogKind::Special => {
            return None;
        }
    };
    let database_id = database_id.trim();
    if database_id.is_empty() {
        None
    } else {
        Some(database_id.to_string())
    }
}

fn increment(counts: &mut BTreeMap<String, usize>, key: &str) {
    *counts.entry(key.to_string()).or_default() += 1;
}

fn push_sample(
    diagnostics: &mut CatalogRelationDiagnostics,
    source_id: String,
    target_id: String,
    relation_kind: String,
    issue: CatalogRelationIssue,
) {
    if diagnostics.samples.len() >= 10 {
        return;
    }
    diagnostics.samples.push(CatalogRelationDiagnostic {
        source_id,
        target_id,
        relation_kind,
        issue,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cardinality, CatalogRelation};
    use serde_json::json;

    fn table(id: &str, database_id: &str) -> CatalogEntry {
        CatalogEntry {
            id: id.to_string(),
            kind: CatalogKind::Table,
            name: id.to_string(),
            qualified_name: Some(id.trim_start_matches("table:").to_string()),
            content: String::new(),
            tags: Vec::new(),
            links: Vec::new(),
            metadata: json!({
                "databaseId": database_id,
                "schemaName": "public",
                "tableName": id.trim_start_matches("table:"),
                "relationType": "table"
            }),
        }
    }

    fn relationship(
        source_table_id: &str,
        target_table_id: &str,
        database_id: &str,
    ) -> CatalogEntry {
        CatalogEntry {
            id: "relationship:orders_products".to_string(),
            kind: CatalogKind::Relationship,
            name: "orders_products".to_string(),
            qualified_name: None,
            content: String::new(),
            tags: Vec::new(),
            links: vec![
                CatalogRelation {
                    target_id: source_table_id.to_string(),
                    kind: relation_kind::RELATIONSHIP_SOURCE_TABLE.to_string(),
                    description: None,
                },
                CatalogRelation {
                    target_id: target_table_id.to_string(),
                    kind: relation_kind::RELATIONSHIP_TARGET_TABLE.to_string(),
                    description: None,
                },
            ],
            metadata: json!({
                "databaseId": database_id,
                "sourceTableId": source_table_id,
                "targetTableId": target_table_id,
                "sourceSchema": "public",
                "sourceTable": "orders",
                "sourceColumn": "product_id",
                "targetSchema": "public",
                "targetTable": "products",
                "targetColumn": "id",
                "sourceCardinality": Cardinality::Many,
                "targetCardinality": Cardinality::One,
                "relationshipKind": "foreign_key"
            }),
        }
    }

    #[test]
    fn relation_diagnostics_reports_orphans_and_database_mismatches() {
        let entries = vec![
            table("table:orders", "warehouse"),
            table("table:products", "other_warehouse"),
            relationship("table:orders", "table:products", "warehouse"),
            CatalogEntry {
                id: "knowledge:rule".to_string(),
                kind: CatalogKind::Knowledge,
                name: "rule".to_string(),
                qualified_name: None,
                content: String::new(),
                tags: Vec::new(),
                links: vec![CatalogRelation {
                    target_id: "table:missing".to_string(),
                    kind: relation_kind::KNOWLEDGE_APPLIES_TO.to_string(),
                    description: None,
                }],
                metadata: json!({}),
            },
        ];

        let diagnostics = diagnose_catalog_relations(&entries);

        assert_eq!(diagnostics.total_relations, 3);
        assert_eq!(diagnostics.orphaned_relations, 1);
        assert_eq!(diagnostics.database_mismatched_relations, 1);
        assert_eq!(
            diagnostics.relation_counts_by_kind[relation_kind::RELATIONSHIP_SOURCE_TABLE],
            1
        );
        assert_eq!(
            diagnostics.relation_counts_by_kind[relation_kind::RELATIONSHIP_TARGET_TABLE],
            1
        );
        assert_eq!(
            diagnostics.orphaned_counts_by_kind[relation_kind::KNOWLEDGE_APPLIES_TO],
            1
        );
        assert_eq!(
            diagnostics.database_mismatch_counts_by_kind[relation_kind::RELATIONSHIP_TARGET_TABLE],
            1
        );
        assert!(diagnostics.samples.iter().any(|sample| {
            sample.relation_kind == relation_kind::KNOWLEDGE_APPLIES_TO
                && sample.target_id == "table:missing"
        }));
        assert!(diagnostics.samples.iter().any(|sample| {
            sample.relation_kind == relation_kind::RELATIONSHIP_TARGET_TABLE
                && sample.target_id == "table:products"
        }));
    }

    #[test]
    fn relation_diagnostics_reports_cross_database_materialized_table_relations() {
        let mut orders = table("table:orders", "warehouse");
        orders.links.push(CatalogRelation {
            target_id: "table:products".to_string(),
            kind: relation_kind::REFERENCES_TABLE.to_string(),
            description: None,
        });
        let entries = vec![orders, table("table:products", "other_warehouse")];

        let diagnostics = diagnose_catalog_relations(&entries);

        assert_eq!(diagnostics.total_relations, 1);
        assert_eq!(diagnostics.orphaned_relations, 0);
        assert_eq!(diagnostics.database_mismatched_relations, 1);
        assert_eq!(
            diagnostics.database_mismatch_counts_by_kind[relation_kind::REFERENCES_TABLE],
            1
        );
        assert!(diagnostics.samples.iter().any(|sample| {
            sample.source_id == "table:orders"
                && sample.target_id == "table:products"
                && sample.relation_kind == relation_kind::REFERENCES_TABLE
                && matches!(
                    &sample.issue,
                    CatalogRelationIssue::RelationEndpointDatabaseMismatch {
                        source_database_id,
                        target_database_id,
                    } if source_database_id == "warehouse"
                        && target_database_id == "other_warehouse"
                )
        }));
    }

    #[test]
    fn relation_diagnostics_is_clean_for_consistent_relationships() {
        let entries = vec![
            table("table:orders", "warehouse"),
            table("table:products", "warehouse"),
            relationship("table:orders", "table:products", "warehouse"),
        ];

        let diagnostics = diagnose_catalog_relations(&entries);

        assert_eq!(diagnostics.total_relations, 2);
        assert!(diagnostics.is_clean(), "{diagnostics:?}");
    }
}
