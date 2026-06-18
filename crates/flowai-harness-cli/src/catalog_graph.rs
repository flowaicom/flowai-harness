use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use agent_fw_catalog::{
    CatalogEntry, CatalogKind, CatalogRelation, CatalogScope, DataCatalog, RelationshipMetadata,
    SemanticEntity,
};
use serde::Serialize;
use serde_json::json;

use crate::CliError;

type GraphEdgeKey = (String, String, String, Option<String>);

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GraphBuildOptions {
    pub include_columns: bool,
    pub max_nodes: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogGraph {
    pub summary: GraphSummary,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GraphSummary {
    pub tenant_id: String,
    pub workspace_id: String,
    pub node_count: usize,
    pub edge_count: usize,
    pub omitted_node_count: usize,
    pub truncated: bool,
    pub counts_by_kind: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub qualified_name: Option<String>,
    pub content: String,
    pub tags: Vec<String>,
    pub details: serde_json::Value,
    pub degree: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub relation_type: String,
    pub description: Option<String>,
    pub relationship_id: Option<String>,
    pub join: Option<GraphJoinMetadata>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GraphJoinMetadata {
    pub source_table_id: String,
    pub target_table_id: String,
    pub source_column: String,
    pub target_column: String,
    pub source_cardinality: String,
    pub target_cardinality: String,
    pub relationship_kind: String,
    pub confidence: Option<String>,
}

pub fn render_json(graph: &CatalogGraph) -> Result<String, CliError> {
    serde_json::to_string_pretty(graph).map_err(CliError::Json)
}

pub fn render_html(graph: &CatalogGraph) -> Result<String, CliError> {
    let graph_json = serde_json::to_string(graph)?;
    let graph_json = escape_script_json(&graph_json);

    Ok(format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Flow AI Catalog Graph</title>
  <style>{css}</style>
</head>
<body>
  <script id="catalog-graph-data" type="application/json">{graph_json}</script>
  <div id="app">
    <aside id="explorer"></aside>
    <main>
      <div id="toolbar"></div>
      <div id="graph"></div>
    </main>
    <aside id="inspector"></aside>
  </div>
  <div id="load-error" hidden></div>
  <script src="https://unpkg.com/three@0.160.1/build/three.min.js"></script>
  <script src="https://unpkg.com/3d-force-graph@1.77.0/dist/3d-force-graph.min.js"></script>
  <script src="https://unpkg.com/three-spritetext@1.8.2/dist/three-spritetext.min.js"></script>
  <script>{js}</script>
</body>
</html>
"#,
        css = VIEWER_CSS,
        graph_json = graph_json,
        js = VIEWER_JS
    ))
}

fn escape_script_json(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    let mut index = 0;

    while index < input.len() {
        let rest = &input[index..];
        if starts_with_script_end_tag(rest) {
            escaped.push_str("<\\/");
            index += 2;
        } else if rest.starts_with("<!--") {
            escaped.push_str("\\u003C!--");
            index += 4;
        } else {
            let ch = rest
                .chars()
                .next()
                .expect("index is in bounds while escaping script JSON");
            escaped.push(ch);
            index += ch.len_utf8();
        }
    }

    escaped
}

fn starts_with_script_end_tag(input: &str) -> bool {
    let bytes = input.as_bytes();
    bytes.len() >= 8
        && bytes[0] == b'<'
        && bytes[1] == b'/'
        && bytes[2..8].eq_ignore_ascii_case(b"script")
}

pub async fn build_catalog_graph(
    catalog: &dyn DataCatalog,
    scope: &CatalogScope,
    options: GraphBuildOptions,
) -> Result<CatalogGraph, CliError> {
    if options.max_nodes == 0 {
        return Err(CliError::Parse(
            "--max-nodes must be greater than 0".to_string(),
        ));
    }

    let map_catalog_error = |error| CliError::Execution(format!("catalog graph failed: {error}"));

    let mut entries = catalog.list_tables().await.map_err(map_catalog_error)?;
    let table_entries = entries.clone();

    for kind in [
        CatalogKind::Relationship,
        CatalogKind::Metric,
        CatalogKind::Document,
        CatalogKind::Knowledge,
        CatalogKind::DataQualityFinding,
    ] {
        entries.extend(
            catalog
                .list_by_type(kind, options.max_nodes)
                .await
                .map_err(map_catalog_error)?,
        );
    }

    if options.include_columns {
        for table in table_entries {
            let table_ref = table.qualified_name.as_deref().unwrap_or(&table.id);
            entries.extend(
                catalog
                    .get_columns(table_ref)
                    .await
                    .map_err(map_catalog_error)?,
            );
        }
        entries.extend(
            catalog
                .list_by_type(CatalogKind::Enum, options.max_nodes)
                .await
                .map_err(map_catalog_error)?,
        );
    }
    expand_context_relation_targets(catalog, &mut entries, options.max_nodes).await?;

    Ok(graph_from_entries(scope, entries, options.max_nodes))
}

async fn expand_context_relation_targets(
    catalog: &dyn DataCatalog,
    entries: &mut Vec<CatalogEntry>,
    max_targets: usize,
) -> Result<(), CliError> {
    let loaded_ids: HashSet<String> = entries.iter().map(|entry| entry.id.clone()).collect();
    let mut target_ids = BTreeSet::new();

    for entry in entries.iter() {
        if !should_expand_relation_targets(entry.kind) {
            continue;
        }
        for link in &entry.links {
            if loaded_ids.contains(&link.target_id) {
                continue;
            }
            target_ids.insert(link.target_id.clone());
            if target_ids.len() >= max_targets {
                break;
            }
        }
        if target_ids.len() >= max_targets {
            break;
        }
    }

    if target_ids.is_empty() {
        return Ok(());
    }

    let ids: Vec<String> = target_ids.into_iter().collect();
    let targets = catalog
        .get_by_ids(&ids)
        .await
        .map_err(|error| CliError::Execution(format!("catalog graph failed: {error}")))?;
    entries.extend(targets);

    Ok(())
}

fn should_expand_relation_targets(kind: CatalogKind) -> bool {
    matches!(
        kind,
        CatalogKind::Metric
            | CatalogKind::Document
            | CatalogKind::Knowledge
            | CatalogKind::DataQualityFinding
    )
}

fn graph_from_entries(
    scope: &CatalogScope,
    entries: Vec<CatalogEntry>,
    max_nodes: usize,
) -> CatalogGraph {
    let mut deduped_by_id = HashMap::new();
    for entry in entries {
        deduped_by_id.entry(entry.id.clone()).or_insert(entry);
    }

    let mut entries: Vec<CatalogEntry> = deduped_by_id
        .into_values()
        .filter(|entry| entry.kind.is_public_searchable())
        .collect();
    entries.sort_by(|left, right| {
        (
            graph_kind_rank(left.kind),
            left.qualified_name.as_deref().unwrap_or(""),
            left.name.as_str(),
            left.id.as_str(),
        )
            .cmp(&(
                graph_kind_rank(right.kind),
                right.qualified_name.as_deref().unwrap_or(""),
                right.name.as_str(),
                right.id.as_str(),
            ))
    });

    let omitted_node_count = entries.len().saturating_sub(max_nodes);
    let truncated = omitted_node_count > 0;
    entries.truncate(max_nodes);

    let visible_ids: HashSet<String> = entries.iter().map(|entry| entry.id.clone()).collect();
    let mut edges = Vec::new();
    let mut edge_keys = BTreeSet::new();

    for entry in &entries {
        for link in &entry.links {
            push_edge(&mut edges, &mut edge_keys, entry, link, &visible_ids);
        }

        if entry.kind == CatalogKind::Relationship {
            push_relationship_edge(&mut edges, &mut edge_keys, entry, &visible_ids);
        }
    }

    edges.sort_by(|left, right| {
        (
            left.source.as_str(),
            left.target.as_str(),
            left.relation_type.as_str(),
            left.id.as_str(),
        )
            .cmp(&(
                right.source.as_str(),
                right.target.as_str(),
                right.relation_type.as_str(),
                right.id.as_str(),
            ))
    });

    let mut degree_by_id: HashMap<String, usize> = HashMap::new();
    for edge in &edges {
        *degree_by_id.entry(edge.source.clone()).or_default() += 1;
        *degree_by_id.entry(edge.target.clone()).or_default() += 1;
    }

    let nodes: Vec<GraphNode> = entries
        .into_iter()
        .map(|entry| {
            let degree = degree_by_id.get(&entry.id).copied().unwrap_or_default();
            let kind = entry.kind.public_name().to_string();
            let details = graph_details(&entry);
            GraphNode {
                degree,
                id: entry.id,
                label: entry.name,
                kind,
                qualified_name: entry.qualified_name,
                content: entry.content,
                tags: entry.tags,
                details,
            }
        })
        .collect();

    let mut counts_by_kind = BTreeMap::new();
    for node in &nodes {
        *counts_by_kind.entry(node.kind.clone()).or_default() += 1;
    }

    CatalogGraph {
        summary: GraphSummary {
            tenant_id: scope.tenant_id.to_string(),
            workspace_id: scope.workspace_id.to_string(),
            node_count: nodes.len(),
            edge_count: edges.len(),
            omitted_node_count,
            truncated,
            counts_by_kind,
        },
        nodes,
        edges,
    }
}

fn graph_kind_rank(kind: CatalogKind) -> u8 {
    match kind {
        CatalogKind::Table => 0,
        CatalogKind::Relationship => 1,
        CatalogKind::Column => 2,
        CatalogKind::Enum => 3,
        CatalogKind::Metric => 4,
        CatalogKind::Document => 5,
        CatalogKind::Knowledge => 6,
        CatalogKind::DataQualityFinding => 7,
        CatalogKind::Special => 8,
    }
}

fn graph_details(entry: &CatalogEntry) -> serde_json::Value {
    match SemanticEntity::try_from(entry) {
        Ok(entity) => graph_details_for_entity(&entity),
        Err(_) => json!({}),
    }
}

fn graph_details_for_entity(entity: &SemanticEntity) -> serde_json::Value {
    match entity {
        SemanticEntity::Table { metadata, .. } => json!({
            "database_id": &metadata.database_id,
            "schema_name": &metadata.schema_name,
            "table_name": &metadata.table_name,
            "relation_type": &metadata.relation_type,
            "row_count": &metadata.row_count,
            "column_count": &metadata.column_count,
            "preferred_query_surface": metadata.preferred_query_surface,
        }),
        SemanticEntity::Column { metadata, .. } => json!({
            "database_id": &metadata.database_id,
            "schema_name": &metadata.schema_name,
            "table_name": &metadata.table_name,
            "column_name": &metadata.column_name,
            "data_type": &metadata.data_type,
            "nullable": metadata.nullable,
            "primary_key": metadata.primary_key,
            "foreign_key": metadata.foreign_key.as_ref().map(|foreign_key| json!({
                "referenced_schema": &foreign_key.referenced_schema,
                "referenced_table": &foreign_key.referenced_table,
                "referenced_column": &foreign_key.referenced_column,
                "constraint_name": &foreign_key.constraint_name,
            })),
            "semantic_type": &metadata.semantic_type,
            "distinct_count": &metadata.distinct_count,
            "null_count": &metadata.null_count,
            "total_count": &metadata.total_count,
            "low_cardinality_enum": metadata.low_cardinality_enum,
        }),
        SemanticEntity::Relationship { metadata, .. } => json!({
            "database_id": &metadata.database_id,
            "source_table_id": &metadata.source_table_id,
            "target_table_id": &metadata.target_table_id,
            "source_schema": &metadata.source_schema,
            "source_table": &metadata.source_table,
            "source_column": &metadata.source_column,
            "target_schema": &metadata.target_schema,
            "target_table": &metadata.target_table,
            "target_column": &metadata.target_column,
            "source_cardinality": metadata.source_cardinality,
            "target_cardinality": metadata.target_cardinality,
            "relationship_kind": &metadata.relationship_kind,
            "confidence": &metadata.confidence,
        }),
        SemanticEntity::EnumValue { metadata, .. } => json!({
            "database_id": &metadata.database_id,
            "schema_name": &metadata.schema_name,
            "table_name": &metadata.table_name,
            "column_name": &metadata.column_name,
            "column_id": &metadata.column_id,
            "value": &metadata.value,
            "normalized_value": &metadata.normalized_value,
            "display_value": &metadata.display_value,
            "frequency": &metadata.frequency,
            "frequency_percentage": &metadata.frequency_percentage,
            "rank": &metadata.rank,
            "synonyms": &metadata.synonyms,
        }),
        SemanticEntity::Metric { metadata, .. } => json!({
            "formula": &metadata.formula,
            "source_tables": &metadata.source_tables,
            "source_columns": &metadata.source_columns,
            "synonyms": &metadata.synonyms,
        }),
        SemanticEntity::Knowledge { metadata, .. } => json!({
            "knowledge_type": &metadata.knowledge_type,
            "scope_tables": &metadata.scope_tables,
            "scope_columns": &metadata.scope_columns,
            "sql_expression": &metadata.sql_expression,
            "synonyms": &metadata.synonyms,
            "source_knowledge_id": &metadata.source_knowledge_id,
            "source_document_id": &metadata.source_document_id,
        }),
        SemanticEntity::Document { metadata, .. } => json!({
            "source_document_id": &metadata.source_document_id,
            "content_available": metadata.content_available,
            "content_source": &metadata.content_source,
            "extraction_status": &metadata.extraction_status,
            "extracted_knowledge_ids": &metadata.extracted_knowledge_ids,
        }),
        SemanticEntity::DataQualityFinding { metadata, .. } => json!({
            "database_id": &metadata.database_id,
            "schema_name": &metadata.schema_name,
            "table_name": &metadata.table_name,
            "column_name": &metadata.column_name,
            "finding_type": &metadata.finding_type,
            "scope_tables": &metadata.scope_tables,
            "scope_columns": &metadata.scope_columns,
            "typical_value_range": &metadata.typical_value_range,
            "validation_rules": &metadata.validation_rules,
        }),
        SemanticEntity::Special { .. } => json!({}),
    }
}

fn push_edge(
    edges: &mut Vec<GraphEdge>,
    edge_keys: &mut BTreeSet<GraphEdgeKey>,
    source: &CatalogEntry,
    link: &CatalogRelation,
    visible_ids: &HashSet<String>,
) {
    if !visible_ids.contains(&link.target_id) {
        return;
    }

    let key = (
        source.id.clone(),
        link.target_id.clone(),
        link.kind.clone(),
        None,
    );
    if !edge_keys.insert(key) {
        return;
    }

    edges.push(GraphEdge {
        id: edge_id(&source.id, &link.target_id, &link.kind),
        source: source.id.clone(),
        target: link.target_id.clone(),
        relation_type: link.kind.clone(),
        description: link.description.clone(),
        relationship_id: None,
        join: None,
    });
}

fn push_relationship_edge(
    edges: &mut Vec<GraphEdge>,
    edge_keys: &mut BTreeSet<GraphEdgeKey>,
    relationship: &CatalogEntry,
    visible_ids: &HashSet<String>,
) {
    let Ok(SemanticEntity::Relationship { metadata, .. }) =
        SemanticEntity::try_from(relationship.clone())
    else {
        return;
    };

    if !visible_ids.contains(&metadata.source_table_id)
        || !visible_ids.contains(&metadata.target_table_id)
    {
        return;
    }

    push_relationship_endpoint_edge(
        edges,
        edge_keys,
        relationship,
        &metadata.source_table_id,
        agent_fw_catalog::relation_kind::RELATIONSHIP_SOURCE_TABLE,
        Some(format!(
            "Relationship source table {}",
            metadata.source_table
        )),
    );
    push_relationship_endpoint_edge(
        edges,
        edge_keys,
        relationship,
        &metadata.target_table_id,
        agent_fw_catalog::relation_kind::RELATIONSHIP_TARGET_TABLE,
        Some(format!(
            "Relationship target table {}",
            metadata.target_table
        )),
    );

    let relation_type = agent_fw_catalog::relation_kind::REFERENCES_TABLE.to_string();
    let key = (
        metadata.source_table_id.clone(),
        metadata.target_table_id.clone(),
        relation_type.clone(),
        Some(relationship.id.clone()),
    );
    if !edge_keys.insert(key) {
        return;
    }

    let description = format!(
        "{}.{} -> {}.{}",
        metadata.source_table,
        metadata.source_column,
        metadata.target_table,
        metadata.target_column
    );
    let join = relationship_join_metadata(&metadata);

    edges.push(GraphEdge {
        id: relationship_edge_id(
            &metadata.source_table_id,
            &metadata.target_table_id,
            &relation_type,
            &relationship.id,
        ),
        source: metadata.source_table_id,
        target: metadata.target_table_id,
        relation_type,
        description: Some(description),
        relationship_id: Some(relationship.id.clone()),
        join: Some(join),
    });
}

fn push_relationship_endpoint_edge(
    edges: &mut Vec<GraphEdge>,
    edge_keys: &mut BTreeSet<GraphEdgeKey>,
    relationship: &CatalogEntry,
    target_id: &str,
    relation_type: &str,
    description: Option<String>,
) {
    let key = (
        relationship.id.clone(),
        target_id.to_string(),
        relation_type.to_string(),
        None,
    );
    if !edge_keys.insert(key) {
        return;
    }

    edges.push(GraphEdge {
        id: edge_id(&relationship.id, target_id, relation_type),
        source: relationship.id.clone(),
        target: target_id.to_string(),
        relation_type: relation_type.to_string(),
        description,
        relationship_id: None,
        join: None,
    });
}

fn relationship_join_metadata(metadata: &RelationshipMetadata) -> GraphJoinMetadata {
    GraphJoinMetadata {
        source_table_id: metadata.source_table_id.clone(),
        target_table_id: metadata.target_table_id.clone(),
        source_column: metadata.source_column.clone(),
        target_column: metadata.target_column.clone(),
        source_cardinality: format!("{:?}", metadata.source_cardinality).to_lowercase(),
        target_cardinality: format!("{:?}", metadata.target_cardinality).to_lowercase(),
        relationship_kind: metadata.relationship_kind.clone(),
        confidence: metadata.confidence.map(|value| value.to_string()),
    }
}

fn edge_id(source: &str, target: &str, relation_type: &str) -> String {
    format!("{source}--{relation_type}--{target}")
}

fn relationship_edge_id(
    source: &str,
    target: &str,
    relation_type: &str,
    relationship_id: &str,
) -> String {
    format!("{relationship_id}--{source}--{relation_type}--{target}")
}

const VIEWER_CSS: &str = r#"
:root {
  color-scheme: light;
  --bg: #edf2f7;
  --panel: #ffffff;
  --panel-soft: #f8fafc;
  --canvas: #07111f;
  --canvas-grid: rgba(148, 163, 184, 0.12);
  --line: #cbd5e1;
  --line-strong: #94a3b8;
  --text: #18202b;
  --muted: #64748b;
  --teal: #0f766e;
  --blue: #2563eb;
  --amber: #b45309;
  --rose: #be123c;
}
* {
  box-sizing: border-box;
}
body {
  margin: 0;
  background: var(--bg);
  color: var(--text);
  font: 14px/1.45 ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}
button,
input {
  font: inherit;
}
button {
  border: 1px solid var(--line);
  border-radius: 7px;
  background: var(--panel);
  color: var(--text);
  cursor: pointer;
}
button:hover,
button:focus-visible {
  border-color: var(--blue);
  outline: none;
}
#app {
  display: grid;
  grid-template-columns: minmax(200px, 220px) minmax(0, 1fr) minmax(230px, 260px);
  gap: 12px;
  height: 100vh;
  padding: 12px;
}
#explorer,
#inspector,
#toolbar {
  background: var(--panel);
  border: 1px solid var(--line);
  border-radius: 8px;
  box-shadow: 0 10px 28px rgba(15, 23, 42, 0.08);
}
#explorer,
#inspector {
  min-width: 0;
  overflow: auto;
  padding: 14px;
}
main {
  min-width: 0;
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  gap: 12px;
}
#toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-height: 56px;
  padding: 10px 12px;
}
#graph {
  min-height: 0;
  border: 1px solid var(--line);
  border-radius: 10px;
  overflow: hidden;
  background-color: var(--canvas);
  background-image:
    linear-gradient(var(--canvas-grid) 1px, transparent 1px),
    linear-gradient(90deg, var(--canvas-grid) 1px, transparent 1px);
  background-size: 42px 42px;
}
h1,
h2,
h3,
p {
  margin-top: 0;
}
h1 {
  font-size: 18px;
  margin-bottom: 4px;
}
h2 {
  font-size: 15px;
  margin-bottom: 8px;
}
h3 {
  font-size: 13px;
  margin: 16px 0 8px;
  color: var(--muted);
  text-transform: uppercase;
}
input {
  width: 100%;
  border: 1px solid var(--line);
  border-radius: 7px;
  padding: 8px 10px;
  color: var(--text);
  background: var(--panel);
}
.toolbar-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.toolbar-actions button {
  min-height: 34px;
  padding: 7px 10px;
}
.summary-line {
  color: var(--muted);
  text-align: right;
}
.stats,
.pills {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin: 12px 0;
}
.stat,
.pill {
  display: inline-flex;
  align-items: center;
  border-radius: 999px;
  padding: 4px 8px;
  font-size: 12px;
  border: 1px solid #bae6fd;
  background: #eff6ff;
  color: #1d4ed8;
}
.pill.table {
  border-color: #99f6e4;
  background: #ecfdf5;
  color: var(--teal);
}
.pill.relationship {
  border-color: #bfdbfe;
  background: #eff6ff;
  color: var(--blue);
}
.pill.metric {
  border-color: #fed7aa;
  background: #fff7ed;
  color: var(--amber);
}
.pill.knowledge,
.pill.document {
  border-color: #fecdd3;
  background: #fff1f2;
  color: var(--rose);
}
.item {
  display: block;
  width: 100%;
  min-height: 54px;
  margin-top: 6px;
  padding: 8px 10px;
  text-align: left;
  background: var(--panel-soft);
}
.item.active {
  border-color: var(--blue);
  box-shadow: inset 3px 0 0 var(--blue);
}
.item strong {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.muted {
  color: var(--muted);
}
.mono {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace;
}
.empty {
  margin-top: 12px;
  padding: 12px;
  border: 1px dashed var(--line-strong);
  border-radius: 8px;
  color: var(--muted);
}
.notice {
  margin: 12px 0;
  padding: 10px;
  border: 1px solid #fed7aa;
  border-radius: 8px;
  background: #fff7ed;
  color: #92400e;
}
pre {
  max-height: 340px;
  overflow: auto;
  white-space: pre-wrap;
  word-break: break-word;
  margin: 0;
  padding: 10px;
  border-radius: 8px;
  background: #0f172a;
  color: #dbeafe;
  font-size: 12px;
}
#load-error {
  margin: 24px;
  padding: 16px;
  border: 1px solid #fca5a5;
  border-radius: 8px;
  background: #fef2f2;
  color: #991b1b;
}
@media (max-width: 760px) {
  #app {
    grid-template-columns: 1fr;
    height: auto;
    min-height: 100vh;
  }
  #graph {
    height: 72vh;
  }
  #toolbar,
  .summary-line {
    align-items: flex-start;
    text-align: left;
  }
}
"#;

const VIEWER_JS: &str = r##"
(function () {
  const dataEl = document.getElementById("catalog-graph-data");
  const graph = JSON.parse(dataEl.textContent);
  const explorer = document.getElementById("explorer");
  const inspector = document.getElementById("inspector");
  const toolbar = document.getElementById("toolbar");
  const graphEl = document.getElementById("graph");
  let graphView = null;
  let selected = null;
  let showLabels = true;
  let frozen = false;

  function h(value) {
    return String(value == null ? "" : value)
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll("\"", "&quot;")
      .replaceAll("'", "&#39;");
  }

  function nodeId(value) {
    return typeof value === "object" && value ? value.id : value;
  }

  function byId(id) {
    return graph.nodes.find((node) => node.id === id);
  }

  function edgeEndpointLabel(value) {
    const node = byId(nodeId(value));
    return node ? node.label : nodeId(value);
  }

  function selectedNodeId() {
    return selected && !(selected.source && selected.target) ? selected.id : null;
  }

  function selectedEdgeId() {
    return selected && selected.source && selected.target ? selected.id : null;
  }

  function kindColor(kind) {
    return {
      table: "#0f766e",
      column: "#64748b",
      relationship: "#2563eb",
      metric: "#b45309",
      document: "#7c3aed",
      knowledge: "#be123c"
    }[kind] || "#475569";
  }

  function raw(value) {
    return `<pre>${h(JSON.stringify(value, null, 2))}</pre>`;
  }

  function cleanEdge(edge) {
    return {
      id: edge.id,
      source: nodeId(edge.source),
      target: nodeId(edge.target),
      relationType: edge.relationType,
      description: edge.description || null,
      relationshipId: edge.relationshipId || null,
      join: edge.join || null
    };
  }

  function renderToolbar() {
    toolbar.innerHTML = `
      <div class="toolbar-actions">
        <button id="fit" type="button">Fit</button>
        <button id="center" type="button">Center</button>
        <button id="freeze" type="button">${frozen ? "Resume" : "Freeze"}</button>
        <button id="labels" type="button">${showLabels ? "Hide Labels" : "Show Labels"}</button>
      </div>
      <div class="summary-line">
        ${h(graph.summary.nodeCount)} nodes / ${h(graph.summary.edgeCount)} edges${graph.summary.truncated ? ` / ${h(graph.summary.omittedNodeCount)} omitted` : ""}
      </div>
    `;
    document.getElementById("fit").onclick = () => graphView && graphView.zoomToFit(450, 80);
    document.getElementById("center").onclick = () => graphView && graphView.cameraPosition({ x: 0, y: 0, z: 600 }, { x: 0, y: 0, z: 0 }, 600);
    document.getElementById("freeze").onclick = () => {
      frozen = !frozen;
      if (graphView) {
        if (frozen) graphView.pauseAnimation();
        else graphView.resumeAnimation();
      }
      renderToolbar();
    };
    document.getElementById("labels").onclick = () => {
      showLabels = !showLabels;
      applyLabelMode();
      if (graphView) {
        graphView.refresh();
      }
      renderToolbar();
    };
  }

  function renderExplorer() {
    const counts = Object.entries(graph.summary.countsByKind || {})
      .map(([kind, count]) => `<span class="pill ${h(kind)}">${h(kind)}: ${h(count)}</span>`)
      .join("");
    const truncationNotice = graph.summary.truncated
      ? `<div class="notice">Some nodes are omitted: ${h(graph.summary.omittedNodeCount)} hidden. Increase <span class="mono">--max-nodes</span> to inspect more of the catalog.</div>`
      : "";
    explorer.innerHTML = `
      <h1>Catalog Graph</h1>
      <p class="muted mono">${h(graph.summary.tenantId)} / ${h(graph.summary.workspaceId)}</p>
      <div class="stats">
        <span class="stat">${h(graph.summary.nodeCount)} nodes</span>
        <span class="stat">${h(graph.summary.edgeCount)} edges</span>
      </div>
      ${truncationNotice}
      <input id="search" placeholder="Search nodes and edges" autocomplete="off">
      <div class="pills">${counts}</div>
      <h3>Nodes</h3>
      <div id="node-list"></div>
      <h3>Edges</h3>
      <div id="edge-list"></div>
    `;
    const search = document.getElementById("search");
    search.oninput = () => {
      renderNodeList(search.value);
      renderEdgeList(search.value);
    };
    renderNodeList("");
    renderEdgeList("");
  }

  function renderNodeList(query) {
    const needle = query.trim().toLowerCase();
    const list = document.getElementById("node-list");
    const nodes = graph.nodes
      .filter((node) => {
        const haystack = `${node.label} ${node.id} ${node.kind} ${node.qualifiedName || ""} ${(node.tags || []).join(" ")} ${JSON.stringify(node.details || {})}`.toLowerCase();
        return !needle || haystack.includes(needle);
      })
      .slice(0, 180);
    list.innerHTML = nodes
      .map((node) => `<button class="item ${selectedNodeId() === node.id ? "active" : ""}" type="button" data-node="${h(node.id)}">
        <strong>${h(node.label)}</strong>
        <span class="muted">${h(node.kind)} / degree ${h(node.degree)}</span>
      </button>`)
      .join("") || "<div class=\"empty\">No nodes match the search.</div>";
    for (const button of list.querySelectorAll("[data-node]")) {
      button.onclick = () => selectNode(button.dataset.node);
    }
  }

  function renderEdgeList(query) {
    const needle = query.trim().toLowerCase();
    const list = document.getElementById("edge-list");
    const edges = graph.edges
      .filter((edge) => {
        const sourceLabel = edgeEndpointLabel(edge.source);
        const targetLabel = edgeEndpointLabel(edge.target);
        const haystack = `${edge.id} ${edge.relationType} ${sourceLabel} ${targetLabel} ${edge.description || ""}`.toLowerCase();
        return !needle || haystack.includes(needle);
      })
      .slice(0, 120);
    list.innerHTML = edges
      .map((edge) => `<button class="item ${selectedEdgeId() === edge.id ? "active" : ""}" type="button" data-edge="${h(edge.id)}">
        <strong>${h(edge.relationType)}</strong>
        <span class="muted">${h(edgeEndpointLabel(edge.source))} -> ${h(edgeEndpointLabel(edge.target))}</span>
      </button>`)
      .join("") || "<div class=\"empty\">No edges match the search.</div>";
    for (const button of list.querySelectorAll("[data-edge]")) {
      button.onclick = () => selectEdge(graph.edges.find((edge) => edge.id === button.dataset.edge));
    }
  }

  function selectNode(id) {
    selected = byId(id);
    renderInspector();
    renderExplorer();
    if (graphView) {
      graphView.refresh();
    }
    if (graphView && selected) {
      graphView.cameraPosition(
        { x: selected.x || 0, y: selected.y || 0, z: (selected.z || 0) + 180 },
        selected,
        600
      );
    }
  }

  function selectEdge(edge) {
    if (!edge) return;
    selected = edge;
    renderInspector();
    renderExplorer();
    if (graphView) {
      graphView.refresh();
    }
  }

  function connectedEdges(node) {
    return graph.edges
      .filter((edge) => nodeId(edge.source) === node.id || nodeId(edge.target) === node.id)
      .slice(0, 40);
  }

  function renderInspector() {
    if (!selected) {
      inspector.innerHTML = "<h2>Inspector</h2><div class=\"empty\">Select a node or edge in the graph.</div>";
      return;
    }

    if (selected.source && selected.target) {
      const sourceLabel = edgeEndpointLabel(selected.source);
      const targetLabel = edgeEndpointLabel(selected.target);
      inspector.innerHTML = `
        <h2>Edge</h2>
        <p><span class="pill relationship">${h(selected.relationType)}</span></p>
        <p><strong>${h(sourceLabel)}</strong> -> <strong>${h(targetLabel)}</strong></p>
        ${selected.relationshipId ? `<p class="muted mono">${h(selected.relationshipId)}</p>` : ""}
        ${selected.description ? `<p>${h(selected.description)}</p>` : ""}
        ${selected.join ? `<h3>Join</h3><p><span class="mono">${h(selected.join.sourceColumn)}</span> = <span class="mono">${h(selected.join.targetColumn)}</span></p>${raw(selected.join)}` : ""}
        <h3>Raw</h3>${raw(cleanEdge(selected))}
      `;
      return;
    }

    const connected = connectedEdges(selected);
    inspector.innerHTML = `
      <h2>${h(selected.label)}</h2>
      <p><span class="pill ${h(selected.kind)}">${h(selected.kind)}</span></p>
      <p class="muted mono">${h(selected.id)}</p>
      ${selected.qualifiedName ? `<p class="mono">${h(selected.qualifiedName)}</p>` : ""}
      ${selected.content ? `<p>${h(selected.content)}</p>` : ""}
      ${(selected.tags || []).length ? `<div class="pills">${selected.tags.map((tag) => `<span class="pill">${h(tag)}</span>`).join("")}</div>` : ""}
      <h3>Connected Edges</h3>
      ${connected.map((edge) => `<button class="item" type="button" data-edge="${h(edge.id)}">
        <strong>${h(edge.relationType)}</strong>
        <span class="muted">${h(edgeEndpointLabel(edge.source))} -> ${h(edgeEndpointLabel(edge.target))}</span>
      </button>`).join("") || "<div class=\"empty\">No visible edges.</div>"}
      <h3>Details</h3>${raw(selected.details || {})}
    `;
    for (const button of inspector.querySelectorAll("[data-edge]")) {
      button.onclick = () => selectEdge(graph.edges.find((edge) => edge.id === button.dataset.edge));
    }
  }

  function renderGraph() {
    if (typeof ForceGraph3D !== "function") {
      throw new Error("3D graph libraries could not be loaded. Check network access for the CDN scripts.");
    }
    graphEl.innerHTML = "";
    graphView = ForceGraph3D()(graphEl)
      .graphData({ nodes: graph.nodes, links: graph.edges })
      .nodeId("id")
      .nodeLabel((node) => `${node.kind}: ${node.label}`)
      .nodeColor((node) => selectedNodeId() === node.id ? "#facc15" : kindColor(node.kind))
      .nodeVal((node) => Math.max(5, Math.min(20, 5 + node.degree)))
      .linkDirectionalArrowLength(4)
      .linkDirectionalArrowRelPos(1)
      .linkColor((edge) => selectedEdgeId() === edge.id ? "#f97316" : edge.join ? "#14b8a6" : "#94a3b8")
      .linkWidth((edge) => selectedEdgeId() === edge.id ? 5 : edge.join ? 2.8 : 1.2)
      .linkLabel((edge) => edge.relationType)
      .backgroundColor("#07111f")
      .onNodeClick((node) => selectNode(node.id))
      .onLinkClick((edge) => selectEdge(edge));

    applyLabelMode();
    if (frozen) {
      graphView.pauseAnimation();
    }
    setTimeout(() => graphView && graphView.zoomToFit(450, 80), 250);
  }

  function applyLabelMode() {
    if (!graphView) return;
    if (showLabels && typeof SpriteText === "function") {
      graphView.nodeThreeObjectExtend(true);
      graphView.nodeThreeObject((node) => {
        const sprite = new SpriteText(node.label);
        sprite.color = "#e2e8f0";
        sprite.textHeight = 5;
        sprite.position.y = 10;
        return sprite;
      });
    } else {
      graphView.nodeThreeObjectExtend(false);
      graphView.nodeThreeObject(null);
    }
  }

  function boot() {
    renderToolbar();
    renderExplorer();
    renderInspector();
    renderGraph();
  }

  try {
    boot();
  } catch (error) {
    document.getElementById("app").hidden = true;
    const box = document.getElementById("load-error");
    box.hidden = false;
    box.innerHTML = `<h2>Catalog graph viewer could not start</h2><p>${h(error.message)}</p><h3>Embedded graph JSON</h3>${raw(graph)}`;
  }
})();
"##;

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use agent_fw_catalog::{CatalogEntry, CatalogKind, CatalogRelation, CatalogScope};
    use agent_fw_core::{TenantId, WorkspaceId};
    use agent_fw_interpreter::MockCatalog;

    use super::{
        build_catalog_graph, graph_from_entries, render_html, render_json, CatalogGraph,
        GraphBuildOptions, GraphNode, GraphSummary,
    };

    fn catalog_entry(
        id: &str,
        kind: CatalogKind,
        name: &str,
        qualified_name: Option<&str>,
        metadata: serde_json::Value,
    ) -> CatalogEntry {
        CatalogEntry {
            id: id.to_string(),
            kind,
            name: name.to_string(),
            qualified_name: qualified_name.map(str::to_string),
            content: format!("{name} catalog entry"),
            tags: Vec::new(),
            links: Vec::new(),
            metadata,
        }
    }

    fn test_scope() -> CatalogScope {
        CatalogScope::new(
            TenantId::new_unchecked("tenant-a"),
            WorkspaceId::new_unchecked("workspace-a"),
        )
    }

    fn linked_catalog_entry(
        id: &str,
        kind: CatalogKind,
        name: &str,
        qualified_name: Option<&str>,
        links: Vec<CatalogRelation>,
    ) -> CatalogEntry {
        CatalogEntry {
            id: id.to_string(),
            kind,
            name: name.to_string(),
            qualified_name: qualified_name.map(str::to_string),
            content: format!("{name} catalog entry"),
            tags: Vec::new(),
            links,
            metadata: serde_json::json!({}),
        }
    }

    #[test]
    fn truncation_keeps_tables_and_relationships_before_metadata_nodes() {
        let graph = graph_from_entries(
            &test_scope(),
            vec![
                catalog_entry(
                    "document:orders",
                    CatalogKind::Document,
                    "orders docs",
                    Some("docs.orders"),
                    serde_json::json!({}),
                ),
                catalog_entry(
                    "knowledge:orders",
                    CatalogKind::Knowledge,
                    "orders knowledge",
                    Some("knowledge.orders"),
                    serde_json::json!({}),
                ),
                catalog_entry(
                    "metric:revenue",
                    CatalogKind::Metric,
                    "revenue",
                    Some("metrics.revenue"),
                    serde_json::json!({}),
                ),
                catalog_entry(
                    "table:public.orders",
                    CatalogKind::Table,
                    "orders",
                    Some("public.orders"),
                    serde_json::json!({}),
                ),
                catalog_entry(
                    "column:public.orders.id",
                    CatalogKind::Column,
                    "id",
                    Some("public.orders.id"),
                    serde_json::json!({}),
                ),
                catalog_entry(
                    "relationship:orders.customer_id:customers.id",
                    CatalogKind::Relationship,
                    "orders_customer_id_customers_id",
                    Some("relationships.orders_customers"),
                    serde_json::json!({}),
                ),
                catalog_entry(
                    "table:public.customers",
                    CatalogKind::Table,
                    "customers",
                    Some("public.customers"),
                    serde_json::json!({}),
                ),
            ],
            3,
        );

        let node_ids: Vec<&str> = graph.nodes.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(
            node_ids,
            vec![
                "table:public.customers",
                "table:public.orders",
                "relationship:orders.customer_id:customers.id",
            ]
        );
        assert!(graph.summary.truncated);
        assert_eq!(graph.summary.omitted_node_count, 4);
    }

    #[test]
    fn graph_nodes_use_public_kinds_typed_details_and_hide_special() {
        let graph = graph_from_entries(
            &test_scope(),
            vec![
                catalog_entry(
                    "enum:public.orders.status.paid",
                    CatalogKind::Enum,
                    "paid",
                    Some("public.orders.status.paid"),
                    serde_json::json!({
                        "databaseId": "warehouse",
                        "schemaName": "public",
                        "tableName": "orders",
                        "columnName": "status",
                        "columnId": "column:public.orders.status",
                        "value": "paid",
                        "normalizedValue": "paid",
                        "displayValue": "Paid",
                        "frequency": 8,
                        "frequencyPercentage": 80.0,
                        "rank": 1,
                        "synonyms": ["complete"]
                    }),
                ),
                catalog_entry(
                    "quality:public.orders.status",
                    CatalogKind::DataQualityFinding,
                    "orders status quality",
                    Some("public.orders.status.quality"),
                    serde_json::json!({
                        "databaseId": "warehouse",
                        "schemaName": "public",
                        "tableName": "orders",
                        "columnName": "status",
                        "findingType": "enum_drift",
                        "scopeTables": ["orders"],
                        "scopeColumns": ["orders.status"],
                        "typicalValueRange": "paid|refunded",
                        "validationRules": ["status in allowed set"]
                    }),
                ),
                catalog_entry(
                    "special:hidden",
                    CatalogKind::Special,
                    "hidden",
                    None,
                    serde_json::json!({"raw": "storage-only"}),
                ),
            ],
            10,
        );

        let kinds: BTreeSet<&str> = graph.nodes.iter().map(|node| node.kind.as_str()).collect();
        assert_eq!(
            kinds,
            BTreeSet::from(["data_quality_finding", "enum_value"])
        );
        assert_eq!(graph.summary.counts_by_kind.get("special"), None);

        let enum_node = graph
            .nodes
            .iter()
            .find(|node| node.id == "enum:public.orders.status.paid")
            .expect("enum value node should be rendered");
        assert_eq!(enum_node.kind, "enum_value");
        assert_eq!(enum_node.details["normalized_value"].as_str(), Some("paid"));
        assert!(enum_node.details.get("normalizedValue").is_none());

        let quality_node = graph
            .nodes
            .iter()
            .find(|node| node.id == "quality:public.orders.status")
            .expect("data quality node should be rendered");
        assert_eq!(
            quality_node.details["validation_rules"][0].as_str(),
            Some("status in allowed set")
        );
    }

    #[test]
    fn edges_are_sorted_deterministically_after_collection() {
        let graph = graph_from_entries(
            &test_scope(),
            vec![
                linked_catalog_entry(
                    "table:public.orders",
                    CatalogKind::Table,
                    "orders",
                    Some("public.orders"),
                    vec![
                        CatalogRelation {
                            target_id: "table:public.products".to_string(),
                            kind: "uses".to_string(),
                            description: None,
                        },
                        CatalogRelation {
                            target_id: "table:public.customers".to_string(),
                            kind: "uses".to_string(),
                            description: None,
                        },
                    ],
                ),
                catalog_entry(
                    "table:public.products",
                    CatalogKind::Table,
                    "products",
                    Some("public.products"),
                    serde_json::json!({}),
                ),
                catalog_entry(
                    "table:public.customers",
                    CatalogKind::Table,
                    "customers",
                    Some("public.customers"),
                    serde_json::json!({}),
                ),
            ],
            10,
        );

        let edge_targets: Vec<&str> = graph
            .edges
            .iter()
            .map(|edge| edge.target.as_str())
            .collect();
        assert_eq!(
            edge_targets,
            vec!["table:public.customers", "table:public.products"]
        );
    }

    #[tokio::test]
    async fn builds_graph_from_inline_relationship_metadata() {
        let catalog = MockCatalog::new();
        catalog
            .load(vec![
                catalog_entry(
                    "table:public.orders",
                    CatalogKind::Table,
                    "orders",
                    Some("public.orders"),
                    serde_json::json!({}),
                ),
                catalog_entry(
                    "table:public.products",
                    CatalogKind::Table,
                    "products",
                    Some("public.products"),
                    serde_json::json!({}),
                ),
                catalog_entry(
                    "relationship:orders.product_id:products.id",
                    CatalogKind::Relationship,
                    "orders_product_id_products_id",
                    None,
                    serde_json::json!({
                        "databaseId": "warehouse",
                        "sourceTableId": "table:public.orders",
                        "targetTableId": "table:public.products",
                        "sourceSchema": "public",
                        "sourceTable": "orders",
                        "sourceColumn": "product_id",
                        "targetSchema": "public",
                        "targetTable": "products",
                        "targetColumn": "id",
                        "sourceCardinality": "many",
                        "targetCardinality": "one",
                        "relationshipKind": "foreign_key",
                        "confidence": 1.0
                    }),
                ),
            ])
            .await;

        let scope = CatalogScope::new(
            TenantId::new_unchecked("tenant-a"),
            WorkspaceId::new_unchecked("workspace-a"),
        );

        let graph = build_catalog_graph(
            &catalog,
            &scope,
            GraphBuildOptions {
                include_columns: false,
                max_nodes: 50,
            },
        )
        .await
        .unwrap();

        assert_eq!(graph.summary.tenant_id, "tenant-a");
        assert_eq!(graph.summary.workspace_id, "workspace-a");
        assert!(graph
            .nodes
            .iter()
            .any(|node| node.id == "table:public.orders"));

        let edge = graph
            .edges
            .iter()
            .find(|edge| {
                edge.source == "table:public.orders"
                    && edge.target == "table:public.products"
                    && edge.join.is_some()
            })
            .expect("expected orders -> products join edge");

        let join = edge.join.as_ref().unwrap();
        assert_eq!(edge.relation_type, "references_table");
        assert_eq!(
            edge.relationship_id.as_deref(),
            Some("relationship:orders.product_id:products.id")
        );
        assert_eq!(join.source_column, "product_id");
        assert_eq!(join.target_column, "id");
        assert_eq!(join.source_cardinality, "many");
        assert_eq!(join.target_cardinality, "one");
        assert_eq!(join.confidence.as_deref(), Some("1"));
        assert_eq!(graph.summary.node_count, 3);
        assert_eq!(graph.summary.edge_count, 3);
        assert_eq!(graph.summary.counts_by_kind.get("table").copied(), Some(2));
        assert_eq!(
            graph.summary.counts_by_kind.get("relationship").copied(),
            Some(1)
        );
        assert_eq!(
            graph
                .nodes
                .iter()
                .find(|node| node.id == "table:public.orders")
                .map(|node| node.degree),
            Some(2)
        );
        assert_eq!(
            graph
                .nodes
                .iter()
                .find(|node| node.id == "table:public.products")
                .map(|node| node.degree),
            Some(2)
        );
        assert_eq!(
            graph
                .nodes
                .iter()
                .find(|node| node.id == "relationship:orders.product_id:products.id")
                .map(|node| node.degree),
            Some(2)
        );
    }

    #[tokio::test]
    async fn relationship_metadata_connects_relationship_vertex_to_endpoint_tables() {
        let relationship_id = "relationship:subsegments.segment_id:segments.segment_id";
        let source_id = "table:public.dim_subsegments";
        let target_id = "table:public.dim_segments";
        let catalog = MockCatalog::new();
        catalog
            .load(vec![
                catalog_entry(
                    source_id,
                    CatalogKind::Table,
                    "public.dim_subsegments",
                    Some("public.dim_subsegments"),
                    serde_json::json!({}),
                ),
                catalog_entry(
                    target_id,
                    CatalogKind::Table,
                    "public.dim_segments",
                    Some("public.dim_segments"),
                    serde_json::json!({}),
                ),
                catalog_entry(
                    relationship_id,
                    CatalogKind::Relationship,
                    "public.dim_subsegments -> public.dim_segments",
                    None,
                    serde_json::json!({
                        "databaseId": "warehouse",
                        "sourceTableId": source_id,
                        "targetTableId": target_id,
                        "sourceSchema": "public",
                        "sourceTable": "public.dim_subsegments",
                        "sourceColumn": "segment_id",
                        "targetSchema": "public",
                        "targetTable": "public.dim_segments",
                        "targetColumn": "segment_id",
                        "sourceCardinality": "many",
                        "targetCardinality": "one",
                        "relationshipKind": "one-to-many",
                        "confidence": 1.0
                    }),
                ),
            ])
            .await;

        let graph = build_catalog_graph(
            &catalog,
            &test_scope(),
            GraphBuildOptions {
                include_columns: false,
                max_nodes: 50,
            },
        )
        .await
        .unwrap();

        assert!(graph.edges.iter().any(|edge| {
            edge.source == relationship_id
                && edge.target == source_id
                && edge.relation_type == agent_fw_catalog::relation_kind::RELATIONSHIP_SOURCE_TABLE
        }));
        assert!(graph.edges.iter().any(|edge| {
            edge.source == relationship_id
                && edge.target == target_id
                && edge.relation_type == agent_fw_catalog::relation_kind::RELATIONSHIP_TARGET_TABLE
        }));
    }

    #[tokio::test]
    async fn preserves_multiple_relationship_edges_for_the_same_table_pair() {
        let catalog = MockCatalog::new();
        catalog
            .load(vec![
                catalog_entry(
                    "table:public.orders",
                    CatalogKind::Table,
                    "orders",
                    Some("public.orders"),
                    serde_json::json!({}),
                ),
                catalog_entry(
                    "table:public.customers",
                    CatalogKind::Table,
                    "customers",
                    Some("public.customers"),
                    serde_json::json!({}),
                ),
                catalog_entry(
                    "relationship:orders.customer_id:customers.id",
                    CatalogKind::Relationship,
                    "orders_customer_id_customers_id",
                    None,
                    serde_json::json!({
                        "databaseId": "warehouse",
                        "sourceTableId": "table:public.orders",
                        "targetTableId": "table:public.customers",
                        "sourceSchema": "public",
                        "sourceTable": "orders",
                        "sourceColumn": "customer_id",
                        "targetSchema": "public",
                        "targetTable": "customers",
                        "targetColumn": "id",
                        "sourceCardinality": "many",
                        "targetCardinality": "one",
                        "relationshipKind": "foreign_key",
                        "confidence": 0.99
                    }),
                ),
                catalog_entry(
                    "relationship:orders.billing_customer_id:customers.id",
                    CatalogKind::Relationship,
                    "orders_billing_customer_id_customers_id",
                    None,
                    serde_json::json!({
                        "databaseId": "warehouse",
                        "sourceTableId": "table:public.orders",
                        "targetTableId": "table:public.customers",
                        "sourceSchema": "public",
                        "sourceTable": "orders",
                        "sourceColumn": "billing_customer_id",
                        "targetSchema": "public",
                        "targetTable": "customers",
                        "targetColumn": "id",
                        "sourceCardinality": "many",
                        "targetCardinality": "one",
                        "relationshipKind": "foreign_key",
                        "confidence": 0.98
                    }),
                ),
            ])
            .await;

        let graph = build_catalog_graph(
            &catalog,
            &test_scope(),
            GraphBuildOptions {
                include_columns: false,
                max_nodes: 50,
            },
        )
        .await
        .unwrap();

        let relationship_edges: Vec<_> = graph
            .edges
            .iter()
            .filter(|edge| {
                edge.source == "table:public.orders"
                    && edge.target == "table:public.customers"
                    && edge.relation_type == "references_table"
                    && edge.join.is_some()
            })
            .collect();

        assert_eq!(relationship_edges.len(), 2);

        let relationship_ids: BTreeSet<&str> = relationship_edges
            .iter()
            .filter_map(|edge| edge.relationship_id.as_deref())
            .collect();
        assert_eq!(
            relationship_ids,
            BTreeSet::from([
                "relationship:orders.billing_customer_id:customers.id",
                "relationship:orders.customer_id:customers.id",
            ])
        );

        let source_columns: BTreeSet<&str> = relationship_edges
            .iter()
            .filter_map(|edge| edge.join.as_ref())
            .map(|join| join.source_column.as_str())
            .collect();
        assert_eq!(
            source_columns,
            BTreeSet::from(["billing_customer_id", "customer_id"])
        );

        let edge_ids: BTreeSet<&str> = relationship_edges
            .iter()
            .map(|edge| edge.id.as_str())
            .collect();
        assert_eq!(edge_ids.len(), 2);
        assert_eq!(graph.summary.edge_count, 6);
    }

    #[tokio::test]
    async fn graph_includes_context_relation_targets_without_loading_all_columns() {
        let catalog = MockCatalog::new();
        catalog
            .load(vec![
                catalog_entry(
                    "table:public.orders",
                    CatalogKind::Table,
                    "orders",
                    Some("public.orders"),
                    serde_json::json!({}),
                ),
                linked_catalog_entry(
                    "column:public.orders.status",
                    CatalogKind::Column,
                    "status",
                    Some("public.orders.status"),
                    vec![CatalogRelation {
                        target_id: "table:public.orders".to_string(),
                        kind: agent_fw_catalog::relation_kind::BELONGS_TO.to_string(),
                        description: Some("Column belongs to orders".to_string()),
                    }],
                ),
                linked_catalog_entry(
                    "knowledge:status-rule",
                    CatalogKind::Knowledge,
                    "Status rule",
                    None,
                    vec![CatalogRelation {
                        target_id: "column:public.orders.status".to_string(),
                        kind: agent_fw_catalog::relation_kind::KNOWLEDGE_APPLIES_TO.to_string(),
                        description: Some("Applies to order status".to_string()),
                    }],
                ),
                catalog_entry(
                    "column:public.orders.unrelated",
                    CatalogKind::Column,
                    "unrelated",
                    Some("public.orders.unrelated"),
                    serde_json::json!({}),
                ),
            ])
            .await;

        let graph = build_catalog_graph(
            &catalog,
            &test_scope(),
            GraphBuildOptions {
                include_columns: false,
                max_nodes: 50,
            },
        )
        .await
        .unwrap();

        let node_ids: BTreeSet<&str> = graph.nodes.iter().map(|node| node.id.as_str()).collect();
        assert!(node_ids.contains("column:public.orders.status"));
        assert!(!node_ids.contains("column:public.orders.unrelated"));
        assert!(graph.edges.iter().any(|edge| {
            edge.source == "knowledge:status-rule"
                && edge.target == "column:public.orders.status"
                && edge.relation_type == agent_fw_catalog::relation_kind::KNOWLEDGE_APPLIES_TO
        }));
        assert!(graph.edges.iter().any(|edge| {
            edge.source == "column:public.orders.status"
                && edge.target == "table:public.orders"
                && edge.relation_type == agent_fw_catalog::relation_kind::BELONGS_TO
        }));
    }

    #[tokio::test]
    async fn rejects_zero_max_nodes() {
        let catalog = MockCatalog::new();

        let error = build_catalog_graph(
            &catalog,
            &test_scope(),
            GraphBuildOptions {
                include_columns: false,
                max_nodes: 0,
            },
        )
        .await
        .expect_err("zero max_nodes should be rejected");

        assert_eq!(error.to_string(), "--max-nodes must be greater than 0");
    }

    #[test]
    fn renders_json_with_nodes_and_edges() {
        let graph = sample_graph();

        let rendered = render_json(&graph).unwrap();

        assert!(rendered.contains("\"nodes\""));
        assert!(rendered.contains("\"edges\""));
        assert!(rendered.contains("table:public.orders"));
    }

    #[test]
    fn renders_html_with_embedded_graph_and_viewer_root() {
        let graph = sample_graph();

        let rendered = render_html(&graph).unwrap();

        assert!(rendered.contains("id=\"catalog-graph-data\""));
        assert!(rendered.contains("id=\"explorer\""));
        assert!(rendered.contains("id=\"toolbar\""));
        assert!(rendered.contains("id=\"graph\""));
        assert!(rendered.contains("id=\"inspector\""));
        assert!(rendered.contains("id=\"load-error\""));
        assert!(rendered.contains("three@0.160.1"));
        assert!(rendered.contains("3d-force-graph"));
        assert!(rendered.contains("three-spritetext"));
        assert!(rendered.contains("table:public.orders"));
    }

    #[test]
    fn renders_html_with_explorer_edge_selection_and_truncation_hooks() {
        let graph = sample_graph();

        let rendered = render_html(&graph).unwrap();

        assert!(rendered.contains("id=\"edge-list\""));
        assert!(rendered.contains("function selectedNodeId()"));
        assert!(rendered.contains("function selectedEdgeId()"));
        assert!(rendered.contains("function cleanEdge(edge)"));
        assert!(rendered.contains("function applyLabelMode()"));
        assert!(rendered.contains("graphView.refresh()"));
        assert!(rendered.contains("Some nodes are omitted"));
        assert!(rendered.contains("--max-nodes"));
    }

    #[test]
    fn renders_html_with_script_safe_embedded_json() {
        let mut graph = sample_graph();
        graph.nodes[0].content = "</script><script>alert('x')</script><!--".to_string();

        let rendered = render_html(&graph).unwrap();

        assert!(!rendered.contains("</script><script>alert"));
        assert!(rendered.contains("<\\/script>"));
        assert!(rendered.contains("\\u003C!--"));
        assert!(!rendered.contains("<\\!--"));
        let embedded = embedded_graph_json(&rendered);
        let parsed: serde_json::Value = serde_json::from_str(embedded).unwrap();
        assert_eq!(
            parsed["nodes"][0]["content"].as_str(),
            Some("</script><script>alert('x')</script><!--")
        );
    }

    #[test]
    fn renders_html_with_mixed_case_script_safe_embedded_json() {
        let mut graph = sample_graph();
        graph.nodes[0].content = "</ScRiPt><script>alert('x')</script><!--".to_string();

        let rendered = render_html(&graph).unwrap();

        assert!(!rendered.contains("</ScRiPt><script>alert"));
        assert!(rendered.contains("<\\/ScRiPt>"));
        assert!(rendered.contains("\\u003C!--"));
        assert!(!rendered.contains("<\\!--"));
        let embedded = embedded_graph_json(&rendered);
        let parsed: serde_json::Value = serde_json::from_str(embedded).unwrap();
        assert_eq!(
            parsed["nodes"][0]["content"].as_str(),
            Some("</ScRiPt><script>alert('x')</script><!--")
        );
    }

    fn embedded_graph_json(rendered: &str) -> &str {
        let marker = r#"<script id="catalog-graph-data" type="application/json">"#;
        let start = rendered
            .find(marker)
            .expect("rendered HTML should include graph data script")
            + marker.len();
        let rest = &rendered[start..];
        let end = rest
            .find("</script>")
            .expect("rendered HTML should close graph data script");
        &rest[..end]
    }

    fn sample_graph() -> CatalogGraph {
        CatalogGraph {
            summary: GraphSummary {
                tenant_id: "tenant-a".to_string(),
                workspace_id: "workspace-a".to_string(),
                node_count: 1,
                edge_count: 0,
                omitted_node_count: 0,
                truncated: false,
                counts_by_kind: BTreeMap::from([("table".to_string(), 1)]),
            },
            nodes: vec![GraphNode {
                id: "table:public.orders".to_string(),
                label: "orders".to_string(),
                kind: "table".to_string(),
                qualified_name: Some("public.orders".to_string()),
                content: "Orders table".to_string(),
                tags: Vec::new(),
                details: serde_json::json!({}),
                degree: 0,
            }],
            edges: Vec::new(),
        }
    }
}
