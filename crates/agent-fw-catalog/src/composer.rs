//! Tagless final algebra for composing catalog entries into text.
//!
//! # CatalogComposer — The Algebra
//!
//! `CatalogComposer` defines a language for rendering [`CatalogEntry`] values
//! into textual context for LLM consumption. Programs written against
//! `C: CatalogComposer` are polymorphic — the same program produces XML
//! (for Claude), Markdown (for OpenAI), or any other format by choosing
//! a different interpreter.
//!
//! This is the catalog companion to [`CardAlg`](crate::card) in `agent-fw-plan`.
//! While `CardAlg` renders plan/sweep cards, `CatalogComposer` renders the
//! thing agents read most: schema context.
//!
//! # Laws
//!
//! **L1 (Determinism)**: `compose(entry, ctx)` produces the same `String`
//! for the same inputs, for any fixed interpreter.
//!
//! **L2 (Non-empty)**: `compose(entry, ctx)` always returns a non-empty string
//! for a valid `CatalogEntry` (at minimum the entry name).
//!
//! **L3 (Composition)**: `compose_many(entries, ctx)` equals the join of
//! individual `compose(entry, ctx)` calls with the interpreter's separator.
//!
//! # Shipped Interpreters
//!
//! - [`Markdown`] — OpenAI-family format (headers, bullet lists, code blocks)
//! - [`Xml`] — Claude-family format (structured XML tags)
//!
//! # Model Family Detection
//!
//! [`ModelFamily::detect`] classifies a model identifier string into the
//! appropriate family, driving automatic interpreter selection.
//!
//! # Example
//!
//! ```
//! use agent_fw_catalog::composer::*;
//! use agent_fw_catalog::entry::{CatalogEntry, CatalogKind};
//!
//! fn render_context<C: CatalogComposer>(c: &C, entry: &CatalogEntry) -> String {
//!     c.compose(entry, &ComposerContext::default())
//! }
//!
//! let entry = CatalogEntry {
//!     id: "tbl-1".into(),
//!     kind: CatalogKind::Table,
//!     name: "users".into(),
//!     qualified_name: Some("public.users".into()),
//!     content: "User accounts".into(),
//!     tags: vec!["core".into()],
//!     links: vec![],
//!     metadata: serde_json::json!({"row_count": 1000}),
//! };
//!
//! let md = render_context(&Markdown, &entry);
//! assert!(md.contains("users"));
//!
//! let xml = render_context(&Xml, &entry);
//! assert!(xml.contains("users"));
//! ```

use crate::entry::{CatalogEntry, CatalogKind};
use crate::semantic::SemanticEntity;

// ─── Domain Types ─────────────────────────────────────────────────────

/// The model family determines the output format convention.
///
/// Claude prefers structured XML; OpenAI-family models prefer Markdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelFamily {
    /// Claude models — XML output with structured tags.
    Claude,
    /// OpenAI-family models — Markdown with headers and lists.
    OpenAi,
}

impl ModelFamily {
    /// Detect model family from a model identifier string.
    ///
    /// Returns `Claude` for any string containing "claude", "anthropic",
    /// or "opus"/"sonnet"/"haiku" (Anthropic model codenames).
    /// Returns `OpenAi` for everything else.
    pub fn detect(model_id: &str) -> Self {
        let lower = model_id.to_lowercase();
        if lower.contains("claude")
            || lower.contains("anthropic")
            || lower.contains("opus")
            || lower.contains("sonnet")
            || lower.contains("haiku")
        {
            Self::Claude
        } else {
            Self::OpenAi
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::OpenAi => "openai",
        }
    }
}

/// The rendering variant controls verbosity and format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ComposerVariant {
    /// Full format with description, metadata, tags.
    #[default]
    Default,
    /// Compact single-line format for tool_use contexts (save tokens).
    ToolUse,
    /// Verbose format with all metadata, synonyms, raw payload.
    Verbose,
    /// Single-line summary for list views.
    Summary,
}

impl ComposerVariant {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::ToolUse => "tool_use",
            Self::Verbose => "verbose",
            Self::Summary => "summary",
        }
    }
}

/// Context passed to every compose call.
///
/// Carries variant selection, optional length limits, and matched terms
/// for highlighting in search result rendering.
#[derive(Debug, Clone)]
pub struct ComposerContext {
    pub variant: ComposerVariant,
    pub max_length: Option<usize>,
    pub matched_terms: Vec<String>,
}

impl Default for ComposerContext {
    fn default() -> Self {
        Self {
            variant: ComposerVariant::Default,
            max_length: None,
            matched_terms: Vec::new(),
        }
    }
}

impl ComposerContext {
    pub fn new(variant: ComposerVariant) -> Self {
        Self {
            variant,
            ..Default::default()
        }
    }

    pub fn with_max_length(mut self, max_length: usize) -> Self {
        self.max_length = Some(max_length);
        self
    }

    pub fn with_matched_terms(mut self, terms: Vec<String>) -> Self {
        self.matched_terms = terms;
        self
    }
}

// ─── CatalogComposer Trait ────────────────────────────────────────────

/// Tagless final algebra for catalog entry rendering.
///
/// Programs written generically against `C: CatalogComposer` are interpreted
/// by choosing a concrete type for `C`. This eliminates intermediate ASTs —
/// the program IS the rendering.
///
/// # Laws
///
/// - **L1 Determinism**: Same `(entry, ctx)` → same output for a fixed `C`.
/// - **L2 Non-empty**: Output is never empty for a valid entry.
/// - **L3 Composition**: `compose_many` = join of individual `compose` calls.
pub trait CatalogComposer {
    /// Render a single catalog entry to text.
    fn compose(&self, entry: &CatalogEntry, ctx: &ComposerContext) -> String;

    /// Render multiple entries, joined by the interpreter's natural separator.
    fn compose_many(&self, entries: &[CatalogEntry], ctx: &ComposerContext) -> String {
        let sep = self.separator();
        entries
            .iter()
            .map(|e| self.compose(e, ctx))
            .collect::<Vec<_>>()
            .join(&sep)
    }

    /// The separator used between entries in `compose_many`.
    fn separator(&self) -> String {
        "\n\n".to_string()
    }
}

// ─── Interpreter: Markdown (OpenAI family) ────────────────────────────

/// Markdown interpreter for catalog entries.
///
/// Produces GitHub-flavored Markdown suitable for OpenAI-family models.
pub struct Markdown;

impl CatalogComposer for Markdown {
    fn compose(&self, entry: &CatalogEntry, ctx: &ComposerContext) -> String {
        let output = match ctx.variant {
            ComposerVariant::Summary => md_summary(entry),
            ComposerVariant::ToolUse => md_tool_use(entry),
            ComposerVariant::Default => md_default(entry),
            ComposerVariant::Verbose => md_verbose(entry),
        };
        truncate_if_needed(&output, ctx.max_length)
    }
}

fn md_summary(entry: &CatalogEntry) -> String {
    let name = display_name(entry);
    let desc = truncate_str(&entry.content, 80);
    if desc.is_empty() {
        name
    } else {
        format!("{name}: {desc}")
    }
}

fn md_tool_use(entry: &CatalogEntry) -> String {
    let name = display_name(entry);
    let desc = truncate_str(&entry.content, 120);
    match entry.kind {
        CatalogKind::Table => {
            let rows = row_count_str(entry);
            if desc.is_empty() {
                format!("{name}{rows}")
            } else {
                format!("{name}{rows}: {desc}")
            }
        }
        CatalogKind::Column => {
            let dtype = data_type_str(entry);
            let nullable = nullable_str(entry);
            if desc.is_empty() {
                format!("{name} ({dtype}{nullable})")
            } else {
                format!("{name} ({dtype}{nullable}): {desc}")
            }
        }
        CatalogKind::Metric => {
            let formula = formula_str(entry);
            if formula.is_empty() {
                format!("{name}: {desc}")
            } else {
                format!("{name} = {formula}: {desc}")
            }
        }
        _ => {
            if desc.is_empty() {
                name
            } else {
                format!("{name}: {desc}")
            }
        }
    }
}

fn md_default(entry: &CatalogEntry) -> String {
    let name = display_name(entry);
    let mut lines = Vec::new();

    match entry.kind {
        CatalogKind::Table => {
            lines.push(format!("### {name}"));
            let rows = row_count_str(entry);
            if !rows.is_empty() {
                lines.push(format!("*{} rows*", rows.trim_start_matches(" (")));
            }
        }
        CatalogKind::Column => {
            let dtype = data_type_str(entry);
            let nullable = nullable_str(entry);
            lines.push(format!("- **{name}** ({dtype}{nullable})"));
        }
        CatalogKind::Metric => {
            lines.push(format!("### {name}"));
            let formula = formula_str(entry);
            if !formula.is_empty() {
                lines.push(format!("`{formula}`"));
            }
        }
        _ => {
            lines.push(format!("### {name}"));
        }
    }

    if !entry.content.is_empty() {
        lines.push(String::new());
        lines.push(entry.content.clone());
    }

    if !entry.tags.is_empty() {
        let tags: Vec<_> = entry.tags.iter().map(|t| format!("`{t}`")).collect();
        lines.push(String::new());
        lines.push(format!("Tags: {}", tags.join(", ")));
    }

    lines.join("\n")
}

fn md_verbose(entry: &CatalogEntry) -> String {
    let mut lines = Vec::new();
    let base = md_default(entry);
    lines.push(base);

    let metadata = typed_metadata_pairs(entry);
    if !metadata.is_empty() {
        lines.push(String::new());
        lines.push("**Metadata:**".to_string());
        for (key, value) in metadata {
            lines.push(format!("- {key}: {value}"));
        }
    }

    // Relations
    if !entry.links.is_empty() {
        lines.push(String::new());
        lines.push("**Relations:**".to_string());
        for rel in &entry.links {
            let desc = rel
                .description
                .as_deref()
                .map(|d| format!(" — {d}"))
                .unwrap_or_default();
            lines.push(format!("- {} → {}{desc}", rel.kind, rel.target_id));
        }
    }

    lines.join("\n")
}

// ─── Interpreter: Xml (Claude family) ─────────────────────────────────

/// XML interpreter for catalog entries.
///
/// Produces structured XML suitable for Claude-family models.
pub struct Xml;

impl CatalogComposer for Xml {
    fn compose(&self, entry: &CatalogEntry, ctx: &ComposerContext) -> String {
        let output = match ctx.variant {
            ComposerVariant::Summary => xml_summary(entry),
            ComposerVariant::ToolUse => xml_tool_use(entry),
            ComposerVariant::Default => xml_default(entry),
            ComposerVariant::Verbose => xml_verbose(entry),
        };
        truncate_if_needed(&output, ctx.max_length)
    }

    fn separator(&self) -> String {
        "\n".to_string()
    }
}

fn xml_summary(entry: &CatalogEntry) -> String {
    let name = display_name(entry);
    let kind = entry.kind.as_str();
    let desc = truncate_str(&entry.content, 80);
    if desc.is_empty() {
        format!("<{kind} name=\"{name}\"/>", name = escape_xml(&name))
    } else {
        format!(
            "<{kind} name=\"{name}\">{desc}</{kind}>",
            name = escape_xml(&name),
            desc = escape_xml(&desc),
        )
    }
}

fn xml_tool_use(entry: &CatalogEntry) -> String {
    let name = display_name(entry);
    let kind = entry.kind.as_str();
    let desc = truncate_str(&entry.content, 120);

    let mut attrs = format!("name=\"{}\"", escape_xml(&name));

    match entry.kind {
        CatalogKind::Table => {
            if let Some(count) = row_count_value(entry) {
                attrs.push_str(&format!(" rows=\"{count}\""));
            }
        }
        CatalogKind::Column => {
            let dtype = data_type_str(entry);
            if !dtype.is_empty() {
                attrs.push_str(&format!(" type=\"{}\"", escape_xml(&dtype)));
            }
            if is_nullable(entry) {
                attrs.push_str(" nullable=\"true\"");
            }
        }
        CatalogKind::Metric => {
            let formula = formula_str(entry);
            if !formula.is_empty() {
                attrs.push_str(&format!(" formula=\"{}\"", escape_xml(&formula)));
            }
        }
        _ => {}
    }

    if desc.is_empty() {
        format!("<{kind} {attrs}/>")
    } else {
        format!("<{kind} {attrs}>{desc}</{kind}>", desc = escape_xml(&desc),)
    }
}

fn xml_default(entry: &CatalogEntry) -> String {
    let name = display_name(entry);
    let kind = entry.kind.as_str();
    let mut parts = Vec::new();

    let mut attrs = format!("name=\"{}\"", escape_xml(&name));

    match entry.kind {
        CatalogKind::Table => {
            if let Some(count) = row_count_value(entry) {
                attrs.push_str(&format!(" rows=\"{count}\""));
            }
        }
        CatalogKind::Column => {
            let dtype = data_type_str(entry);
            if !dtype.is_empty() {
                attrs.push_str(&format!(" type=\"{}\"", escape_xml(&dtype)));
            }
            if is_nullable(entry) {
                attrs.push_str(" nullable=\"true\"");
            }
        }
        _ => {}
    }

    parts.push(format!("<{kind} {attrs}>"));

    if !entry.content.is_empty() {
        parts.push(format!(
            "  <description>{}</description>",
            escape_xml(&entry.content)
        ));
    }

    if !entry.tags.is_empty() {
        let tags_str = entry.tags.join(", ");
        parts.push(format!("  <tags>{}</tags>", escape_xml(&tags_str)));
    }

    parts.push(format!("</{kind}>"));
    parts.join("\n")
}

fn xml_verbose(entry: &CatalogEntry) -> String {
    let name = display_name(entry);
    let kind = entry.kind.as_str();
    let mut parts = Vec::new();

    parts.push(format!(
        "<{kind} name=\"{}\" id=\"{}\">",
        escape_xml(&name),
        escape_xml(&entry.id),
    ));

    if !entry.content.is_empty() {
        parts.push(format!(
            "  <description>{}</description>",
            escape_xml(&entry.content)
        ));
    }

    if !entry.tags.is_empty() {
        let tags_str = entry.tags.join(", ");
        parts.push(format!("  <tags>{}</tags>", escape_xml(&tags_str)));
    }

    let metadata = typed_metadata_pairs(entry);
    if !metadata.is_empty() {
        parts.push("  <metadata>".to_string());
        for (key, value) in metadata {
            parts.push(format!("    <{key}>{}</{key}>", escape_xml(&value)));
        }
        parts.push("  </metadata>".to_string());
    }

    // Relations
    if !entry.links.is_empty() {
        parts.push("  <relations>".to_string());
        for rel in &entry.links {
            let desc_attr = rel
                .description
                .as_deref()
                .map(|d| format!(" description=\"{}\"", escape_xml(d)))
                .unwrap_or_default();
            parts.push(format!(
                "    <relation type=\"{}\" target=\"{}\"{desc_attr}/>",
                escape_xml(&rel.kind),
                escape_xml(&rel.target_id),
            ));
        }
        parts.push("  </relations>".to_string());
    }

    parts.push(format!("</{kind}>"));
    parts.join("\n")
}

// ─── Convenience: Auto-dispatch by ModelFamily ────────────────────────

/// Compose an entry using the appropriate interpreter for the model family.
///
/// This is the primary entry point for most consumers. It detects the
/// model family and dispatches to [`Markdown`] or [`Xml`] accordingly.
pub fn compose(entry: &CatalogEntry, family: ModelFamily, ctx: &ComposerContext) -> String {
    match family {
        ModelFamily::OpenAi => Markdown.compose(entry, ctx),
        ModelFamily::Claude => Xml.compose(entry, ctx),
    }
}

/// Compose multiple entries for a given model family.
pub fn compose_many(
    entries: &[CatalogEntry],
    family: ModelFamily,
    ctx: &ComposerContext,
) -> String {
    match family {
        ModelFamily::OpenAi => Markdown.compose_many(entries, ctx),
        ModelFamily::Claude => Xml.compose_many(entries, ctx),
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────

fn display_name(entry: &CatalogEntry) -> String {
    entry
        .qualified_name
        .as_deref()
        .unwrap_or(&entry.name)
        .to_string()
}

fn semantic_entity(entry: &CatalogEntry) -> Option<SemanticEntity> {
    SemanticEntity::try_from(entry).ok()
}

fn row_count_value(entry: &CatalogEntry) -> Option<i64> {
    match semantic_entity(entry) {
        Some(SemanticEntity::Table { metadata, .. }) => metadata.row_count,
        _ => None,
    }
}

fn row_count_str(entry: &CatalogEntry) -> String {
    row_count_value(entry)
        .map(|n| format!(" ({n} rows)"))
        .unwrap_or_default()
}

fn data_type_str(entry: &CatalogEntry) -> String {
    match semantic_entity(entry) {
        Some(SemanticEntity::Column { metadata, .. }) => metadata.data_type,
        _ => String::new(),
    }
}

fn formula_str(entry: &CatalogEntry) -> String {
    match semantic_entity(entry) {
        Some(SemanticEntity::Metric { metadata, .. }) => metadata.formula.unwrap_or_default(),
        _ => String::new(),
    }
}

fn is_nullable(entry: &CatalogEntry) -> bool {
    match semantic_entity(entry) {
        Some(SemanticEntity::Column { metadata, .. }) => metadata.nullable,
        _ => false,
    }
}

fn nullable_str(entry: &CatalogEntry) -> &'static str {
    if is_nullable(entry) {
        "?"
    } else {
        ""
    }
}

fn typed_metadata_pairs(entry: &CatalogEntry) -> Vec<(&'static str, String)> {
    let Some(entity) = semantic_entity(entry) else {
        return Vec::new();
    };
    let mut pairs = Vec::new();
    match entity {
        SemanticEntity::Table { metadata, .. } => {
            pairs.push(("database_id", metadata.database_id));
            pairs.push(("schema_name", metadata.schema_name));
            pairs.push(("table_name", metadata.table_name));
            push_optional(&mut pairs, "relation_type", metadata.relation_type);
            push_optional(
                &mut pairs,
                "row_count",
                metadata.row_count.map(|value| value.to_string()),
            );
            push_optional(
                &mut pairs,
                "column_count",
                metadata.column_count.map(|value| value.to_string()),
            );
            pairs.push((
                "preferred_query_surface",
                metadata.preferred_query_surface.to_string(),
            ));
        }
        SemanticEntity::Column { metadata, .. } => {
            pairs.push(("database_id", metadata.database_id));
            pairs.push(("schema_name", metadata.schema_name));
            pairs.push(("table_name", metadata.table_name));
            pairs.push(("column_name", metadata.column_name));
            pairs.push(("data_type", metadata.data_type));
            pairs.push(("nullable", metadata.nullable.to_string()));
            pairs.push(("primary_key", metadata.primary_key.to_string()));
            push_optional(&mut pairs, "semantic_type", metadata.semantic_type);
            push_optional(
                &mut pairs,
                "distinct_count",
                metadata.distinct_count.map(|value| value.to_string()),
            );
            push_optional(
                &mut pairs,
                "null_count",
                metadata.null_count.map(|value| value.to_string()),
            );
            push_optional(
                &mut pairs,
                "total_count",
                metadata.total_count.map(|value| value.to_string()),
            );
            pairs.push((
                "low_cardinality_enum",
                metadata.low_cardinality_enum.to_string(),
            ));
        }
        SemanticEntity::Relationship { metadata, .. } => {
            pairs.push(("database_id", metadata.database_id));
            pairs.push(("source_table_id", metadata.source_table_id));
            pairs.push(("target_table_id", metadata.target_table_id));
            pairs.push(("source_schema", metadata.source_schema));
            pairs.push(("source_table", metadata.source_table));
            pairs.push(("source_column", metadata.source_column));
            pairs.push(("target_schema", metadata.target_schema));
            pairs.push(("target_table", metadata.target_table));
            pairs.push(("target_column", metadata.target_column));
            pairs.push((
                "source_cardinality",
                format!("{:?}", metadata.source_cardinality),
            ));
            pairs.push((
                "target_cardinality",
                format!("{:?}", metadata.target_cardinality),
            ));
            pairs.push(("relationship_kind", metadata.relationship_kind));
            push_optional(
                &mut pairs,
                "confidence",
                metadata.confidence.map(|value| value.to_string()),
            );
        }
        SemanticEntity::EnumValue { metadata, .. } => {
            pairs.push(("database_id", metadata.database_id));
            pairs.push(("schema_name", metadata.schema_name));
            pairs.push(("table_name", metadata.table_name));
            pairs.push(("column_name", metadata.column_name));
            pairs.push(("column_id", metadata.column_id));
            pairs.push(("value", metadata.value));
            pairs.push(("normalized_value", metadata.normalized_value));
            push_optional(&mut pairs, "display_value", metadata.display_value);
            push_optional(
                &mut pairs,
                "frequency",
                metadata.frequency.map(|value| value.to_string()),
            );
            push_optional(
                &mut pairs,
                "frequency_percentage",
                metadata.frequency_percentage.map(|value| value.to_string()),
            );
            push_optional(
                &mut pairs,
                "rank",
                metadata.rank.map(|value| value.to_string()),
            );
            push_vec(&mut pairs, "synonyms", metadata.synonyms);
        }
        SemanticEntity::Metric { metadata, .. } => {
            push_optional(&mut pairs, "formula", metadata.formula);
            push_vec(&mut pairs, "source_tables", metadata.source_tables);
            push_vec(&mut pairs, "source_columns", metadata.source_columns);
            push_vec(&mut pairs, "synonyms", metadata.synonyms);
        }
        SemanticEntity::Knowledge { metadata, .. } => {
            push_optional(&mut pairs, "knowledge_type", metadata.knowledge_type);
            push_vec(&mut pairs, "scope_tables", metadata.scope_tables);
            push_vec(&mut pairs, "scope_columns", metadata.scope_columns);
            push_optional(&mut pairs, "sql_expression", metadata.sql_expression);
            push_vec(&mut pairs, "synonyms", metadata.synonyms);
            push_optional(
                &mut pairs,
                "source_knowledge_id",
                metadata.source_knowledge_id,
            );
            push_optional(
                &mut pairs,
                "source_document_id",
                metadata.source_document_id,
            );
        }
        SemanticEntity::Document { metadata, .. } => {
            pairs.push(("source_document_id", metadata.source_document_id));
            pairs.push(("content_available", metadata.content_available.to_string()));
            push_optional(&mut pairs, "content_source", metadata.content_source);
            push_optional(&mut pairs, "extraction_status", metadata.extraction_status);
            push_vec(
                &mut pairs,
                "extracted_knowledge_ids",
                metadata.extracted_knowledge_ids,
            );
        }
        SemanticEntity::DataQualityFinding { metadata, .. } => {
            pairs.push(("database_id", metadata.database_id));
            pairs.push(("schema_name", metadata.schema_name));
            pairs.push(("table_name", metadata.table_name));
            push_optional(&mut pairs, "column_name", metadata.column_name);
            push_optional(&mut pairs, "finding_type", metadata.finding_type);
            push_vec(&mut pairs, "scope_tables", metadata.scope_tables);
            push_vec(&mut pairs, "scope_columns", metadata.scope_columns);
            push_optional(
                &mut pairs,
                "typical_value_range",
                metadata.typical_value_range,
            );
            push_vec(&mut pairs, "validation_rules", metadata.validation_rules);
        }
        SemanticEntity::Special { .. } => {}
    }
    pairs
}

fn push_optional(
    pairs: &mut Vec<(&'static str, String)>,
    key: &'static str,
    value: Option<String>,
) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        pairs.push((key, value));
    }
}

fn push_vec(pairs: &mut Vec<(&'static str, String)>, key: &'static str, value: Vec<String>) {
    if !value.is_empty() {
        pairs.push((key, format!("[{}]", value.join(", "))));
    }
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let boundary = s
            .char_indices()
            .take_while(|(i, _)| *i < max.saturating_sub(3))
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        format!("{}...", &s[..boundary])
    }
}

fn truncate_if_needed(s: &str, max_length: Option<usize>) -> String {
    match max_length {
        Some(max) if s.len() > max => truncate_str(s, max),
        _ => s.to_string(),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::CatalogRelation;

    fn table_entry() -> CatalogEntry {
        CatalogEntry {
            id: "tbl-001".into(),
            kind: CatalogKind::Table,
            name: "users".into(),
            qualified_name: Some("public.users".into()),
            content: "User accounts table with login credentials".into(),
            tags: vec!["core".into(), "auth".into()],
            links: vec![CatalogRelation {
                target_id: "col-001".into(),
                kind: "has_column".into(),
                description: Some("Primary key".into()),
            }],
            metadata: serde_json::json!({
                "databaseId": "warehouse",
                "schemaName": "public",
                "tableName": "users",
                "relationType": "base_table",
                "rowCount": 15000,
                "columnCount": 1,
                "preferredQuerySurface": true,
                "source": {}
            }),
        }
    }

    fn column_entry() -> CatalogEntry {
        CatalogEntry {
            id: "col-001".into(),
            kind: CatalogKind::Column,
            name: "email".into(),
            qualified_name: Some("public.users.email".into()),
            content: "User email address, unique constraint".into(),
            tags: vec!["pii".into()],
            links: vec![],
            metadata: serde_json::json!({
                "databaseId": "warehouse",
                "schemaName": "public",
                "tableName": "users",
                "columnName": "email",
                "dataType": "varchar(255)",
                "nullable": false,
                "primaryKey": false,
                "foreignKey": null,
                "semanticType": null,
                "distinctCount": null,
                "nullCount": null,
                "totalCount": null,
                "lowCardinalityEnum": false
            }),
        }
    }

    fn metric_entry() -> CatalogEntry {
        CatalogEntry {
            id: "m-001".into(),
            kind: CatalogKind::Metric,
            name: "revenue".into(),
            qualified_name: None,
            content: "Total revenue across all orders".into(),
            tags: vec!["kpi".into()],
            links: vec![],
            metadata: serde_json::json!({
                "formula": "SUM(orders.amount)",
                "sourceTables": ["orders"],
                "sourceColumns": ["orders.amount"],
                "synonyms": []
            }),
        }
    }

    // ─── Polymorphic program ────────────────────────────────────────

    fn render_table_context<C: CatalogComposer>(c: &C) -> String {
        let ctx = ComposerContext::default();
        let entries = vec![table_entry(), column_entry()];
        c.compose_many(&entries, &ctx)
    }

    #[test]
    fn polymorphic_program_produces_output_for_both() {
        let md = render_table_context(&Markdown);
        assert!(md.contains("public.users"));
        assert!(md.contains("public.users.email"));

        let xml = render_table_context(&Xml);
        assert!(xml.contains("public.users"));
        assert!(xml.contains("public.users.email"));
    }

    // ─── L1 Determinism ────────────────────────────────────────────

    #[test]
    fn determinism_markdown() {
        let ctx = ComposerContext::default();
        let entry = table_entry();
        let a = Markdown.compose(&entry, &ctx);
        let b = Markdown.compose(&entry, &ctx);
        assert_eq!(a, b);
    }

    #[test]
    fn determinism_xml() {
        let ctx = ComposerContext::default();
        let entry = table_entry();
        let a = Xml.compose(&entry, &ctx);
        let b = Xml.compose(&entry, &ctx);
        assert_eq!(a, b);
    }

    // ─── L2 Non-empty ──────────────────────────────────────────────

    #[test]
    fn non_empty_all_variants_markdown() {
        let entry = table_entry();
        for variant in [
            ComposerVariant::Default,
            ComposerVariant::ToolUse,
            ComposerVariant::Verbose,
            ComposerVariant::Summary,
        ] {
            let ctx = ComposerContext::new(variant);
            let output = Markdown.compose(&entry, &ctx);
            assert!(!output.is_empty(), "Empty for variant {:?}", variant);
        }
    }

    #[test]
    fn non_empty_all_variants_xml() {
        let entry = table_entry();
        for variant in [
            ComposerVariant::Default,
            ComposerVariant::ToolUse,
            ComposerVariant::Verbose,
            ComposerVariant::Summary,
        ] {
            let ctx = ComposerContext::new(variant);
            let output = Xml.compose(&entry, &ctx);
            assert!(!output.is_empty(), "Empty for variant {:?}", variant);
        }
    }

    // ─── L3 Composition ────────────────────────────────────────────

    #[test]
    fn composition_law_markdown() {
        let ctx = ComposerContext::default();
        let entries = vec![table_entry(), column_entry()];
        let composed = Markdown.compose_many(&entries, &ctx);
        let manual = format!(
            "{}\n\n{}",
            Markdown.compose(&entries[0], &ctx),
            Markdown.compose(&entries[1], &ctx),
        );
        assert_eq!(composed, manual);
    }

    #[test]
    fn composition_law_xml() {
        let ctx = ComposerContext::default();
        let entries = vec![table_entry(), column_entry()];
        let composed = Xml.compose_many(&entries, &ctx);
        let manual = format!(
            "{}\n{}",
            Xml.compose(&entries[0], &ctx),
            Xml.compose(&entries[1], &ctx),
        );
        assert_eq!(composed, manual);
    }

    // ─── Markdown variant tests ────────────────────────────────────

    #[test]
    fn md_summary_table() {
        let ctx = ComposerContext::new(ComposerVariant::Summary);
        let output = Markdown.compose(&table_entry(), &ctx);
        assert!(output.starts_with("public.users:"));
    }

    #[test]
    fn md_tool_use_table_has_row_count() {
        let ctx = ComposerContext::new(ComposerVariant::ToolUse);
        let output = Markdown.compose(&table_entry(), &ctx);
        assert!(output.contains("15000 rows"));
    }

    #[test]
    fn md_tool_use_column_has_type() {
        let ctx = ComposerContext::new(ComposerVariant::ToolUse);
        let output = Markdown.compose(&column_entry(), &ctx);
        assert!(output.contains("varchar(255)"));
    }

    #[test]
    fn md_default_table_has_heading() {
        let ctx = ComposerContext::default();
        let output = Markdown.compose(&table_entry(), &ctx);
        assert!(output.contains("### public.users"));
    }

    #[test]
    fn md_default_has_tags() {
        let ctx = ComposerContext::default();
        let output = Markdown.compose(&table_entry(), &ctx);
        assert!(output.contains("`core`"));
        assert!(output.contains("`auth`"));
    }

    #[test]
    fn md_verbose_has_relations() {
        let ctx = ComposerContext::new(ComposerVariant::Verbose);
        let output = Markdown.compose(&table_entry(), &ctx);
        assert!(output.contains("**Relations:**"));
        assert!(output.contains("has_column"));
    }

    #[test]
    fn md_verbose_has_typed_metadata() {
        let ctx = ComposerContext::new(ComposerVariant::Verbose);
        let output = Markdown.compose(&table_entry(), &ctx);
        assert!(output.contains("**Metadata:**"));
        assert!(output.contains("- database_id: warehouse"));
        assert!(output.contains("- row_count: 15000"));
        assert!(!output.contains("databaseId"));
        assert!(!output.contains("rowCount"));
    }

    #[test]
    fn md_default_metric_has_formula() {
        let ctx = ComposerContext::default();
        let output = Markdown.compose(&metric_entry(), &ctx);
        assert!(output.contains("`SUM(orders.amount)`"));
    }

    // ─── XML variant tests ─────────────────────────────────────────

    #[test]
    fn xml_summary_table() {
        let ctx = ComposerContext::new(ComposerVariant::Summary);
        let output = Xml.compose(&table_entry(), &ctx);
        assert!(output.starts_with("<table name=\"public.users\">"));
    }

    #[test]
    fn xml_tool_use_table_has_rows_attr() {
        let ctx = ComposerContext::new(ComposerVariant::ToolUse);
        let output = Xml.compose(&table_entry(), &ctx);
        assert!(output.contains("rows=\"15000\""));
    }

    #[test]
    fn xml_tool_use_column_has_type_attr() {
        let ctx = ComposerContext::new(ComposerVariant::ToolUse);
        let output = Xml.compose(&column_entry(), &ctx);
        assert!(output.contains("type=\"varchar(255)\""));
    }

    #[test]
    fn xml_default_has_description_tag() {
        let ctx = ComposerContext::default();
        let output = Xml.compose(&table_entry(), &ctx);
        assert!(output.contains("<description>"));
    }

    #[test]
    fn xml_verbose_has_relations() {
        let ctx = ComposerContext::new(ComposerVariant::Verbose);
        let output = Xml.compose(&table_entry(), &ctx);
        assert!(output.contains("<relations>"));
        assert!(output.contains("has_column"));
    }

    #[test]
    fn xml_verbose_has_metadata() {
        let ctx = ComposerContext::new(ComposerVariant::Verbose);
        let output = Xml.compose(&table_entry(), &ctx);
        assert!(output.contains("<metadata>"));
        assert!(output.contains("<row_count>"));
    }

    #[test]
    fn composer_does_not_render_raw_metadata_when_semantic_decode_fails() {
        let entry = CatalogEntry {
            id: "col-raw".into(),
            kind: CatalogKind::Column,
            name: "raw_column".into(),
            qualified_name: Some("public.users.raw_column".into()),
            content: "Column with invalid legacy metadata".into(),
            tags: vec![],
            links: vec![],
            metadata: serde_json::json!({
                "data_type": "raw_only_secret_type",
                "nullable": true,
                "raw_only_secret": "must-not-render"
            }),
        };
        let ctx = ComposerContext::new(ComposerVariant::Verbose);

        let md = Markdown.compose(&entry, &ctx);
        assert!(!md.contains("raw_only_secret"));
        assert!(!md.contains("raw_only_secret_type"));
        assert!(!md.contains("**Metadata:**"));

        let xml = Xml.compose(&entry, &ctx);
        assert!(!xml.contains("raw_only_secret"));
        assert!(!xml.contains("raw_only_secret_type"));
        assert!(!xml.contains("<metadata>"));
    }

    // ─── Convenience function tests ────────────────────────────────

    #[test]
    fn compose_dispatches_by_family() {
        let ctx = ComposerContext::default();
        let entry = table_entry();
        let md = compose(&entry, ModelFamily::OpenAi, &ctx);
        let xml = compose(&entry, ModelFamily::Claude, &ctx);
        assert!(md.contains("###")); // Markdown heading
        assert!(xml.contains("<table")); // XML tag
    }

    // ─── ModelFamily detection ─────────────────────────────────────

    #[test]
    fn model_family_detection() {
        assert_eq!(ModelFamily::detect("claude-opus-4-6"), ModelFamily::Claude);
        assert_eq!(
            ModelFamily::detect("claude-sonnet-4-6"),
            ModelFamily::Claude
        );
        assert_eq!(
            ModelFamily::detect("claude-haiku-4-5-20251001"),
            ModelFamily::Claude
        );
        assert_eq!(ModelFamily::detect("gpt-4o"), ModelFamily::OpenAi);
        assert_eq!(ModelFamily::detect("cerebras/llama-4"), ModelFamily::OpenAi);
        assert_eq!(
            ModelFamily::detect("anthropic/claude-3"),
            ModelFamily::Claude
        );
    }

    // ─── Truncation ────────────────────────────────────────────────

    #[test]
    fn max_length_truncates() {
        let ctx = ComposerContext::new(ComposerVariant::Default).with_max_length(50);
        let output = Markdown.compose(&table_entry(), &ctx);
        assert!(output.len() <= 50);
        assert!(output.ends_with("..."));
    }

    // ─── XML escaping ──────────────────────────────────────────────

    #[test]
    fn xml_escapes_special_chars() {
        let entry = CatalogEntry {
            id: "tbl-xss".into(),
            kind: CatalogKind::Table,
            name: "t<a&b>".into(),
            qualified_name: None,
            content: "Uses <html> & \"quotes\"".into(),
            tags: vec![],
            links: vec![],
            metadata: serde_json::json!({}),
        };
        let ctx = ComposerContext::default();
        let output = Xml.compose(&entry, &ctx);
        assert!(!output.contains("<html>"));
        assert!(output.contains("&lt;html&gt;"));
        assert!(output.contains("&amp;"));
    }

    // ─── Empty entries ─────────────────────────────────────────────

    #[test]
    fn minimal_entry_renders() {
        let entry = CatalogEntry {
            id: "x".into(),
            kind: CatalogKind::Knowledge,
            name: "rule".into(),
            qualified_name: None,
            content: String::new(),
            tags: vec![],
            links: vec![],
            metadata: serde_json::json!(null),
        };
        let ctx = ComposerContext::default();
        let md = Markdown.compose(&entry, &ctx);
        assert!(!md.is_empty());
        let xml = Xml.compose(&entry, &ctx);
        assert!(!xml.is_empty());
    }
}
