use async_trait::async_trait;
use serde_json::{json, Value};

use agent_fw_catalog::{
    provenance_origin, relation_kind, CatalogEntry, CatalogError, CatalogKind, CatalogRelation,
    CatalogScope, CatalogSearchBackend, CatalogSearchHealth, CatalogSearchRequest,
    CatalogSearchResults, DataCatalog, JoinPath,
};
use agent_fw_catalog_tools::surface::{
    CatalogEntityAssembler, CatalogEntityKind, CatalogFilterRef, CatalogFilterResolver,
    CatalogFilters, CatalogGraphService, CatalogRef, CatalogRefResolver, GetCatalogRelationsInput,
    OutputPolicyRegistry, RelationDirection, SearchCatalogInput,
};
use agent_fw_catalog_tools::tool_metadata;

fn schema_for<T: agent_fw_tool::ToolSchema>() -> Value {
    T::json_schema()
}

#[test]
fn manual_schemas_express_catalog_ref_and_filter_ref_contracts() {
    let ref_schema: Value = schema_for::<CatalogRef>();
    assert_eq!(ref_schema["type"], json!("object"));
    assert_eq!(ref_schema["additionalProperties"], json!(false));
    assert_eq!(ref_schema["oneOf"].as_array().unwrap().len(), 3);

    let filter_ref_schema: Value = schema_for::<CatalogFilterRef>();
    let variants: &Vec<Value> = filter_ref_schema["oneOf"].as_array().unwrap();
    assert_eq!(variants.len(), 2);
    assert!(variants
        .iter()
        .any(|variant| variant["type"] == json!("string")));

    let search_schema: Value = schema_for::<SearchCatalogInput>();
    assert_eq!(search_schema["additionalProperties"], json!(false));
    assert!(search_schema["required"]
        .as_array()
        .unwrap()
        .contains(&json!("query")));
    assert_eq!(search_schema["properties"]["limit"]["maximum"], json!(50));
    assert_eq!(
        search_schema["properties"]["filters"]["$ref"],
        json!("#/definitions/CatalogFilters")
    );
}

#[test]
fn entity_assembler_uses_typed_metadata_and_public_kind_names() {
    let policy = OutputPolicyRegistry::default()
        .get("entity_verbose")
        .expect("entity_verbose policy")
        .clone();
    let assembler = CatalogEntityAssembler::new(policy);

    let enum_output = assembler.assemble(enum_entry()).entity.unwrap();
    assert_eq!(enum_output.kind, CatalogEntityKind::EnumValue);
    assert_eq!(enum_output.details["value"], "awaiting_payment");
    assert_eq!(enum_output.details["synonyms"], json!(["open invoice"]));
    assert!(
        !serde_json::to_string(&enum_output)
            .unwrap()
            .contains("\"enum\""),
        "public output should not expose the internal enum kind"
    );

    let knowledge_output = assembler.assemble(knowledge_entry()).entity.unwrap();
    assert_eq!(knowledge_output.details["knowledge_type"], "constraint");
    assert_eq!(knowledge_output.details["synonyms"], json!(["slow mover"]));
    assert!(
        !serde_json::to_string(&knowledge_output)
            .unwrap()
            .contains("raw_only_secret"),
        "assembler leaked raw metadata instead of typed details"
    );
}

#[test]
fn entity_assembler_exposes_relationship_provenance() {
    let policy = OutputPolicyRegistry::default()
        .get("entity_verbose")
        .expect("entity_verbose policy")
        .clone();
    let assembler = CatalogEntityAssembler::new(policy);
    let mut entry = relationship_entry();
    entry.metadata["source"] = json!({
        "origin": provenance_origin::PHYSICAL_SCHEMA,
        "profilingRunId": "profile-1"
    });

    let relationship_output = assembler.assemble(entry).entity.unwrap();

    assert_eq!(
        relationship_output.details["source"]["origin"],
        provenance_origin::PHYSICAL_SCHEMA
    );
    assert_eq!(
        relationship_output.details["source"]["profilingRunId"],
        "profile-1"
    );
    assert_eq!(
        relationship_output.details["relationship_kind"],
        "foreign_key"
    );
}

#[test]
fn invalid_metadata_warns_without_raw_json_leakage() {
    let policy = OutputPolicyRegistry::default()
        .get("entity_verbose")
        .expect("entity_verbose policy")
        .clone();
    let assembler = CatalogEntityAssembler::new(policy);

    let output = assembler.assemble(invalid_column_entry());
    let entity = output
        .entity
        .expect("invalid metadata still returns envelope");

    assert!(entity
        .warnings
        .iter()
        .any(|warning| warning.contains("invalid column metadata")));
    assert!(
        !serde_json::to_string(&entity)
            .unwrap()
            .contains("raw_only_secret"),
        "invalid metadata warning path leaked raw JSON"
    );
}

#[test]
fn output_policy_registry_contains_required_task3_profiles() {
    let registry = OutputPolicyRegistry::default();
    for id in [
        "tool_use_compact",
        "entity_summary",
        "entity_verbose",
        "schema_compact",
        "schema_verbose",
    ] {
        assert!(registry.get(id).is_some(), "missing output policy {id}");
    }
}

#[test]
fn catalog_tool_descriptions_use_structured_agent_guidance_template() {
    for metadata in tool_metadata::SURFACE_TOOLS {
        let description = metadata.description;
        assert!(
            description.starts_with("/**\n * "),
            "{} description should use the structured doc-comment format",
            metadata.name
        );
        for section in [
            "Use this when:",
            "Do not use this when:",
            "Inputs:",
            "Behavior:",
            "Returns:",
            "After calling:",
        ] {
            assert!(
                description.contains(section),
                "{} description is missing `{section}`",
                metadata.name
            );
        }
        assert!(
            description.ends_with("\n */"),
            "{} description should close the doc-comment format",
            metadata.name
        );
        let normalized_description = description.to_ascii_lowercase();
        for forbidden in [
            "product",
            "slow moving",
            "supermarket",
            "fact_scenario",
            "fact_sales",
            "dim_products",
            "invoice",
            "sales",
        ] {
            assert!(
                !normalized_description.contains(forbidden),
                "{} description should not use domain-specific example `{forbidden}`",
                metadata.name
            );
        }
    }

    let search = tool_metadata::SEARCH_CATALOG.description;
    assert!(search.contains("Discover candidate catalog entities"));
    assert!(search.contains("- query: Natural-language search phrase or identifier"));
    assert!(search.contains("- kinds: Optional array of entity kinds"));
    assert!(search.contains("- filters: Optional structured filters"));
    assert!(search.contains("- limit: Optional requested result count, 1..=50"));
    assert!(search.contains("- cursor: Optional opaque cursor"));
    assert!(
        search.contains("- Does not return complete schema, joins, or executable query context")
    );
    assert!(search.contains("- Select only the relevant refs, then call get_catalog_entities"));
    assert!(search.contains("Do not treat broad search results as executable SQL context"));
    assert!(
        !search.to_ascii_lowercase().contains("trivial"),
        "search_catalog guidance must not include shortcuts from broad search to SQL"
    );

    let hydrate = tool_metadata::GET_CATALOG_ENTITIES.description;
    assert!(hydrate.contains("Use after search_catalog"));
    assert!(hydrate.contains("selected ids or qualified names"));
    assert!(hydrate.contains("use list_schema_fields for table columns"));

    let fields = tool_metadata::LIST_SCHEMA_FIELDS.description;
    assert!(fields.contains("Use after selecting table or query-surface refs"));
    assert!(fields.contains("before execute_query"));

    let execute = tool_metadata::EXECUTE_QUERY.description;
    assert!(execute.contains("Run a validated read-only SQL query as the final step"));
    assert!(execute.contains("- You only have broad search_catalog candidates"));
    assert!(execute.contains("- sql: Required single read-only SELECT or WITH query"));
    assert!(execute.contains("- params: Optional positional bind parameters"));
    assert!(execute.contains("- purpose: Optional short audit/trace explanation"));
    assert!(execute.contains("confirming tables, fields, joins, filters, and semantic rules"));
}

#[tokio::test]
async fn ref_resolver_exact_paths_do_not_use_catalog_search() {
    let catalog = NoSearchCatalog::new(vec![table_entry(), column_entry("id", "column:one")]);
    let resolver = CatalogRefResolver::new(&catalog);

    let by_qualified = resolver
        .resolve(&CatalogRef {
            id: None,
            qualified_name: Some("public.fact_sales".to_string()),
            name: None,
            kind: Some(CatalogEntityKind::Table),
        })
        .await
        .unwrap();
    assert_eq!(by_qualified.resolved.unwrap().id, "table:public.fact_sales");

    let by_name = resolver
        .resolve(&CatalogRef {
            id: None,
            qualified_name: None,
            name: Some("id".to_string()),
            kind: Some(CatalogEntityKind::Column),
        })
        .await
        .unwrap();
    assert_eq!(by_name.resolved.unwrap().id, "column:one");

    let missing_kind = resolver
        .resolve(&CatalogRef {
            id: None,
            qualified_name: None,
            name: Some("id".to_string()),
            kind: None,
        })
        .await
        .unwrap();
    assert_eq!(
        missing_kind.unresolved.as_deref(),
        Some("kind_required_for_name_reference")
    );
}

#[tokio::test]
async fn ref_resolver_exact_name_path_uses_indexed_name_lookup() {
    let catalog = ExactNameOnlyCatalog {
        entry: column_entry("id", "column:one"),
    };
    let resolver = CatalogRefResolver::new(&catalog);

    let by_name = resolver
        .resolve_exact(&CatalogRef {
            id: None,
            qualified_name: None,
            name: Some("id".to_string()),
            kind: Some(CatalogEntityKind::Column),
        })
        .await
        .unwrap();

    assert_eq!(by_name.resolved.unwrap().id, "column:one");
}

#[tokio::test]
async fn canonical_refs_emitted_by_resolution_are_reusable() {
    let catalog = NoSearchCatalog::new(vec![table_entry()]);
    let resolver = CatalogRefResolver::new(&catalog);

    let resolved = resolver
        .resolve_exact(&CatalogRef {
            id: None,
            qualified_name: Some("public.fact_sales".to_string()),
            name: None,
            kind: Some(CatalogEntityKind::Table),
        })
        .await
        .unwrap()
        .resolved
        .expect("qualified name resolves");

    let canonical = resolved.catalog_ref();
    assert_eq!(
        canonical.provided_reference_count(),
        1,
        "canonical CatalogRef must satisfy the one-of schema"
    );

    let resolved_again = resolver.resolve_exact(&canonical).await.unwrap();
    assert_eq!(
        resolved_again.resolved.unwrap().id,
        "table:public.fact_sales"
    );
}

#[tokio::test]
async fn qualified_name_resolution_uses_exact_lookup_path() {
    let catalog = QualifiedLookupOnlyCatalog {
        entry: table_entry(),
    };
    let resolver = CatalogRefResolver::new(&catalog);

    let resolved = resolver
        .resolve_exact(&CatalogRef {
            id: None,
            qualified_name: Some("public.fact_sales".to_string()),
            name: None,
            kind: Some(CatalogEntityKind::Table),
        })
        .await
        .unwrap();

    assert_eq!(resolved.resolved.unwrap().id, "table:public.fact_sales");
}

#[tokio::test]
async fn graph_service_normalizes_direct_reverse_and_relationship_vertex_edges() {
    let catalog = NoSearchCatalog::new(vec![
        table_entry(),
        table_entry_named("dim_products"),
        column_entry("id", "column:one"),
        relationship_entry(),
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

    let output = graph
        .get_relations(GetCatalogRelationsInput {
            refs: vec![CatalogRef::id("table:public.fact_sales")],
            direction: Some(RelationDirection::Both),
            relation_kinds: Vec::new(),
            target_kinds: Vec::new(),
            limit_per_ref: Some(10),
        })
        .await
        .unwrap();

    let relation_kinds: Vec<String> = output.results[0]
        .relations
        .iter()
        .map(|edge| edge.relation_kind.clone())
        .collect();
    assert!(relation_kinds.contains(&relation_kind::HAS_COLUMN.to_string()));
    assert!(relation_kinds.contains(&relation_kind::BELONGS_TO.to_string()));
    assert!(relation_kinds.contains(&relation_kind::REFERENCES_TABLE.to_string()));
    assert!(output.results[0]
        .relations
        .iter()
        .any(|edge| edge.relationship.is_some()));

    let invalid = graph
        .get_relations(GetCatalogRelationsInput {
            refs: vec![CatalogRef::id("table:public.fact_sales")],
            direction: Some(RelationDirection::Both),
            relation_kinds: vec!["not_a_relation_kind".to_string()],
            target_kinds: Vec::new(),
            limit_per_ref: Some(10),
        })
        .await
        .unwrap();
    assert!(invalid.results[0].relations.is_empty());
    assert!(invalid
        .warnings
        .iter()
        .any(|warning| warning.contains("unknown relation_kind filter")));
}

#[tokio::test]
async fn graph_service_collapses_materialized_fk_link_and_relationship_vertex() {
    // Reproduces the production shape: the interpreters materialize a
    // `references_table` link on the source table (emitted as
    // `relationship: None`) AND a relationship vertex that drives
    // `relationship_vertex_edges` to emit the SAME logical edge with
    // `relationship: Some(..)`. The two must collapse into a single edge that
    // carries the relationship vertex and its join-column description.
    let catalog = NoSearchCatalog::new(vec![
        fact_sales_with_fk_link(),
        table_entry_named("dim_products"),
        column_entry("id", "column:one"),
        relationship_entry(),
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

    let output = graph
        .get_relations(GetCatalogRelationsInput {
            refs: vec![CatalogRef::id("table:public.fact_sales")],
            direction: Some(RelationDirection::Outgoing),
            relation_kinds: Vec::new(),
            target_kinds: Vec::new(),
            limit_per_ref: Some(10),
        })
        .await
        .unwrap();

    let fk_edges: Vec<_> = output.results[0]
        .relations
        .iter()
        .filter(|edge| {
            edge.relation_kind == relation_kind::REFERENCES_TABLE
                && edge.target.id == "table:public.dim_products"
        })
        .collect();
    assert_eq!(
        fk_edges.len(),
        1,
        "materialized FK link and relationship vertex must collapse into one edge"
    );

    let fk_edge = fk_edges[0];
    let relationship = fk_edge
        .relationship
        .as_ref()
        .expect("surviving FK edge must carry the relationship vertex metadata");
    assert_eq!(relationship.id, "relationship:fact_sales_dim_products");
    assert!(
        fk_edge
            .description
            .as_deref()
            .is_some_and(|description| description.contains("product_id")),
        "the relationship vertex join description must survive the merge, got {:?}",
        fk_edge.description
    );
}

#[tokio::test]
async fn graph_service_warns_when_relationship_vertex_scan_hits_cap() {
    let mut entries = vec![table_entry()];
    entries.extend((0..10_000).map(invalid_relationship_entry));
    let catalog = NoSearchCatalog::new(entries);
    let policy = OutputPolicyRegistry::default()
        .get("entity_summary")
        .expect("entity_summary policy")
        .clone();
    let graph = CatalogGraphService::new(
        &catalog,
        CatalogRefResolver::new(&catalog),
        CatalogEntityAssembler::new(policy),
    );

    let output = graph
        .get_relations(GetCatalogRelationsInput {
            refs: vec![CatalogRef::id("table:public.fact_sales")],
            direction: Some(RelationDirection::Both),
            relation_kinds: Vec::new(),
            target_kinds: Vec::new(),
            limit_per_ref: Some(10),
        })
        .await
        .unwrap();

    assert!(output
        .warnings
        .iter()
        .any(|warning| warning.contains("relationship vertex scan reached limit")));
}

#[tokio::test]
async fn filter_resolver_reports_ambiguous_column_without_applying_it() {
    let catalog = NoSearchCatalog::new(vec![
        table_entry(),
        column_entry("id", "column:one"),
        column_entry("id", "column:two"),
    ]);
    let resolver = CatalogFilterResolver::new(CatalogRefResolver::new(&catalog));

    let resolved = resolver
        .resolve(&CatalogFilters {
            column: Some(CatalogFilterRef::String("id".to_string())),
            relation_kind: Some("not_a_relation_kind".to_string()),
            ..CatalogFilters::default()
        })
        .await
        .unwrap();

    assert!(resolved.backend_filters.column.is_none());
    assert_eq!(resolved.resolution.ambiguous.len(), 1);
    assert_eq!(resolved.resolution.ambiguous[0].field, "column");
    assert!(resolved
        .resolution
        .unresolved
        .iter()
        .any(|item| item.field == "relation_kind"));
}

#[tokio::test]
async fn filter_resolver_does_not_use_fuzzy_fallback_for_canonicalization() {
    let catalog = NoSearchCatalog::new(vec![table_entry()]);
    let resolver = CatalogFilterResolver::new(
        CatalogRefResolver::new(&catalog)
            .with_search_backend(CatalogScope::legacy_unscoped(), &PanicSearchBackend),
    );

    let resolved = resolver
        .resolve(&CatalogFilters {
            column: Some(CatalogFilterRef::String("missing_column".to_string())),
            ..CatalogFilters::default()
        })
        .await
        .unwrap();

    assert!(resolved.backend_filters.column.is_none());
    assert!(resolved
        .resolution
        .unresolved
        .iter()
        .any(|item| item.field == "column"));
}

fn table_entry() -> CatalogEntry {
    table_entry_named("fact_sales")
}

/// Mirrors what the SQLite/Postgres interpreters materialize for a table that
/// participates in a foreign-key relationship: alongside the `has_column` link,
/// a `references_table` link to the target table is stored with `relationship`
/// metadata captured in the description. `outgoing_edges` emits this with
/// `relationship: None`, which must collapse with the relationship-vertex edge.
fn fact_sales_with_fk_link() -> CatalogEntry {
    let mut entry = table_entry();
    entry.links.push(CatalogRelation {
        target_id: "table:public.dim_products".to_string(),
        kind: relation_kind::REFERENCES_TABLE.to_string(),
        description: Some(
            "[materialized_relationship] fact_sales.product_id -> dim_products.product_id"
                .to_string(),
        ),
    });
    entry
}

fn table_entry_named(table: &str) -> CatalogEntry {
    CatalogEntry {
        id: format!("table:public.{table}"),
        kind: CatalogKind::Table,
        name: table.to_string(),
        qualified_name: Some(format!("public.{table}")),
        content: format!("{table} table"),
        tags: vec!["sales".to_string()],
        links: vec![CatalogRelation {
            target_id: "column:one".to_string(),
            kind: relation_kind::HAS_COLUMN.to_string(),
            description: Some("table column".to_string()),
        }],
        metadata: json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": table,
            "relationType": "base_table",
            "rowCount": 10,
            "columnCount": 1,
            "preferredQuerySurface": true,
            "source": {}
        }),
    }
}

fn relationship_entry() -> CatalogEntry {
    CatalogEntry {
        id: "relationship:fact_sales_dim_products".to_string(),
        kind: CatalogKind::Relationship,
        name: "fact_sales_to_dim_products".to_string(),
        qualified_name: None,
        content: "fact_sales.product_id references dim_products.product_id".to_string(),
        tags: vec!["relationship".to_string()],
        links: vec![
            CatalogRelation {
                target_id: "table:public.fact_sales".to_string(),
                kind: relation_kind::RELATIONSHIP_SOURCE_TABLE.to_string(),
                description: Some("source table".to_string()),
            },
            CatalogRelation {
                target_id: "table:public.dim_products".to_string(),
                kind: relation_kind::RELATIONSHIP_TARGET_TABLE.to_string(),
                description: Some("target table".to_string()),
            },
        ],
        metadata: json!({
            "databaseId": "warehouse",
            "sourceTableId": "table:public.fact_sales",
            "targetTableId": "table:public.dim_products",
            "sourceSchema": "public",
            "sourceTable": "fact_sales",
            "sourceColumn": "product_id",
            "targetSchema": "public",
            "targetTable": "dim_products",
            "targetColumn": "product_id",
            "sourceCardinality": "many",
            "targetCardinality": "one",
            "relationshipKind": "foreign_key",
            "confidence": 1.0
        }),
    }
}

fn invalid_relationship_entry(index: usize) -> CatalogEntry {
    CatalogEntry {
        id: format!("relationship:invalid:{index}"),
        kind: CatalogKind::Relationship,
        name: format!("invalid_relationship_{index}"),
        qualified_name: None,
        content: "Invalid relationship metadata".to_string(),
        tags: Vec::new(),
        links: Vec::new(),
        metadata: json!({
            "raw_only_secret": true
        }),
    }
}

fn column_entry(name: &str, id: &str) -> CatalogEntry {
    CatalogEntry {
        id: id.to_string(),
        kind: CatalogKind::Column,
        name: name.to_string(),
        qualified_name: Some(format!("public.fact_sales.{name}")),
        content: "Column".to_string(),
        tags: Vec::new(),
        links: vec![CatalogRelation {
            target_id: "table:public.fact_sales".to_string(),
            kind: relation_kind::BELONGS_TO.to_string(),
            description: Some("column table".to_string()),
        }],
        metadata: json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": "fact_sales",
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

fn enum_entry() -> CatalogEntry {
    CatalogEntry {
        id: "enum:public.fact_sales.status.awaiting_payment".to_string(),
        kind: CatalogKind::Enum,
        name: "awaiting_payment".to_string(),
        qualified_name: Some("public.fact_sales.status.awaiting_payment".to_string()),
        content: "Awaiting payment order status".to_string(),
        tags: Vec::new(),
        links: Vec::new(),
        metadata: json!({
            "databaseId": "warehouse",
            "schemaName": "public",
            "tableName": "fact_sales",
            "columnName": "status",
            "columnId": "column:status",
            "value": "awaiting_payment",
            "normalizedValue": "awaiting_payment",
            "displayValue": "Awaiting payment",
            "frequency": 3,
            "frequencyPercentage": 30.0,
            "rank": 1,
            "synonyms": ["open invoice"]
        }),
    }
}

fn knowledge_entry() -> CatalogEntry {
    CatalogEntry {
        id: "knowledge:slow_mover".to_string(),
        kind: CatalogKind::Knowledge,
        name: "Slow mover".to_string(),
        qualified_name: None,
        content: "Slow moving product rule".to_string(),
        tags: Vec::new(),
        links: Vec::new(),
        metadata: json!({
            "knowledgeType": "constraint",
            "scopeTables": ["public.fact_sales"],
            "scopeColumns": ["public.fact_sales.units"],
            "sqlExpression": "velocity < 0.25",
            "synonyms": ["slow mover"],
            "raw_only_secret": true
        }),
    }
}

fn invalid_column_entry() -> CatalogEntry {
    CatalogEntry {
        id: "column:bad".to_string(),
        kind: CatalogKind::Column,
        name: "bad".to_string(),
        qualified_name: Some("public.fact_sales.bad".to_string()),
        content: "Bad column".to_string(),
        tags: Vec::new(),
        links: Vec::new(),
        metadata: json!({
            "raw_only_secret": true,
            "dataType": "text"
        }),
    }
}

struct NoSearchCatalog {
    entries: Vec<CatalogEntry>,
}

struct QualifiedLookupOnlyCatalog {
    entry: CatalogEntry,
}

struct ExactNameOnlyCatalog {
    entry: CatalogEntry,
}

struct PanicSearchBackend;

#[async_trait]
impl CatalogSearchBackend for PanicSearchBackend {
    async fn search(
        &self,
        _scope: &CatalogScope,
        _request: CatalogSearchRequest,
    ) -> Result<CatalogSearchResults, CatalogError> {
        panic!("filter canonicalization must not use CatalogSearchBackend fallback")
    }

    async fn health(&self, _scope: &CatalogScope) -> Result<CatalogSearchHealth, CatalogError> {
        Ok(CatalogSearchHealth::Ready {
            indexed_entries: 0,
            projection_version: 1,
        })
    }
}

impl NoSearchCatalog {
    fn new(entries: Vec<CatalogEntry>) -> Self {
        Self { entries }
    }
}

#[async_trait]
impl DataCatalog for NoSearchCatalog {
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

    async fn get_by_name(
        &self,
        kind: CatalogKind,
        name: &str,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(self
            .entries
            .iter()
            .filter(|entry| entry.kind == kind && entry.name == name)
            .cloned()
            .collect())
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

#[async_trait]
impl DataCatalog for QualifiedLookupOnlyCatalog {
    async fn get_by_id(&self, id: &str) -> Result<Option<CatalogEntry>, CatalogError> {
        Ok((self.entry.id == id).then(|| self.entry.clone()))
    }

    async fn get_by_ids(&self, ids: &[String]) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(ids
            .iter()
            .filter(|id| self.entry.id == **id)
            .map(|_| self.entry.clone())
            .collect())
    }

    async fn get_by_qualified_name(
        &self,
        kind: CatalogKind,
        qualified_name: &str,
    ) -> Result<Option<CatalogEntry>, CatalogError> {
        Ok((self.entry.kind == kind
            && self.entry.qualified_name.as_deref() == Some(qualified_name))
        .then(|| self.entry.clone()))
    }

    async fn get_by_name(
        &self,
        _kind: CatalogKind,
        _name: &str,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        panic!("qualified-name resolution must not scan exact names")
    }

    async fn list_by_type(
        &self,
        _kind: CatalogKind,
        _limit: usize,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        panic!("qualified-name resolution must not scan list_by_type")
    }

    async fn get_related(
        &self,
        _id: &str,
        _relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(Vec::new())
    }

    async fn get_related_reverse(
        &self,
        _id: &str,
        _relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(Vec::new())
    }

    async fn find_join_path(
        &self,
        _from_table: &str,
        _to_table: &str,
    ) -> Result<Option<JoinPath>, CatalogError> {
        Ok(None)
    }

    async fn list_tables(&self) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(Vec::new())
    }

    async fn get_columns(&self, _table_name: &str) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(Vec::new())
    }

    async fn get_enum_values(&self, _column_id: &str) -> Result<Vec<String>, CatalogError> {
        Ok(Vec::new())
    }
}

#[async_trait]
impl DataCatalog for ExactNameOnlyCatalog {
    async fn get_by_id(&self, id: &str) -> Result<Option<CatalogEntry>, CatalogError> {
        Ok((self.entry.id == id).then(|| self.entry.clone()))
    }

    async fn get_by_ids(&self, ids: &[String]) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(ids
            .iter()
            .filter(|id| self.entry.id == **id)
            .map(|_| self.entry.clone())
            .collect())
    }

    async fn get_by_qualified_name(
        &self,
        _kind: CatalogKind,
        _qualified_name: &str,
    ) -> Result<Option<CatalogEntry>, CatalogError> {
        Ok(None)
    }

    async fn get_by_name(
        &self,
        kind: CatalogKind,
        name: &str,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok((self.entry.kind == kind && self.entry.name == name)
            .then(|| vec![self.entry.clone()])
            .unwrap_or_default())
    }

    async fn list_by_type(
        &self,
        _kind: CatalogKind,
        _limit: usize,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        panic!("exact-name resolution must not scan list_by_type")
    }

    async fn get_related(
        &self,
        _id: &str,
        _relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(Vec::new())
    }

    async fn get_related_reverse(
        &self,
        _id: &str,
        _relation_type: Option<&str>,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(Vec::new())
    }

    async fn find_join_path(
        &self,
        _from_table: &str,
        _to_table: &str,
    ) -> Result<Option<JoinPath>, CatalogError> {
        Ok(None)
    }

    async fn list_tables(&self) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(Vec::new())
    }

    async fn get_columns(&self, _table_name: &str) -> Result<Vec<CatalogEntry>, CatalogError> {
        Ok(Vec::new())
    }

    async fn get_enum_values(&self, _column_id: &str) -> Result<Vec<String>, CatalogError> {
        Ok(Vec::new())
    }
}
