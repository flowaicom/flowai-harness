use async_trait::async_trait;
use serde_json::json;

use agent_fw_catalog::{
    relation_kind, CatalogEntry, CatalogError, CatalogKind, CatalogRelation, DataCatalog, JoinPath,
};
use agent_fw_catalog_tools::surface::{
    CatalogEntityAssembler, CatalogEntityKind, CatalogFilterRef, CatalogFilterResolver,
    CatalogFilters, CatalogGraphService, CatalogRef, CatalogRefResolver, GetCatalogRelationsInput,
    OutputPolicyRegistry, RelationDirection, ResolvedCatalogRef,
};

#[hegel::test]
fn canonical_ref_one_of_law(tc: hegel::TestCase) {
    let suffix = draw_suffix(&tc);
    let kind = draw_kind(&tc);
    let has_qualified_name = tc.draw(hegel::generators::booleans());
    let qualified_name = has_qualified_name.then(|| format!("public.entity_{suffix}"));
    let resolved = ResolvedCatalogRef {
        id: format!("{}:{suffix}", kind.public_name()),
        kind,
        name: format!("entity_{suffix}"),
        qualified_name: qualified_name.clone(),
    };

    let reference = resolved.catalog_ref();

    assert_eq!(
        reference.provided_reference_count(),
        1,
        "canonical refs must satisfy the CatalogRef one-of contract"
    );
    assert_eq!(reference.kind, Some(kind));
    if let Some(qualified_name) = qualified_name {
        assert_eq!(
            reference.qualified_name.as_deref(),
            Some(qualified_name.as_str())
        );
        assert!(reference.id.is_none());
        assert!(reference.name.is_none());
    } else {
        assert_eq!(reference.id.as_deref(), Some(resolved.id.as_str()));
        assert!(reference.qualified_name.is_none());
        assert!(reference.name.is_none());
    }
}

#[hegel::test]
fn filter_ref_string_hint_one_of_law(tc: hegel::TestCase) {
    let suffix = draw_suffix(&tc);
    let variant = tc.draw(hegel::generators::integers::<u8>().max_value(2));
    let input = match variant {
        0 => format!("table:public.entity_{suffix}"),
        1 => format!("public.entity_{suffix}"),
        _ => format!("entity_{suffix}"),
    };
    let reference =
        CatalogFilterRef::String(input.clone()).to_catalog_ref(CatalogEntityKind::Table);

    assert_eq!(reference.provided_reference_count(), 1);
    assert_eq!(reference.kind, Some(CatalogEntityKind::Table));
    match variant {
        0 => assert_eq!(reference.id.as_deref(), Some(input.as_str())),
        1 => assert_eq!(reference.qualified_name.as_deref(), Some(input.as_str())),
        _ => assert_eq!(reference.name.as_deref(), Some(input.as_str())),
    }
}

#[hegel::test]
fn ambiguous_and_unresolved_filters_are_not_applied_law(tc: hegel::TestCase) {
    let suffix = draw_suffix(&tc);
    let column_name = format!("id_{suffix}");
    let catalog = LawCatalog::new(vec![
        table_entry(suffix, Vec::new()),
        column_entry(suffix, &column_name, "a"),
        column_entry(suffix, &column_name, "b"),
    ]);
    let resolver = CatalogFilterResolver::new(CatalogRefResolver::new(&catalog));

    let resolved = tokio_test::block_on(resolver.resolve(&CatalogFilters {
        column: Some(CatalogFilterRef::String(column_name)),
        relation_kind: Some(format!("not_allowed_relation_{suffix}")),
        ..CatalogFilters::default()
    }))
    .unwrap();

    assert!(resolved.backend_filters.column.is_none());
    assert!(resolved.resolution.applied.column.is_none());
    assert!(resolved
        .resolution
        .ambiguous
        .iter()
        .any(|item| item.field == "column"));
    assert!(resolved
        .resolution
        .unresolved
        .iter()
        .any(|item| item.field == "relation_kind"));
}

#[hegel::test]
fn typed_entity_assembly_does_not_leak_raw_metadata_law(tc: hegel::TestCase) {
    let suffix = draw_suffix(&tc);
    let raw_key = format!("raw_secret_{suffix}");
    let raw_value = format!("secret_value_{suffix}");
    let policy = OutputPolicyRegistry::default()
        .get("entity_verbose")
        .expect("entity_verbose policy")
        .clone();
    let assembler = CatalogEntityAssembler::new(policy);

    let entity = assembler
        .assemble(knowledge_entry(suffix, &raw_key, &raw_value))
        .entity
        .expect("valid typed metadata should assemble");
    let rendered = serde_json::to_string(&entity).unwrap();

    assert_eq!(entity.details["knowledge_type"], "constraint");
    assert!(!rendered.contains(&raw_key));
    assert!(!rendered.contains(&raw_value));
}

#[hegel::test]
fn invalid_metadata_warns_and_redacts_raw_json_law(tc: hegel::TestCase) {
    let suffix = draw_suffix(&tc);
    let raw_key = format!("raw_secret_{suffix}");
    let raw_value = format!("secret_value_{suffix}");
    let policy = OutputPolicyRegistry::default()
        .get("entity_verbose")
        .expect("entity_verbose policy")
        .clone();
    let assembler = CatalogEntityAssembler::new(policy);

    let entity = assembler
        .assemble(invalid_column_entry(suffix, &raw_key, &raw_value))
        .entity
        .expect("invalid metadata should still return an entity envelope");
    let rendered = serde_json::to_string(&entity).unwrap();

    assert!(!entity.warnings.is_empty());
    assert!(!rendered.contains(&raw_key));
    assert!(!rendered.contains(&raw_value));
}

#[test]
fn output_policy_caps_detail_fields_and_reports_truncation() {
    let mut policy = OutputPolicyRegistry::default()
        .get("entity_verbose")
        .expect("entity_verbose policy")
        .clone();
    policy.id = "tiny_full_policy".to_string();
    policy.max_fields_per_entity = 3;
    let assembler = CatalogEntityAssembler::new(policy);

    let entity = assembler
        .assemble(column_entry(42, "unit_sales", "a"))
        .entity
        .expect("valid column should assemble");

    assert!(
        entity.details.as_object().unwrap().len() <= 3,
        "details must honor max_fields_per_entity"
    );
    assert!(
        entity
            .warnings
            .iter()
            .any(|warning| warning.contains("truncated")),
        "detail truncation should be visible to callers, got {:?}",
        entity.warnings
    );
}

#[test]
fn output_policy_summary_and_full_details_are_distinct() {
    let summary_policy = OutputPolicyRegistry::default()
        .get("entity_summary")
        .expect("entity_summary policy")
        .clone();
    let full_policy = OutputPolicyRegistry::default()
        .get("entity_verbose")
        .expect("entity_verbose policy")
        .clone();

    let summary = CatalogEntityAssembler::new(summary_policy)
        .assemble(column_entry(43, "unit_sales", "a"))
        .entity
        .expect("summary entity");
    let full = CatalogEntityAssembler::new(full_policy)
        .assemble(column_entry(43, "unit_sales", "a"))
        .entity
        .expect("full entity");

    assert!(summary.details.get("distinct_count").is_none());
    assert!(full.details.get("distinct_count").is_some());
}

#[hegel::test]
fn duplicate_graph_links_are_deduped_law(tc: hegel::TestCase) {
    let suffix = draw_suffix(&tc);
    let target_id = format!("column:public.fact_{suffix}.dedupe");
    let duplicate_links = vec![
        CatalogRelation {
            target_id: target_id.clone(),
            kind: relation_kind::HAS_COLUMN.to_string(),
            description: Some("first duplicate".to_string()),
        },
        CatalogRelation {
            target_id: target_id.clone(),
            kind: relation_kind::HAS_COLUMN.to_string(),
            description: Some("second duplicate".to_string()),
        },
    ];
    let catalog = LawCatalog::new(vec![
        table_entry(suffix, duplicate_links),
        column_entry(suffix, "id", "dedupe"),
    ]);
    let policy = OutputPolicyRegistry::default()
        .get("entity_summary")
        .expect("entity_summary policy")
        .clone();
    let graph = CatalogGraphService::new(
        &catalog,
        CatalogRefResolver::new(&catalog),
        CatalogEntityAssembler::new(policy),
    );

    let output = tokio_test::block_on(graph.get_relations(GetCatalogRelationsInput {
        refs: vec![CatalogRef::id(format!("table:public.fact_{suffix}"))],
        direction: Some(RelationDirection::Outgoing),
        relation_kinds: Vec::new(),
        target_kinds: Vec::new(),
        limit_per_ref: Some(10),
    }))
    .unwrap();

    assert_eq!(output.results.len(), 1);
    assert_eq!(output.results[0].relations.len(), 1);
    assert_eq!(output.results[0].relations[0].target.id, target_id);
}

fn draw_suffix(tc: &hegel::TestCase) -> u16 {
    tc.draw(hegel::generators::integers::<u16>())
}

fn draw_kind(tc: &hegel::TestCase) -> CatalogEntityKind {
    tc.draw(hegel::generators::sampled_from(
        CatalogEntityKind::PUBLIC_SEARCHABLE.to_vec(),
    ))
}

fn table_entry(suffix: u16, links: Vec<CatalogRelation>) -> CatalogEntry {
    CatalogEntry {
        id: format!("table:public.fact_{suffix}"),
        kind: CatalogKind::Table,
        name: format!("fact_{suffix}"),
        qualified_name: Some(format!("public.fact_{suffix}")),
        content: "Fact table".to_string(),
        tags: Vec::new(),
        links,
        metadata: json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": format!("fact_{suffix}"),
            "relationType": "base_table",
            "rowCount": 10,
            "columnCount": 1,
            "preferredQuerySurface": true,
            "source": {}
        }),
    }
}

fn column_entry(suffix: u16, name: &str, variant: &str) -> CatalogEntry {
    CatalogEntry {
        id: format!("column:public.fact_{suffix}.{variant}"),
        kind: CatalogKind::Column,
        name: name.to_string(),
        qualified_name: Some(format!("public.fact_{suffix}.{name}_{variant}")),
        content: "Column".to_string(),
        tags: Vec::new(),
        links: vec![CatalogRelation {
            target_id: format!("table:public.fact_{suffix}"),
            kind: relation_kind::BELONGS_TO.to_string(),
            description: Some("column table".to_string()),
        }],
        metadata: json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": format!("fact_{suffix}"),
            "columnName": name,
            "dataType": "text",
            "nullable": false,
            "primaryKey": false,
            "foreignKey": null,
            "semanticType": "identifier",
            "distinctCount": 10,
            "nullCount": 0,
            "totalCount": 10,
            "lowCardinalityEnum": false
        }),
    }
}

fn knowledge_entry(suffix: u16, raw_key: &str, raw_value: &str) -> CatalogEntry {
    let mut metadata = json!({
        "knowledgeType": "constraint",
        "scopeTables": [format!("public.fact_{suffix}")],
        "scopeColumns": [format!("public.fact_{suffix}.units")],
        "sqlExpression": "units > 0",
        "synonyms": ["typed synonym"],
    });
    metadata[raw_key] = json!(raw_value);

    CatalogEntry {
        id: format!("knowledge:constraint_{suffix}"),
        kind: CatalogKind::Knowledge,
        name: format!("Constraint {suffix}"),
        qualified_name: None,
        content: "Business rule".to_string(),
        tags: Vec::new(),
        links: Vec::new(),
        metadata,
    }
}

fn invalid_column_entry(suffix: u16, raw_key: &str, raw_value: &str) -> CatalogEntry {
    let mut metadata = json!({
        "dataType": "text",
    });
    metadata[raw_key] = json!(raw_value);

    CatalogEntry {
        id: format!("column:bad_{suffix}"),
        kind: CatalogKind::Column,
        name: format!("bad_{suffix}"),
        qualified_name: Some(format!("public.fact_{suffix}.bad")),
        content: "Bad column".to_string(),
        tags: Vec::new(),
        links: Vec::new(),
        metadata,
    }
}

struct LawCatalog {
    entries: Vec<CatalogEntry>,
}

impl LawCatalog {
    fn new(entries: Vec<CatalogEntry>) -> Self {
        Self { entries }
    }
}

#[async_trait]
impl DataCatalog for LawCatalog {
    async fn get_by_id(&self, id: &str) -> Result<Option<CatalogEntry>, CatalogError> {
        Ok(self.entries.iter().find(|entry| entry.id == id).cloned())
    }

    async fn get_by_ids(&self, ids: &[String]) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(self
            .entries
            .iter()
            .filter(|entry| ids.contains(&entry.id))
            .cloned()
            .collect())
    }

    async fn get_by_qualified_name(
        &self,
        kind: CatalogKind,
        qualified_name: &str,
    ) -> Result<Option<CatalogEntry>, CatalogError> {
        Ok(self
            .entries
            .iter()
            .find(|entry| {
                entry.kind == kind && entry.qualified_name.as_deref() == Some(qualified_name)
            })
            .cloned())
    }

    async fn list_by_type(
        &self,
        kind: CatalogKind,
        limit: usize,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(self
            .entries
            .iter()
            .filter(|entry| entry.kind == kind)
            .take(limit)
            .cloned()
            .collect())
    }

    async fn get_related(
        &self,
        id: &str,
        relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        let Some(source) = self.entries.iter().find(|entry| entry.id == id) else {
            return Ok(Vec::new());
        };
        let target_ids: Vec<&str> = source
            .links
            .iter()
            .filter(|link| relation_type.is_none_or(|kind| link.kind == kind))
            .map(|link| link.target_id.as_str())
            .collect();
        Ok(self
            .entries
            .iter()
            .filter(|entry| target_ids.contains(&entry.id.as_str()))
            .cloned()
            .collect())
    }

    async fn get_related_reverse(
        &self,
        id: &str,
        relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(self
            .entries
            .iter()
            .filter(|entry| {
                entry.links.iter().any(|link| {
                    link.target_id == id && relation_type.is_none_or(|kind| link.kind == kind)
                })
            })
            .cloned()
            .collect())
    }

    async fn find_join_path(
        &self,
        _from_table: &str,
        _to_table: &str,
    ) -> Result<Option<JoinPath>, CatalogError> {
        Ok(None)
    }

    async fn list_tables(&self) -> Result<Vec<CatalogEntry>, CatalogError> {
        self.list_by_type(CatalogKind::Table, usize::MAX).await
    }

    async fn get_columns(&self, table_name: &str) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(self
            .entries
            .iter()
            .filter(|entry| {
                entry.kind == CatalogKind::Column
                    && entry
                        .qualified_name
                        .as_deref()
                        .is_some_and(|qualified_name| qualified_name.starts_with(table_name))
            })
            .cloned()
            .collect())
    }

    async fn get_enum_values(&self, _column_id: &str) -> Result<Vec<String>, CatalogError> {
        Ok(Vec::new())
    }
}
