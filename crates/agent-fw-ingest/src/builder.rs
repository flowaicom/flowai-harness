//! Catalog builder — pure functions constructing catalog entries from profiling results.
//!
//! No effects. Every function is `PhysicalTable + Profile + Semantic → Vec<CatalogEntry>`.
//!
//! # Named Parameter Types
//!
//! `CatalogItemPath` bundles `(schema, table, column)` to prevent parameter swaps —
//! all three are `Option<&str>` and trivially interchangeable at call sites.
//!
//! `generate_catalog_id` takes `CatalogKind` (not `&str`) to prevent accidental
//! swaps between `kind` and `database_id`.

use sha2::{Digest, Sha256};

use agent_fw_catalog::{
    provenance_origin, relation_kind, Cardinality, CatalogEntry, CatalogKind, CatalogProvenance,
    CatalogRelation, CategoryValue, ColumnDescriptions, ColumnMetadata, DataQualityFindingMetadata,
    DocumentItem, DocumentMetadata, EnumValueMetadata, ExtractionStatus, ForeignKeyMetadata,
    KnowledgeItem, KnowledgeMetadata, PhysicalTable, QualityNote, RelationshipMetadata,
    SemanticTableProfile, TableMetadata, TableProfile, TypeSpecificStats,
};

pub(crate) const LOW_CARDINALITY_ENUM_THRESHOLD: usize = 50;

// =============================================================================
// CatalogItemPath — bundles (schema, table, column) to prevent swaps
// =============================================================================

/// Location coordinates for a catalog item within the database hierarchy.
#[derive(Clone, Copy, Debug, Default)]
pub struct CatalogItemPath<'a> {
    pub schema: Option<&'a str>,
    pub table: Option<&'a str>,
    pub column: Option<&'a str>,
}

impl<'a> CatalogItemPath<'a> {
    pub fn table(schema: &'a str, table: &'a str) -> Self {
        Self {
            schema: Some(schema),
            table: Some(table),
            column: None,
        }
    }

    pub fn column(schema: &'a str, table: &'a str, column: &'a str) -> Self {
        Self {
            schema: Some(schema),
            table: Some(table),
            column: Some(column),
        }
    }

    pub fn schema_only(schema: &'a str) -> Self {
        Self {
            schema: Some(schema),
            table: None,
            column: None,
        }
    }
}

// =============================================================================
// Pure functions
// =============================================================================

/// Generate a deterministic catalog ID from kind + database + parts.
pub fn generate_catalog_id(kind: CatalogKind, database_id: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_str().as_bytes());
    hasher.update(b":");
    hasher.update(database_id.as_bytes());
    for part in parts {
        hasher.update(b":");
        hasher.update(part.as_bytes());
    }
    hex::encode(hasher.finalize())
}

/// Build tags for a catalog entry.
pub fn build_tags(kind: CatalogKind, path: CatalogItemPath<'_>) -> Vec<String> {
    let mut tags = vec![format!("[TYPE:{}]", kind.as_str())];
    if let Some(s) = path.schema {
        tags.push(format!("[SCHEMA:{}]", s));
    }
    if let Some(t) = path.table {
        tags.push(format!("[TABLE:{}]", t));
    }
    if let Some(c) = path.column {
        tags.push(format!("[COLUMN:{}]", c));
    }
    tags
}

fn metadata_value<T: serde::Serialize>(metadata: T) -> serde_json::Value {
    serde_json::to_value(metadata).expect("typed catalog metadata serializes")
}

fn non_negative_u64(value: i64) -> Option<u64> {
    u64::try_from(value).ok()
}

fn preferred_query_surface(table_name: &str) -> bool {
    table_name.starts_with("v_") || table_name.contains("denormalized")
}

fn normalize_enum_value(value: &str) -> String {
    value.trim().to_lowercase()
}

fn compact_quality_note_name(value: &str) -> String {
    let mut normalized = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
        } else if !normalized.ends_with('_') {
            normalized.push('_');
        }
    }
    let trimmed = normalized.trim_matches('_');
    if trimmed.is_empty() {
        "table".to_string()
    } else {
        trimmed.to_string()
    }
}

fn relationship_join_description(
    source_table: &str,
    target_table: &str,
    join_pair: Option<&agent_fw_catalog::JoinPair>,
) -> Option<String> {
    Some(match join_pair {
        Some(pair) => format!(
            "{}.{} -> {}.{}",
            source_table, pair.source_column, target_table, pair.target_column
        ),
        None => format!("{source_table} -> {target_table}"),
    })
}

fn physical_fk_join_description(
    source_table: &str,
    source_column: &str,
    target_table: &str,
    target_column: &str,
) -> Option<String> {
    Some(format!(
        "{source_table}.{source_column} -> {target_table}.{target_column}"
    ))
}

fn push_unique_link(links: &mut Vec<CatalogRelation>, link: CatalogRelation) {
    if links
        .iter()
        .any(|existing| existing.kind == link.kind && existing.target_id == link.target_id)
    {
        return;
    }
    links.push(link);
}

fn emits_enum_values(
    column: &agent_fw_catalog::ColumnInfo,
    profile: Option<&agent_fw_catalog::ColumnProfile>,
) -> bool {
    if column.is_primary_key {
        return false;
    }

    matches!(
        profile.map(|profile| &profile.stats),
        Some(TypeSpecificStats::Categorical { top_values })
            if !top_values.is_empty() && top_values.len() <= LOW_CARDINALITY_ENUM_THRESHOLD
    )
}

/// Build a catalog entry for a table.
pub fn build_table_entry(
    physical: &PhysicalTable,
    semantic: &SemanticTableProfile,
    database_id: &str,
) -> CatalogEntry {
    build_table_entry_with_provenance(
        physical,
        semantic,
        database_id,
        CatalogProvenance::default(),
    )
}

/// Build a catalog entry for a table with profiling provenance.
pub fn build_table_entry_with_provenance(
    physical: &PhysicalTable,
    semantic: &SemanticTableProfile,
    database_id: &str,
    source: CatalogProvenance,
) -> CatalogEntry {
    let id = generate_catalog_id(
        CatalogKind::Table,
        database_id,
        &[&physical.schema_name, &physical.table_name],
    );
    let qualified_name = format!("{}.{}", physical.schema_name, physical.table_name);
    let column_names: Vec<&str> = physical
        .columns
        .iter()
        .map(|c| c.column_name.as_str())
        .collect();
    let preferred_query_surface = preferred_query_surface(&physical.table_name);
    let mut links: Vec<CatalogRelation> = physical
        .columns
        .iter()
        .map(|c| CatalogRelation {
            target_id: generate_catalog_id(
                CatalogKind::Column,
                database_id,
                &[&physical.schema_name, &physical.table_name, &c.column_name],
            ),
            kind: relation_kind::HAS_COLUMN.to_string(),
            description: Some(format!("Column {}", c.column_name)),
        })
        .collect();

    for relationship in &semantic.relationships {
        let (source_schema, source_table) =
            relationship_table_parts(&physical.schema_name, &relationship.source_table);
        let (target_schema, target_table) =
            relationship_table_parts(&physical.schema_name, &relationship.target_table);

        if source_schema == physical.schema_name && source_table == physical.table_name {
            push_unique_link(
                &mut links,
                CatalogRelation {
                    target_id: generate_catalog_id(
                        CatalogKind::Table,
                        database_id,
                        &[&target_schema, &target_table],
                    ),
                    kind: relation_kind::REFERENCES_TABLE.to_string(),
                    description: relationship_join_description(
                        &source_table,
                        &target_table,
                        relationship.join_columns.first(),
                    ),
                },
            );
        }
        if target_schema == physical.schema_name && target_table == physical.table_name {
            push_unique_link(
                &mut links,
                CatalogRelation {
                    target_id: generate_catalog_id(
                        CatalogKind::Table,
                        database_id,
                        &[&source_schema, &source_table],
                    ),
                    kind: relation_kind::REFERENCED_BY_TABLE.to_string(),
                    description: relationship_join_description(
                        &source_table,
                        &target_table,
                        relationship.join_columns.first(),
                    ),
                },
            );
        }
    }

    for column in &physical.columns {
        let Some(fk) = &column.foreign_key else {
            continue;
        };
        push_unique_link(
            &mut links,
            CatalogRelation {
                target_id: generate_catalog_id(
                    CatalogKind::Table,
                    database_id,
                    &[&fk.referenced_schema, &fk.referenced_table],
                ),
                kind: relation_kind::REFERENCES_TABLE.to_string(),
                description: physical_fk_join_description(
                    &physical.table_name,
                    &column.column_name,
                    &fk.referenced_table,
                    &fk.referenced_column,
                ),
            },
        );
    }

    links.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.target_id.cmp(&b.target_id))
    });

    CatalogEntry {
        id,
        kind: CatalogKind::Table,
        name: physical.table_name.clone(),
        qualified_name: Some(qualified_name),
        content: format!(
            "{}\n\nShort: {}\n\nColumns: {}\nRow count: {}",
            semantic.description,
            semantic.short_description,
            column_names.join(", "),
            physical.row_count,
        ),
        tags: build_tags(
            CatalogKind::Table,
            CatalogItemPath::table(&physical.schema_name, &physical.table_name),
        ),
        links,
        metadata: metadata_value(TableMetadata {
            database_id: database_id.to_string(),
            schema_name: physical.schema_name.clone(),
            table_name: physical.table_name.clone(),
            relation_type: Some(if physical.table_name.starts_with("v_") {
                "view".to_string()
            } else {
                "base_table".to_string()
            }),
            row_count: Some(physical.row_count),
            column_count: Some(physical.columns.len()),
            preferred_query_surface,
            source,
        }),
    }
}

/// Build catalog entries for all columns in a table.
pub fn build_column_entries(
    physical: &PhysicalTable,
    semantic: &SemanticTableProfile,
    profile: &TableProfile,
    database_id: &str,
) -> Vec<CatalogEntry> {
    let table_id = generate_catalog_id(
        CatalogKind::Table,
        database_id,
        &[&physical.schema_name, &physical.table_name],
    );

    physical
        .columns
        .iter()
        .map(|col| {
            let col_id = generate_catalog_id(
                CatalogKind::Column,
                database_id,
                &[
                    &physical.schema_name,
                    &physical.table_name,
                    &col.column_name,
                ],
            );
            let description = semantic
                .column_descriptions
                .get(&col.column_name)
                .cloned()
                .unwrap_or_default();
            let col_profile = profile
                .columns
                .iter()
                .find(|cp| cp.column_name == col.column_name);

            let mut content = format!(
                "{}\n\nType: {}, Nullable: {}",
                description, col.data_type, col.is_nullable
            );
            if col.is_primary_key {
                content.push_str(", Primary Key");
            }
            if let Some(fk) = &col.foreign_key {
                content.push_str(&format!(
                    ", FK -> {}.{}",
                    fk.referenced_table, fk.referenced_column
                ));
            }
            if let Some(cp) = col_profile {
                content.push_str(&format!(
                    "\nNull: {}/{}, Distinct: {}",
                    cp.null_count, cp.total_count, cp.distinct_count
                ));
            }

            CatalogEntry {
                id: col_id,
                kind: CatalogKind::Column,
                name: col.column_name.clone(),
                qualified_name: Some(format!(
                    "{}.{}.{}",
                    physical.schema_name, physical.table_name, col.column_name
                )),
                content,
                tags: build_tags(
                    CatalogKind::Column,
                    CatalogItemPath::column(
                        &physical.schema_name,
                        &physical.table_name,
                        &col.column_name,
                    ),
                ),
                links: {
                    let mut rels = vec![CatalogRelation {
                        target_id: table_id.clone(),
                        kind: relation_kind::BELONGS_TO.to_string(),
                        description: Some(format!("Column of {}", physical.table_name)),
                    }];
                    if let Some(fk) = &col.foreign_key {
                        rels.push(CatalogRelation {
                            target_id: generate_catalog_id(
                                CatalogKind::Table,
                                database_id,
                                &[&fk.referenced_schema, &fk.referenced_table],
                            ),
                            kind: relation_kind::REFERENCES.to_string(),
                            description: Some(format!(
                                "FK -> {}.{}",
                                fk.referenced_table, fk.referenced_column
                            )),
                        });
                    }
                    rels
                },
                metadata: metadata_value(ColumnMetadata {
                    database_id: database_id.to_string(),
                    schema_name: physical.schema_name.clone(),
                    table_name: physical.table_name.clone(),
                    column_name: col.column_name.clone(),
                    data_type: col.data_type.clone(),
                    nullable: col.is_nullable,
                    primary_key: col.is_primary_key,
                    foreign_key: col.foreign_key.as_ref().map(|fk| ForeignKeyMetadata {
                        referenced_schema: fk.referenced_schema.clone(),
                        referenced_table: fk.referenced_table.clone(),
                        referenced_column: fk.referenced_column.clone(),
                        constraint_name: Some(fk.constraint_name.clone()),
                    }),
                    semantic_type: col_profile.map(|cp| cp.semantic_type.to_string()),
                    distinct_count: col_profile.and_then(|cp| non_negative_u64(cp.distinct_count)),
                    null_count: col_profile.and_then(|cp| non_negative_u64(cp.null_count)),
                    total_count: col_profile.and_then(|cp| non_negative_u64(cp.total_count)),
                    low_cardinality_enum: emits_enum_values(col, col_profile),
                }),
            }
        })
        .collect()
}

/// Build catalog entries for inferred relationships.
pub fn build_relationship_entries(
    semantic: &SemanticTableProfile,
    schema: &str,
    database_id: &str,
) -> Vec<CatalogEntry> {
    build_relationship_entries_with_provenance(
        semantic,
        schema,
        database_id,
        provenance_with_origin(
            CatalogProvenance::default(),
            provenance_origin::LLM_ENRICHMENT,
        ),
    )
}

/// Build catalog entries for inferred relationships with explicit provenance.
pub fn build_relationship_entries_with_provenance(
    semantic: &SemanticTableProfile,
    schema: &str,
    database_id: &str,
    source: CatalogProvenance,
) -> Vec<CatalogEntry> {
    let source = ensure_provenance_origin(source, provenance_origin::LLM_ENRICHMENT);
    semantic
        .relationships
        .iter()
        .map(|rel| {
            let (source_schema, source_table) = relationship_table_parts(schema, &rel.source_table);
            let (target_schema, target_table) = relationship_table_parts(schema, &rel.target_table);
            let source_display = format!("{source_schema}.{source_table}");
            let target_display = format!("{target_schema}.{target_table}");
            let source_table_id = generate_catalog_id(
                CatalogKind::Table,
                database_id,
                &[&source_schema, &source_table],
            );
            let target_table_id = generate_catalog_id(
                CatalogKind::Table,
                database_id,
                &[&target_schema, &target_table],
            );
            let join_pair = rel.join_columns.first();
            let id = generate_catalog_id(
                CatalogKind::Relationship,
                database_id,
                &[
                    &source_schema,
                    &source_table,
                    &target_schema,
                    &target_table,
                    rel.relationship_type.as_str(),
                ],
            );
            CatalogEntry {
                id,
                kind: CatalogKind::Relationship,
                name: format!("{source_display} -> {target_display}"),
                qualified_name: None,
                content: format!(
                    "{}: {source_display} -> {target_display} ({})",
                    rel.relationship_type, rel.description
                ),
                tags: build_tags(
                    CatalogKind::Relationship,
                    CatalogItemPath::schema_only(&source_schema),
                ),
                links: vec![
                    CatalogRelation {
                        target_id: source_table_id.clone(),
                        kind: relation_kind::RELATIONSHIP_SOURCE_TABLE.to_string(),
                        description: Some(format!("Source: {source_display}")),
                    },
                    CatalogRelation {
                        target_id: target_table_id.clone(),
                        kind: relation_kind::RELATIONSHIP_TARGET_TABLE.to_string(),
                        description: Some(format!("Target: {target_display}")),
                    },
                ],
                metadata: metadata_value(RelationshipMetadata {
                    database_id: database_id.to_string(),
                    source_table_id,
                    target_table_id,
                    source_schema,
                    source_table,
                    source_column: join_pair
                        .map(|pair| pair.source_column.clone())
                        .unwrap_or_default(),
                    target_schema,
                    target_table,
                    target_column: join_pair
                        .map(|pair| pair.target_column.clone())
                        .unwrap_or_default(),
                    source_cardinality: Cardinality::Many,
                    target_cardinality: Cardinality::One,
                    relationship_kind: rel.relationship_type.to_string(),
                    confidence: Some(1.0),
                    source: source.clone(),
                }),
            }
        })
        .collect()
}

fn provenance_with_origin(mut source: CatalogProvenance, origin: &str) -> CatalogProvenance {
    source.origin = Some(origin.to_string());
    source
}

fn ensure_provenance_origin(source: CatalogProvenance, origin: &str) -> CatalogProvenance {
    if source.origin.is_some() {
        source
    } else {
        provenance_with_origin(source, origin)
    }
}

fn relationship_table_parts(default_schema: &str, table_ref: &str) -> (String, String) {
    table_scope_parts(Some(default_schema), table_ref)
        .unwrap_or_else(|| (default_schema.to_string(), table_ref.trim().to_string()))
}

/// Build relationship entries for physical foreign keys discovered from the
/// target database. These are deterministic schema facts and do not depend on
/// semantic enrichment restating the same relationship.
pub fn build_physical_relationship_entries(
    physical: &PhysicalTable,
    database_id: &str,
) -> Vec<CatalogEntry> {
    build_physical_relationship_entries_with_provenance(
        physical,
        database_id,
        provenance_with_origin(
            CatalogProvenance::default(),
            provenance_origin::PHYSICAL_SCHEMA,
        ),
    )
}

/// Build physical relationship entries with explicit schema/profiling provenance.
pub fn build_physical_relationship_entries_with_provenance(
    physical: &PhysicalTable,
    database_id: &str,
    source: CatalogProvenance,
) -> Vec<CatalogEntry> {
    let source = ensure_provenance_origin(source, provenance_origin::PHYSICAL_SCHEMA);
    physical
        .columns
        .iter()
        .filter_map(|column| {
            let fk = column.foreign_key.as_ref()?;
            let source_table_id = generate_catalog_id(
                CatalogKind::Table,
                database_id,
                &[&physical.schema_name, &physical.table_name],
            );
            let target_table_id = generate_catalog_id(
                CatalogKind::Table,
                database_id,
                &[&fk.referenced_schema, &fk.referenced_table],
            );
            let id = generate_catalog_id(
                CatalogKind::Relationship,
                database_id,
                &[
                    &physical.schema_name,
                    &physical.table_name,
                    &column.column_name,
                    &fk.referenced_schema,
                    &fk.referenced_table,
                    &fk.referenced_column,
                    &fk.constraint_name,
                ],
            );

            Some(CatalogEntry {
                id,
                kind: CatalogKind::Relationship,
                name: format!("{} -> {}", physical.table_name, fk.referenced_table),
                qualified_name: None,
                content: format!(
                    "foreign_key: {}.{} -> {}.{}",
                    physical.table_name,
                    column.column_name,
                    fk.referenced_table,
                    fk.referenced_column
                ),
                tags: build_tags(
                    CatalogKind::Relationship,
                    CatalogItemPath::schema_only(&physical.schema_name),
                ),
                links: vec![
                    CatalogRelation {
                        target_id: source_table_id.clone(),
                        kind: relation_kind::RELATIONSHIP_SOURCE_TABLE.to_string(),
                        description: Some(format!("Source: {}", physical.table_name)),
                    },
                    CatalogRelation {
                        target_id: target_table_id.clone(),
                        kind: relation_kind::RELATIONSHIP_TARGET_TABLE.to_string(),
                        description: Some(format!("Target: {}", fk.referenced_table)),
                    },
                ],
                metadata: metadata_value(RelationshipMetadata {
                    database_id: database_id.to_string(),
                    source_table_id,
                    target_table_id,
                    source_schema: physical.schema_name.clone(),
                    source_table: physical.table_name.clone(),
                    source_column: column.column_name.clone(),
                    target_schema: fk.referenced_schema.clone(),
                    target_table: fk.referenced_table.clone(),
                    target_column: fk.referenced_column.clone(),
                    source_cardinality: Cardinality::Many,
                    target_cardinality: Cardinality::One,
                    relationship_kind: "foreign_key".to_string(),
                    confidence: Some(1.0),
                    source: source.clone(),
                }),
            })
        })
        .collect()
}

/// Build catalog entries for extracted enum values.
///
/// `detected_patterns` maps column names to their detected text patterns.
/// The caller computes patterns (e.g. via `detect_pattern_from_values`)
/// and passes them explicitly — the builder only assembles, never detects.
///
/// This keeps the builder pure and honest: every dependency is in the signature.
pub fn build_enum_entries(
    enums: &std::collections::HashMap<String, Vec<CategoryValue>>,
    detected_patterns: &std::collections::HashMap<String, crate::profiling::TextPattern>,
    table_name: &str,
    schema_name: &str,
    database_id: &str,
) -> Vec<CatalogEntry> {
    enums
        .iter()
        .flat_map(|(column_name, values)| {
            let column_id = generate_catalog_id(
                CatalogKind::Column,
                database_id,
                &[schema_name, table_name, column_name],
            );
            values.iter().enumerate().map(move |(index, value)| {
                let id = generate_catalog_id(
                    CatalogKind::Enum,
                    database_id,
                    &[schema_name, table_name, column_name, &value.value],
                );
                let pattern = detected_patterns.get(column_name).map(|p| p.as_str());

                CatalogEntry {
                    id,
                    kind: CatalogKind::Enum,
                    name: value.value.clone(),
                    qualified_name: Some(format!(
                        "{}.{}.{}.{}",
                        schema_name, table_name, column_name, value.value
                    )),
                    content: match pattern {
                        Some(pattern) => format!(
                            "Enum value {} for {}.{} (count {}, {:.2}%, pattern {})",
                            value.value,
                            table_name,
                            column_name,
                            value.count,
                            value.percentage,
                            pattern
                        ),
                        None => format!(
                            "Enum value {} for {}.{} (count {}, {:.2}%)",
                            value.value, table_name, column_name, value.count, value.percentage
                        ),
                    },
                    tags: build_tags(
                        CatalogKind::Enum,
                        CatalogItemPath::column(schema_name, table_name, column_name),
                    ),
                    links: vec![CatalogRelation {
                        target_id: column_id.clone(),
                        kind: relation_kind::ENUM_VALUE_OF.to_string(),
                        description: Some(format!("Enum value of {}.{}", table_name, column_name)),
                    }],
                    metadata: metadata_value(EnumValueMetadata {
                        database_id: database_id.to_string(),
                        schema_name: schema_name.to_string(),
                        table_name: table_name.to_string(),
                        column_name: column_name.clone(),
                        column_id: column_id.clone(),
                        value: value.value.clone(),
                        normalized_value: normalize_enum_value(&value.value),
                        display_value: Some(value.value.clone()),
                        frequency: non_negative_u64(value.count),
                        frequency_percentage: Some(value.percentage),
                        rank: Some((index + 1) as u32),
                        synonyms: vec![],
                    }),
                }
            })
        })
        .collect()
}

/// Convenience: detect patterns for all enum columns then build entries.
///
/// Composes `detect_pattern_from_values` with `build_enum_entries` for
/// callers who want the default detection behavior.
pub fn build_enum_entries_with_detection(
    enums: &std::collections::HashMap<String, Vec<CategoryValue>>,
    table_name: &str,
    schema_name: &str,
    database_id: &str,
) -> Vec<CatalogEntry> {
    let detected_patterns = detect_enum_patterns(enums);
    build_enum_entries(
        enums,
        &detected_patterns,
        table_name,
        schema_name,
        database_id,
    )
}

/// Detect text patterns for all enum columns. Pure, no IO.
pub fn detect_enum_patterns(
    enums: &std::collections::HashMap<String, Vec<CategoryValue>>,
) -> std::collections::HashMap<String, crate::profiling::TextPattern> {
    enums
        .iter()
        .filter_map(|(col, values)| {
            let refs: Vec<&str> = values.iter().map(|v| v.value.as_str()).collect();
            crate::profiling::detect_pattern_from_values(&refs).map(|p| (col.clone(), p))
        })
        .collect()
}

/// Build a catalog entry for a document.
pub fn build_document_entry(doc: &DocumentItem, database_id: &str) -> CatalogEntry {
    let id = generate_catalog_id(CatalogKind::Document, database_id, &[&doc.id]);
    let content_available = !doc.content.is_empty();
    CatalogEntry {
        id,
        kind: CatalogKind::Document,
        name: doc.name.clone(),
        qualified_name: None,
        content: String::new(),
        tags: build_tags(CatalogKind::Document, CatalogItemPath::default()),
        links: vec![],
        metadata: metadata_value(DocumentMetadata {
            source_document_id: doc.id.clone(),
            content_available,
            content_source: content_available.then(|| "kv".to_string()),
            extraction_status: Some(extraction_status_as_str(doc.extraction_status).to_string()),
            extracted_knowledge_ids: doc.extracted_knowledge_ids.clone(),
        }),
    }
}

/// Build catalog entries for extracted knowledge.
pub fn build_knowledge_entries(
    items: &[KnowledgeItem],
    database_id: &str,
    source_doc_id: Option<&str>,
    schema: Option<&str>,
) -> Vec<CatalogEntry> {
    build_knowledge_entries_with_namespaces(items, database_id, database_id, source_doc_id, schema)
}

/// Build extracted knowledge entries using separate namespaces for knowledge
/// nodes and the schema targets they apply to.
pub fn build_knowledge_entries_with_namespaces(
    items: &[KnowledgeItem],
    entity_database_id: &str,
    target_database_id: &str,
    source_doc_id: Option<&str>,
    schema: Option<&str>,
) -> Vec<CatalogEntry> {
    items
        .iter()
        .map(|ki| {
            let id = generate_catalog_id(CatalogKind::Knowledge, entity_database_id, &[&ki.id]);
            let mut tags = build_tags(CatalogKind::Knowledge, CatalogItemPath::default());
            tags.push(format!("[KNOWLEDGE_TYPE:{}]", ki.knowledge_type.as_str()));
            for table in &ki.scope_tables {
                tags.push(format!("[SCOPE_TABLE:{}]", table));
            }

            let effective_source_doc_id = source_doc_id.or(ki.source_document_id.as_deref());
            let mut links = vec![];
            if let Some(doc_id) = effective_source_doc_id {
                links.push(CatalogRelation {
                    target_id: generate_catalog_id(
                        CatalogKind::Document,
                        entity_database_id,
                        &[doc_id],
                    ),
                    kind: relation_kind::EXTRACTED_FROM.to_string(),
                    description: Some("Extracted from document".to_string()),
                });
            }
            for table in &ki.scope_tables {
                if let Some((schema_name, table_name)) = table_scope_parts(schema, table) {
                    push_unique_link(
                        &mut links,
                        CatalogRelation {
                            target_id: generate_catalog_id(
                                CatalogKind::Table,
                                target_database_id,
                                &[&schema_name, &table_name],
                            ),
                            kind: relation_kind::KNOWLEDGE_APPLIES_TO.to_string(),
                            description: Some(format!("Applies to table {}", table)),
                        },
                    );
                }
            }
            for column in &ki.scope_columns {
                if let Some((schema_name, table_name, column_name)) =
                    column_scope_parts(schema, column)
                {
                    push_unique_link(
                        &mut links,
                        CatalogRelation {
                            target_id: generate_catalog_id(
                                CatalogKind::Column,
                                target_database_id,
                                &[&schema_name, &table_name, &column_name],
                            ),
                            kind: relation_kind::KNOWLEDGE_APPLIES_TO.to_string(),
                            description: Some(format!("Applies to column {}", column)),
                        },
                    );
                }
            }

            CatalogEntry {
                id,
                kind: CatalogKind::Knowledge,
                name: ki.name.clone(),
                qualified_name: None,
                content: ki.description.clone(),
                tags,
                links,
                metadata: metadata_value(KnowledgeMetadata {
                    knowledge_type: Some(ki.knowledge_type.as_str().to_string()),
                    scope_tables: ki.scope_tables.clone(),
                    scope_columns: ki.scope_columns.clone(),
                    sql_expression: ki.sql_expression.clone(),
                    synonyms: ki.synonyms.clone(),
                    source_knowledge_id: Some(ki.id.clone()),
                    source_document_id: effective_source_doc_id.map(str::to_string),
                }),
            }
        })
        .collect()
}

fn extraction_status_as_str(status: ExtractionStatus) -> &'static str {
    match status {
        ExtractionStatus::Pending => "pending",
        ExtractionStatus::Processing => "processing",
        ExtractionStatus::Processed => "processed",
        ExtractionStatus::Failed => "failed",
    }
}

fn table_scope_parts(default_schema: Option<&str>, table_ref: &str) -> Option<(String, String)> {
    let table_ref = table_ref.trim();
    if table_ref.is_empty() {
        return None;
    }
    if let Some((schema_name, table_name)) = table_ref.split_once('.') {
        let schema_name = schema_name.trim();
        let table_name = table_name.trim();
        if !schema_name.is_empty() && !table_name.is_empty() {
            return Some((schema_name.to_string(), table_name.to_string()));
        }
    }
    default_schema
        .map(str::trim)
        .filter(|schema_name| !schema_name.is_empty())
        .map(|schema_name| (schema_name.to_string(), table_ref.to_string()))
}

fn column_scope_parts(
    default_schema: Option<&str>,
    column_ref: &str,
) -> Option<(String, String, String)> {
    let parts: Vec<&str> = column_ref
        .split('.')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    match parts.as_slice() {
        [schema_name, table_name, column_name] => Some((
            (*schema_name).to_string(),
            (*table_name).to_string(),
            (*column_name).to_string(),
        )),
        [table_name, column_name] => default_schema
            .map(str::trim)
            .filter(|schema_name| !schema_name.is_empty())
            .map(|schema_name| {
                (
                    schema_name.to_string(),
                    (*table_name).to_string(),
                    (*column_name).to_string(),
                )
            }),
        _ => None,
    }
}

/// Build durable data quality finding entries from parsed enrichment quality notes.
pub fn build_quality_note_entries(
    physical: &PhysicalTable,
    semantic: &SemanticTableProfile,
    database_id: &str,
    source: CatalogProvenance,
) -> Vec<CatalogEntry> {
    semantic
        .quality_notes
        .iter()
        .enumerate()
        .filter_map(|(idx, note)| {
            let notes = note.notes.trim();
            if notes.is_empty() {
                return None;
            }

            Some(build_quality_note_entry(
                physical,
                note,
                database_id,
                source.clone(),
                idx,
                notes,
            ))
        })
        .collect()
}

fn build_quality_note_entry(
    physical: &PhysicalTable,
    note: &QualityNote,
    database_id: &str,
    source: CatalogProvenance,
    idx: usize,
    notes: &str,
) -> CatalogEntry {
    let table_ref = format!("{}.{}", physical.schema_name, physical.table_name);
    let column_exists = physical
        .columns
        .iter()
        .any(|column| column.column_name == note.column_name);
    let is_table_note =
        note.column_name == "*" || note.column_name.trim().is_empty() || !column_exists;
    let scope_columns = if is_table_note {
        Vec::new()
    } else {
        vec![format!(
            "{}.{}.{}",
            physical.schema_name, physical.table_name, note.column_name
        )]
    };
    let target_id = if is_table_note {
        generate_catalog_id(
            CatalogKind::Table,
            database_id,
            &[&physical.schema_name, &physical.table_name],
        )
    } else {
        generate_catalog_id(
            CatalogKind::Column,
            database_id,
            &[
                &physical.schema_name,
                &physical.table_name,
                &note.column_name,
            ],
        )
    };

    let mut content = format!("Data quality note for {table_ref}: {notes}");
    if !is_table_note {
        content = format!(
            "Data quality note for {}.{}: {notes}",
            table_ref, note.column_name
        );
    }
    if let Some(range) = note
        .typical_value_range
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        content.push_str(&format!("\nTypical value range: {range}"));
    }
    if !note.validation_rules.is_empty() {
        content.push_str(&format!(
            "\nValidation rules: {}",
            note.validation_rules.join("; ")
        ));
    }

    let idx_part = idx.to_string();
    let note_name_part = compact_quality_note_name(notes);
    let mut id_parts = vec![
        physical.schema_name.as_str(),
        physical.table_name.as_str(),
        note.column_name.as_str(),
        idx_part.as_str(),
        note_name_part.as_str(),
    ];
    if is_table_note {
        id_parts[2] = "*";
    }

    let mut tags = build_tags(
        CatalogKind::DataQualityFinding,
        CatalogItemPath::table(&physical.schema_name, &physical.table_name),
    );
    tags.push("[FINDING_TYPE:data_quality]".to_string());
    if !is_table_note {
        tags.push(format!("[COLUMN:{}]", note.column_name));
    }

    CatalogEntry {
        id: generate_catalog_id(CatalogKind::DataQualityFinding, database_id, &id_parts),
        kind: CatalogKind::DataQualityFinding,
        name: if is_table_note {
            format!("Data quality finding: {}", physical.table_name)
        } else {
            format!(
                "Data quality finding: {}.{}",
                physical.table_name, note.column_name
            )
        },
        qualified_name: None,
        content,
        tags,
        links: vec![CatalogRelation {
            target_id,
            kind: relation_kind::DATA_QUALITY_FINDING_APPLIES_TO.to_string(),
            description: Some(if is_table_note {
                format!(
                    "Data quality finding applies to table {}",
                    physical.table_name
                )
            } else {
                format!(
                    "Data quality finding applies to column {}",
                    note.column_name
                )
            }),
        }],
        metadata: metadata_value(DataQualityFindingMetadata {
            database_id: database_id.to_string(),
            schema_name: physical.schema_name.clone(),
            table_name: physical.table_name.clone(),
            column_name: if is_table_note {
                None
            } else {
                Some(note.column_name.clone())
            },
            finding_type: Some("data_quality".to_string()),
            scope_tables: vec![table_ref],
            scope_columns,
            source,
            typical_value_range: note.typical_value_range.clone(),
            validation_rules: note.validation_rules.clone(),
        }),
    }
}

/// Derive a minimal semantic profile from physical schema when enrichment fails.
pub fn fallback_semantic_profile(
    physical: &PhysicalTable,
    schema: &str,
    table: &str,
) -> SemanticTableProfile {
    let mut column_descriptions = ColumnDescriptions::new();
    for c in &physical.columns {
        let desc = format!(
            "{} ({}{})",
            c.column_name,
            c.data_type,
            if c.is_nullable { ", nullable" } else { "" },
        );
        column_descriptions.insert(c.column_name.clone(), desc);
    }
    SemanticTableProfile {
        description: format!(
            "Table {} in schema {} [enrichment unavailable]",
            table, schema
        ),
        short_description: table.to_string(),
        column_descriptions,
        relationships: vec![],
        quality_notes: vec![],
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_catalog::{
        provenance_origin, relation_kind, Cardinality, ColumnInfo, ColumnMetadata,
        EnumValueMetadata, ForeignKeyRef, RelationshipKind, RelationshipMetadata, SemanticType,
        TableMetadata, TypeSpecificStats,
    };

    #[test]
    fn generate_catalog_id_deterministic() {
        let id1 = generate_catalog_id(CatalogKind::Table, "db1", &["public", "bookings"]);
        let id2 = generate_catalog_id(CatalogKind::Table, "db1", &["public", "bookings"]);
        assert_eq!(id1, id2);
    }

    #[test]
    fn generate_catalog_id_different_inputs() {
        let id1 = generate_catalog_id(CatalogKind::Table, "db1", &["public", "bookings"]);
        let id2 = generate_catalog_id(CatalogKind::Table, "db1", &["public", "flights"]);
        assert_ne!(id1, id2);
    }

    #[test]
    fn generate_catalog_id_different_kinds() {
        let id1 = generate_catalog_id(CatalogKind::Table, "db1", &["public", "bookings"]);
        let id2 = generate_catalog_id(CatalogKind::Column, "db1", &["public", "bookings"]);
        assert_ne!(
            id1, id2,
            "different CatalogKind should produce different IDs"
        );
    }

    #[test]
    fn build_tags_full() {
        let tags = build_tags(
            CatalogKind::Column,
            CatalogItemPath::column("public", "bookings", "id"),
        );
        assert_eq!(tags.len(), 4);
        assert!(tags.contains(&"[TYPE:column]".to_string()));
        assert!(tags.contains(&"[SCHEMA:public]".to_string()));
        assert!(tags.contains(&"[TABLE:bookings]".to_string()));
        assert!(tags.contains(&"[COLUMN:id]".to_string()));
    }

    #[test]
    fn build_tags_partial() {
        let tags = build_tags(CatalogKind::Table, CatalogItemPath::schema_only("public"));
        assert_eq!(tags.len(), 2);
    }

    #[test]
    fn build_document_entry_emits_typed_document_metadata() {
        let document = DocumentItem {
            id: "doc-1".to_string(),
            name: "Guide".to_string(),
            content: "Full document body".to_string(),
            target_database_id: None,
            extraction_status: ExtractionStatus::Processed,
            extracted_knowledge_ids: vec!["knowledge-1".to_string()],
            created_at: "2026-05-29T00:00:00Z".to_string(),
        };

        let entry = build_document_entry(&document, "warehouse");
        let metadata: DocumentMetadata = serde_json::from_value(entry.metadata.clone()).unwrap();

        assert_eq!(metadata.source_document_id, "doc-1");
        assert!(metadata.content_available);
        assert_eq!(metadata.content_source.as_deref(), Some("kv"));
        assert_eq!(metadata.extraction_status.as_deref(), Some("processed"));
        assert_eq!(metadata.extracted_knowledge_ids, vec!["knowledge-1"]);
        assert!(
            entry.content.is_empty(),
            "catalog Document content should remain empty until a real summary is projected"
        );
    }

    #[test]
    fn build_knowledge_entries_emit_typed_metadata_and_source_links() {
        let item = KnowledgeItem {
            id: "knowledge-1".to_string(),
            name: "Slow mover threshold".to_string(),
            description: "Slow movers use a velocity ratio below 0.25.".to_string(),
            knowledge_type: agent_fw_catalog::KnowledgeType::BusinessRule,
            scope_tables: vec!["public.fact_scenario".to_string()],
            scope_columns: vec!["fact_scenario.velocity_ratio".to_string()],
            sql_expression: Some("velocity_ratio < 0.25".to_string()),
            synonyms: vec!["slow mover cutoff".to_string()],
            source_document_id: Some("doc-1".to_string()),
        };

        let entries = build_knowledge_entries(&[item], "warehouse", None, Some("public"));
        let entry = entries.into_iter().next().unwrap();
        let metadata: KnowledgeMetadata = serde_json::from_value(entry.metadata.clone()).unwrap();

        assert_eq!(
            entry.content,
            "Slow movers use a velocity ratio below 0.25."
        );
        assert!(entry
            .tags
            .contains(&"[KNOWLEDGE_TYPE:business_rule]".to_string()));
        assert_eq!(metadata.knowledge_type.as_deref(), Some("business_rule"));
        assert_eq!(metadata.scope_tables, vec!["public.fact_scenario"]);
        assert_eq!(metadata.scope_columns, vec!["fact_scenario.velocity_ratio"]);
        assert_eq!(
            metadata.sql_expression.as_deref(),
            Some("velocity_ratio < 0.25")
        );
        assert_eq!(metadata.synonyms, vec!["slow mover cutoff"]);
        assert_eq!(metadata.source_knowledge_id.as_deref(), Some("knowledge-1"));
        assert_eq!(metadata.source_document_id.as_deref(), Some("doc-1"));
        assert!(entry.links.iter().any(|relation| {
            relation.kind == relation_kind::EXTRACTED_FROM
                && relation.target_id
                    == generate_catalog_id(CatalogKind::Document, "warehouse", &["doc-1"])
        }));
        assert!(entry.links.iter().any(|relation| {
            relation.kind == relation_kind::KNOWLEDGE_APPLIES_TO
                && relation.target_id
                    == generate_catalog_id(
                        CatalogKind::Table,
                        "warehouse",
                        &["public", "fact_scenario"],
                    )
        }));
        assert!(entry.links.iter().any(|relation| {
            relation.kind == relation_kind::KNOWLEDGE_APPLIES_TO
                && relation.target_id
                    == generate_catalog_id(
                        CatalogKind::Column,
                        "warehouse",
                        &["public", "fact_scenario", "velocity_ratio"],
                    )
        }));
    }

    #[test]
    fn build_knowledge_entries_can_decouple_entity_and_schema_databases() {
        let item = KnowledgeItem {
            id: "knowledge-1".to_string(),
            name: "Slow mover threshold".to_string(),
            description: "Slow movers use a velocity ratio below 0.25.".to_string(),
            knowledge_type: agent_fw_catalog::KnowledgeType::BusinessRule,
            scope_tables: vec!["public.fact_scenario".to_string()],
            scope_columns: vec!["public.fact_scenario.velocity_ratio".to_string()],
            sql_expression: None,
            synonyms: vec![],
            source_document_id: Some("doc-1".to_string()),
        };

        let entry = build_knowledge_entries_with_namespaces(
            &[item],
            "knowledge:acme:analytics",
            "warehouse",
            None,
            None,
        )
        .into_iter()
        .next()
        .unwrap();

        assert_eq!(
            entry.id,
            generate_catalog_id(
                CatalogKind::Knowledge,
                "knowledge:acme:analytics",
                &["knowledge-1"]
            )
        );
        assert!(entry.links.iter().any(|relation| {
            relation.kind == relation_kind::EXTRACTED_FROM
                && relation.target_id
                    == generate_catalog_id(
                        CatalogKind::Document,
                        "knowledge:acme:analytics",
                        &["doc-1"],
                    )
        }));
        assert!(entry.links.iter().any(|relation| {
            relation.kind == relation_kind::KNOWLEDGE_APPLIES_TO
                && relation.target_id
                    == generate_catalog_id(
                        CatalogKind::Table,
                        "warehouse",
                        &["public", "fact_scenario"],
                    )
        }));
        assert!(entry.links.iter().any(|relation| {
            relation.kind == relation_kind::KNOWLEDGE_APPLIES_TO
                && relation.target_id
                    == generate_catalog_id(
                        CatalogKind::Column,
                        "warehouse",
                        &["public", "fact_scenario", "velocity_ratio"],
                    )
        }));
    }

    #[test]
    fn build_table_entry_has_column_links() {
        let physical = PhysicalTable {
            schema_name: "public".into(),
            table_name: "users".into(),
            columns: vec![agent_fw_catalog::ColumnInfo {
                column_name: "id".into(),
                data_type: "integer".into(),
                is_nullable: false,
                column_default: None,
                ordinal_position: 1,
                is_primary_key: true,
                foreign_key: None,
            }],
            constraints: vec![],
            indexes: vec![],
            row_count: 1000,
        };
        let semantic = SemanticTableProfile {
            description: "User accounts".into(),
            short_description: "Users".into(),
            column_descriptions: ColumnDescriptions::new(),
            relationships: vec![],
            quality_notes: vec![],
        };
        let entry = build_table_entry(&physical, &semantic, "db1");
        assert_eq!(entry.kind, CatalogKind::Table);
        assert_eq!(entry.name, "users");
        assert_eq!(entry.links.len(), 1);
        assert_eq!(entry.links[0].kind, "has_column");
    }

    #[test]
    fn build_table_entry_emits_typed_metadata_and_table_graph_edges() {
        let physical = PhysicalTable {
            schema_name: "public".into(),
            table_name: "fact_sales".into(),
            columns: vec![ColumnInfo {
                column_name: "product_id".into(),
                data_type: "integer".into(),
                is_nullable: false,
                column_default: None,
                ordinal_position: 1,
                is_primary_key: false,
                foreign_key: None,
            }],
            constraints: vec![],
            indexes: vec![],
            row_count: 100,
        };
        let semantic = SemanticTableProfile {
            description: "Sales facts".into(),
            short_description: "Sales".into(),
            column_descriptions: ColumnDescriptions::new(),
            relationships: vec![agent_fw_catalog::InferredRelationship {
                source_table: "fact_sales".into(),
                target_table: "dim_products".into(),
                relationship_type: RelationshipKind::OneToMany,
                join_columns: vec![("product_id".to_string(), "product_id".to_string()).into()],
                description: "Sales reference products".into(),
            }],
            quality_notes: vec![],
        };

        let entry = build_table_entry(&physical, &semantic, "warehouse");
        let metadata: TableMetadata = serde_json::from_value(entry.metadata.clone()).unwrap();

        assert_eq!(metadata.database_id, "warehouse");
        assert_eq!(metadata.schema_name, "public");
        assert_eq!(metadata.table_name, "fact_sales");
        assert_eq!(metadata.relation_type.as_deref(), Some("base_table"));
        assert_eq!(metadata.row_count, Some(100));
        assert_eq!(metadata.column_count, Some(1));
        assert!(!metadata.preferred_query_surface);
        assert!(entry.metadata.get("tenantId").is_none());
        assert!(entry.metadata.get("workspaceId").is_none());
        assert!(entry
            .links
            .iter()
            .any(|rel| rel.kind == relation_kind::HAS_COLUMN));
        assert!(entry
            .links
            .iter()
            .any(|rel| rel.kind == relation_kind::REFERENCES_TABLE));
    }

    #[test]
    fn build_table_entry_normalizes_schema_qualified_relationship_refs() {
        let fact_sales = PhysicalTable {
            schema_name: "public".into(),
            table_name: "fact_sales".into(),
            columns: vec![ColumnInfo {
                column_name: "product_id".into(),
                data_type: "integer".into(),
                is_nullable: false,
                column_default: None,
                ordinal_position: 1,
                is_primary_key: false,
                foreign_key: None,
            }],
            constraints: vec![],
            indexes: vec![],
            row_count: 100,
        };
        let dim_products = PhysicalTable {
            schema_name: "public".into(),
            table_name: "dim_products".into(),
            columns: vec![ColumnInfo {
                column_name: "product_id".into(),
                data_type: "integer".into(),
                is_nullable: false,
                column_default: None,
                ordinal_position: 1,
                is_primary_key: false,
                foreign_key: None,
            }],
            constraints: vec![],
            indexes: vec![],
            row_count: 10,
        };
        let semantic = SemanticTableProfile {
            description: "Sales facts".into(),
            short_description: "Sales".into(),
            column_descriptions: ColumnDescriptions::new(),
            relationships: vec![agent_fw_catalog::InferredRelationship {
                source_table: "public.fact_sales".into(),
                target_table: "public.dim_products".into(),
                relationship_type: RelationshipKind::OneToMany,
                join_columns: vec![("product_id".to_string(), "product_id".to_string()).into()],
                description: "Sales reference products".into(),
            }],
            quality_notes: vec![],
        };

        let fact_entry = build_table_entry(&fact_sales, &semantic, "warehouse");
        let dim_entry = build_table_entry(&dim_products, &semantic, "warehouse");
        let fact_sales_id =
            generate_catalog_id(CatalogKind::Table, "warehouse", &["public", "fact_sales"]);
        let dim_products_id =
            generate_catalog_id(CatalogKind::Table, "warehouse", &["public", "dim_products"]);
        let bad_dim_products_id = generate_catalog_id(
            CatalogKind::Table,
            "warehouse",
            &["public", "public.dim_products"],
        );
        let bad_fact_sales_id = generate_catalog_id(
            CatalogKind::Table,
            "warehouse",
            &["public", "public.fact_sales"],
        );

        assert!(fact_entry.links.iter().any(|rel| {
            rel.kind == relation_kind::REFERENCES_TABLE && rel.target_id == dim_products_id
        }));
        assert!(!fact_entry
            .links
            .iter()
            .any(|rel| rel.target_id == bad_dim_products_id));
        assert!(dim_entry.links.iter().any(|rel| {
            rel.kind == relation_kind::REFERENCED_BY_TABLE && rel.target_id == fact_sales_id
        }));
        assert!(!dim_entry
            .links
            .iter()
            .any(|rel| rel.target_id == bad_fact_sales_id));
    }

    #[test]
    fn build_table_entry_emits_physical_fk_table_graph_edges() {
        let physical = PhysicalTable {
            schema_name: "public".into(),
            table_name: "dim_products".into(),
            columns: vec![ColumnInfo {
                column_name: "brand_id".into(),
                data_type: "integer".into(),
                is_nullable: false,
                column_default: None,
                ordinal_position: 1,
                is_primary_key: false,
                foreign_key: Some(ForeignKeyRef {
                    referenced_schema: "public".into(),
                    referenced_table: "dim_brands".into(),
                    referenced_column: "brand_id".into(),
                    constraint_name: "fk_products_brand".into(),
                }),
            }],
            constraints: vec![],
            indexes: vec![],
            row_count: 100,
        };
        let semantic = SemanticTableProfile {
            description: "Products".into(),
            short_description: "Products".into(),
            column_descriptions: ColumnDescriptions::new(),
            relationships: vec![],
            quality_notes: vec![],
        };

        let entry = build_table_entry(&physical, &semantic, "warehouse");

        assert!(entry.links.iter().any(|rel| {
            rel.kind == relation_kind::REFERENCES_TABLE
                && rel.target_id
                    == generate_catalog_id(
                        CatalogKind::Table,
                        "warehouse",
                        &["public", "dim_brands"],
                    )
        }));
    }

    #[test]
    fn build_physical_relationship_entries_emit_typed_fk_metadata() {
        let physical = PhysicalTable {
            schema_name: "public".into(),
            table_name: "dim_products".into(),
            columns: vec![ColumnInfo {
                column_name: "brand_id".into(),
                data_type: "integer".into(),
                is_nullable: false,
                column_default: None,
                ordinal_position: 1,
                is_primary_key: false,
                foreign_key: Some(ForeignKeyRef {
                    referenced_schema: "public".into(),
                    referenced_table: "dim_brands".into(),
                    referenced_column: "brand_id".into(),
                    constraint_name: "fk_products_brand".into(),
                }),
            }],
            constraints: vec![],
            indexes: vec![],
            row_count: 100,
        };

        let entries = build_physical_relationship_entries(&physical, "warehouse");
        let entry = entries.first().expect("physical FK relationship");
        let metadata: RelationshipMetadata =
            serde_json::from_value(entry.metadata.clone()).unwrap();

        assert_eq!(metadata.source_table, "dim_products");
        assert_eq!(metadata.source_column, "brand_id");
        assert_eq!(metadata.target_table, "dim_brands");
        assert_eq!(metadata.target_column, "brand_id");
        assert_eq!(metadata.relationship_kind, "foreign_key");
        assert_eq!(
            metadata.source.origin.as_deref(),
            Some(provenance_origin::PHYSICAL_SCHEMA)
        );
    }

    #[test]
    fn build_view_table_entry_marks_preferred_query_surface() {
        let physical = PhysicalTable {
            schema_name: "public".into(),
            table_name: "v_scenario_denormalized".into(),
            columns: vec![],
            constraints: vec![],
            indexes: vec![],
            row_count: 7,
        };
        let semantic = SemanticTableProfile {
            description: "Reporting view".into(),
            short_description: "View".into(),
            column_descriptions: ColumnDescriptions::new(),
            relationships: vec![],
            quality_notes: vec![],
        };

        let entry = build_table_entry(&physical, &semantic, "warehouse");
        let metadata: TableMetadata = serde_json::from_value(entry.metadata).unwrap();

        assert_eq!(metadata.relation_type.as_deref(), Some("view"));
        assert!(metadata.preferred_query_surface);
    }

    #[test]
    fn build_column_entries_emit_typed_metadata_and_fk_edges() {
        let physical = PhysicalTable {
            schema_name: "public".into(),
            table_name: "fact_sales".into(),
            columns: vec![
                ColumnInfo {
                    column_name: "product_id".into(),
                    data_type: "integer".into(),
                    is_nullable: false,
                    column_default: None,
                    ordinal_position: 1,
                    is_primary_key: true,
                    foreign_key: Some(ForeignKeyRef {
                        referenced_schema: "public".into(),
                        referenced_table: "dim_products".into(),
                        referenced_column: "product_id".into(),
                        constraint_name: "fk_sales_product".into(),
                    }),
                },
                ColumnInfo {
                    column_name: "order_status".into(),
                    data_type: "text".into(),
                    is_nullable: false,
                    column_default: None,
                    ordinal_position: 2,
                    is_primary_key: false,
                    foreign_key: None,
                },
            ],
            constraints: vec![],
            indexes: vec![],
            row_count: 100,
        };
        let mut column_descriptions = ColumnDescriptions::new();
        column_descriptions.insert("product_id".into(), "Product key".into());
        column_descriptions.insert("order_status".into(), "Order status".into());
        let semantic = SemanticTableProfile {
            description: "Sales facts".into(),
            short_description: "Sales".into(),
            column_descriptions,
            relationships: vec![],
            quality_notes: vec![],
        };
        let profile = TableProfile {
            table_name: "fact_sales".into(),
            columns: vec![
                agent_fw_catalog::ColumnProfile {
                    column_name: "product_id".into(),
                    data_type: "integer".into(),
                    null_count: 0,
                    distinct_count: 10,
                    total_count: 100,
                    semantic_type: SemanticType::Identifier,
                    stats: TypeSpecificStats::Categorical {
                        top_values: vec![CategoryValue {
                            value: "1".into(),
                            count: 10,
                            percentage: 10.0,
                        }],
                    },
                },
                agent_fw_catalog::ColumnProfile {
                    column_name: "order_status".into(),
                    data_type: "text".into(),
                    null_count: 0,
                    distinct_count: 2,
                    total_count: 100,
                    semantic_type: SemanticType::Categorical,
                    stats: TypeSpecificStats::Categorical {
                        top_values: vec![
                            CategoryValue {
                                value: "paid".into(),
                                count: 70,
                                percentage: 70.0,
                            },
                            CategoryValue {
                                value: "refunded".into(),
                                count: 30,
                                percentage: 30.0,
                            },
                        ],
                    },
                },
            ],
        };

        let entries = build_column_entries(&physical, &semantic, &profile, "warehouse");
        let entry = entries
            .iter()
            .find(|entry| entry.name == "product_id")
            .unwrap();
        let metadata: ColumnMetadata = serde_json::from_value(entry.metadata.clone()).unwrap();

        assert_eq!(metadata.database_id, "warehouse");
        assert_eq!(metadata.schema_name, "public");
        assert_eq!(metadata.table_name, "fact_sales");
        assert_eq!(metadata.column_name, "product_id");
        assert!(metadata.primary_key);
        assert_eq!(metadata.distinct_count, Some(10));
        assert_eq!(metadata.null_count, Some(0));
        assert_eq!(metadata.total_count, Some(100));
        assert_eq!(metadata.semantic_type.as_deref(), Some("identifier"));
        assert!(!metadata.low_cardinality_enum);
        let foreign_key = metadata.foreign_key.unwrap();
        assert_eq!(foreign_key.referenced_table, "dim_products");
        assert!(entry.metadata.get("tenantId").is_none());
        assert!(entry.metadata.get("workspaceId").is_none());
        assert!(entry
            .links
            .iter()
            .any(|rel| rel.kind == relation_kind::BELONGS_TO));
        assert!(entry
            .links
            .iter()
            .any(|rel| rel.kind == relation_kind::REFERENCES));

        let status = entries
            .iter()
            .find(|entry| entry.name == "order_status")
            .unwrap();
        let status_metadata: ColumnMetadata =
            serde_json::from_value(status.metadata.clone()).unwrap();
        assert!(status_metadata.low_cardinality_enum);
    }

    #[test]
    fn build_enum_entries_emit_one_typed_enum_value_entry_per_value() {
        let enums = std::collections::HashMap::from([(
            "segment_reference_id".to_string(),
            vec![
                CategoryValue {
                    value: "ice_tea".into(),
                    count: 12,
                    percentage: 0.6,
                },
                CategoryValue {
                    value: "sparkling_water".into(),
                    count: 8,
                    percentage: 0.4,
                },
            ],
        )]);
        let detected_patterns = std::collections::HashMap::new();

        let entries = build_enum_entries(
            &enums,
            &detected_patterns,
            "v_scenario_denormalized",
            "public",
            "warehouse",
        );

        assert_eq!(entries.len(), 2);
        let ice_tea = entries
            .iter()
            .find(|entry| entry.name == "ice_tea")
            .expect("ice_tea enum value entry");
        let metadata: EnumValueMetadata = serde_json::from_value(ice_tea.metadata.clone()).unwrap();
        assert_eq!(ice_tea.kind, CatalogKind::Enum);
        assert_eq!(
            ice_tea.qualified_name.as_deref(),
            Some("public.v_scenario_denormalized.segment_reference_id.ice_tea")
        );
        assert_eq!(metadata.database_id, "warehouse");
        assert_eq!(metadata.table_name, "v_scenario_denormalized");
        assert_eq!(metadata.column_name, "segment_reference_id");
        assert_eq!(metadata.value, "ice_tea");
        assert_eq!(metadata.normalized_value, "ice_tea");
        assert_eq!(metadata.frequency, Some(12));
        assert_eq!(metadata.frequency_percentage, Some(0.6));
        assert_eq!(metadata.rank, Some(1));
        assert!(ice_tea.metadata.get("tenantId").is_none());
        assert!(ice_tea.metadata.get("workspaceId").is_none());
        assert!(ice_tea
            .links
            .iter()
            .any(|rel| rel.kind == relation_kind::ENUM_VALUE_OF));
    }

    #[test]
    fn build_relationship_entries_emit_typed_metadata_and_graph_edges() {
        let semantic = SemanticTableProfile {
            description: "Sales facts".into(),
            short_description: "Sales".into(),
            column_descriptions: ColumnDescriptions::new(),
            relationships: vec![agent_fw_catalog::InferredRelationship {
                source_table: "fact_sales".into(),
                target_table: "dim_products".into(),
                relationship_type: RelationshipKind::OneToMany,
                join_columns: vec![("product_id".to_string(), "product_id".to_string()).into()],
                description: "Sales reference products".into(),
            }],
            quality_notes: vec![],
        };

        let entries = build_relationship_entries_with_provenance(
            &semantic,
            "public",
            "warehouse",
            CatalogProvenance {
                origin: Some(provenance_origin::LLM_ENRICHMENT.to_string()),
                profiling_run_id: Some("profile-1".to_string()),
                enrichment_source: Some("fresh".to_string()),
                model_id: Some("claude-test-model".to_string()),
                fallback_reason: None,
                schema_snapshot_at: None,
                target_fingerprint: None,
            },
        );
        let entry = entries.first().unwrap();
        let metadata: RelationshipMetadata =
            serde_json::from_value(entry.metadata.clone()).unwrap();

        assert_eq!(metadata.database_id, "warehouse");
        assert_eq!(metadata.source_table, "fact_sales");
        assert_eq!(metadata.target_table, "dim_products");
        assert_eq!(metadata.source_column, "product_id");
        assert_eq!(metadata.target_column, "product_id");
        assert_eq!(metadata.source_cardinality, Cardinality::Many);
        assert_eq!(metadata.target_cardinality, Cardinality::One);
        assert_eq!(metadata.relationship_kind, "one-to-many");
        assert_eq!(metadata.confidence, Some(1.0));
        assert_eq!(
            metadata.source.origin.as_deref(),
            Some(provenance_origin::LLM_ENRICHMENT)
        );
        assert_eq!(
            metadata.source.profiling_run_id.as_deref(),
            Some("profile-1")
        );
        assert_eq!(
            metadata.source.model_id.as_deref(),
            Some("claude-test-model")
        );
        assert!(entry.metadata.get("tenantId").is_none());
        assert!(entry.metadata.get("workspaceId").is_none());
        assert!(entry
            .links
            .iter()
            .any(|rel| rel.kind == relation_kind::RELATIONSHIP_SOURCE_TABLE));
        assert!(entry
            .links
            .iter()
            .any(|rel| rel.kind == relation_kind::RELATIONSHIP_TARGET_TABLE));
    }

    #[test]
    fn build_relationship_entries_normalizes_schema_qualified_table_refs() {
        let semantic = SemanticTableProfile {
            description: "Segment hierarchy".into(),
            short_description: "Segments".into(),
            column_descriptions: ColumnDescriptions::new(),
            relationships: vec![agent_fw_catalog::InferredRelationship {
                source_table: "public.dim_subsegments".into(),
                target_table: "public.dim_segments".into(),
                relationship_type: RelationshipKind::OneToMany,
                join_columns: vec![("segment_id".to_string(), "segment_id".to_string()).into()],
                description: "Subsegments roll up to parent segments".into(),
            }],
            quality_notes: vec![],
        };

        let entry = build_relationship_entries(&semantic, "public", "neondb")
            .into_iter()
            .next()
            .unwrap();
        let metadata: RelationshipMetadata =
            serde_json::from_value(entry.metadata.clone()).unwrap();

        let source_table_id =
            generate_catalog_id(CatalogKind::Table, "neondb", &["public", "dim_subsegments"]);
        let target_table_id =
            generate_catalog_id(CatalogKind::Table, "neondb", &["public", "dim_segments"]);
        assert_eq!(metadata.source_schema, "public");
        assert_eq!(metadata.source_table, "dim_subsegments");
        assert_eq!(metadata.source_table_id, source_table_id);
        assert_eq!(metadata.target_schema, "public");
        assert_eq!(metadata.target_table, "dim_segments");
        assert_eq!(metadata.target_table_id, target_table_id);
        assert!(entry
            .links
            .iter()
            .any(|rel| rel.kind == relation_kind::RELATIONSHIP_SOURCE_TABLE
                && rel.target_id == source_table_id));
        assert!(entry
            .links
            .iter()
            .any(|rel| rel.kind == relation_kind::RELATIONSHIP_TARGET_TABLE
                && rel.target_id == target_table_id));
    }

    #[test]
    fn fallback_semantic_profile_generates_column_descriptions() {
        let physical = PhysicalTable {
            schema_name: "public".into(),
            table_name: "orders".into(),
            columns: vec![
                agent_fw_catalog::ColumnInfo {
                    column_name: "id".into(),
                    data_type: "integer".into(),
                    is_nullable: false,
                    column_default: None,
                    ordinal_position: 1,
                    is_primary_key: true,
                    foreign_key: None,
                },
                agent_fw_catalog::ColumnInfo {
                    column_name: "amount".into(),
                    data_type: "numeric".into(),
                    is_nullable: true,
                    column_default: None,
                    ordinal_position: 2,
                    is_primary_key: false,
                    foreign_key: None,
                },
            ],
            constraints: vec![],
            indexes: vec![],
            row_count: 500,
        };
        let profile = fallback_semantic_profile(&physical, "public", "orders");
        assert!(profile.description.contains("enrichment unavailable"));
        assert_eq!(profile.column_descriptions.len(), 2);
        assert!(profile
            .column_descriptions
            .get("amount")
            .unwrap()
            .contains("nullable"));
    }
}
