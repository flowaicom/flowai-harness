use tantivy::schema::{Field, Schema, FAST, STORED, STRING, TEXT};

#[derive(Debug, Clone)]
pub(crate) struct CatalogIndexSchema {
    pub schema: Schema,
    pub fields: CatalogIndexFields,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CatalogIndexFields {
    pub scope_tenant: Field,
    pub scope_workspace: Field,
    pub entry_id: Field,
    pub kind: Field,
    pub name_exact: Field,
    pub qualified_name_exact: Field,
    pub name_text: Field,
    pub qualified_name_text: Field,
    pub description_text: Field,
    pub tags: Field,
    pub tags_text: Field,
    pub database_id: Field,
    pub schema_name: Field,
    pub table_name: Field,
    pub column_name: Field,
    pub data_type: Field,
    pub semantic_type: Field,
    pub knowledge_type: Field,
    pub relation_kind: Field,
    pub source_table: Field,
    pub source_column: Field,
    pub target_table: Field,
    pub target_column: Field,
    pub preferred_query_surface: Field,
    pub low_cardinality_enum: Field,
    pub synonyms_text: Field,
    pub context_text: Field,
    pub document_body_text: Field,
    pub enum_value_exact: Field,
    pub enum_value_text: Field,
    pub enum_display_value_text: Field,
    pub projection_version: Field,
    pub updated_at: Field,
    pub relation_type: Field,
    pub nullable: Field,
    pub primary_key: Field,
    pub foreign_key_text: Field,
    pub cardinality: Field,
    pub confidence: Field,
    pub formula_text: Field,
    pub sql_expression_text: Field,
    pub validation_rules_text: Field,
}

pub(crate) fn build_catalog_schema() -> CatalogIndexSchema {
    let mut builder = Schema::builder();
    let fields = CatalogIndexFields {
        scope_tenant: builder.add_text_field("scope_tenant", STRING | STORED),
        scope_workspace: builder.add_text_field("scope_workspace", STRING | STORED),
        // `STRING | STORED` is required for the exact-term `entry_id` shortcut
        // query and for hydration via the stored value. `FAST` adds the
        // columnar string column the search/facet collectors read to break
        // equal-score ties on ascending `entry_id` (stable across rebuilds,
        // segments, and SQLite- vs Postgres-built indexes).
        entry_id: builder.add_text_field("entry_id", STRING | STORED | FAST),
        kind: builder.add_text_field("kind", STRING | STORED),
        name_exact: builder.add_text_field("name_exact", STRING | STORED),
        qualified_name_exact: builder.add_text_field("qualified_name_exact", STRING | STORED),
        name_text: builder.add_text_field("name_text", TEXT | STORED),
        qualified_name_text: builder.add_text_field("qualified_name_text", TEXT | STORED),
        description_text: builder.add_text_field("description_text", TEXT | STORED),
        tags: builder.add_text_field("tags", STRING | STORED),
        tags_text: builder.add_text_field("tags_text", TEXT | STORED),
        database_id: builder.add_text_field("database_id", STRING | STORED),
        schema_name: builder.add_text_field("schema_name", STRING | STORED),
        table_name: builder.add_text_field("table_name", STRING | STORED),
        column_name: builder.add_text_field("column_name", STRING | STORED),
        data_type: builder.add_text_field("data_type", STRING | STORED),
        semantic_type: builder.add_text_field("semantic_type", STRING | STORED),
        knowledge_type: builder.add_text_field("knowledge_type", STRING | STORED),
        relation_kind: builder.add_text_field("relation_kind", STRING | STORED),
        source_table: builder.add_text_field("source_table", STRING | STORED),
        source_column: builder.add_text_field("source_column", STRING | STORED),
        target_table: builder.add_text_field("target_table", STRING | STORED),
        target_column: builder.add_text_field("target_column", STRING | STORED),
        preferred_query_surface: builder.add_text_field("preferred_query_surface", STRING | STORED),
        low_cardinality_enum: builder.add_text_field("low_cardinality_enum", STRING | STORED),
        synonyms_text: builder.add_text_field("synonyms_text", TEXT | STORED),
        context_text: builder.add_text_field("context_text", TEXT | STORED),
        document_body_text: builder.add_text_field("document_body_text", TEXT | STORED),
        enum_value_exact: builder.add_text_field("enum_value_exact", STRING | STORED),
        enum_value_text: builder.add_text_field("enum_value_text", TEXT | STORED),
        enum_display_value_text: builder.add_text_field("enum_display_value_text", TEXT | STORED),
        projection_version: builder.add_text_field("projection_version", STRING | STORED),
        updated_at: builder.add_text_field("updated_at", STRING | STORED),
        relation_type: builder.add_text_field("relation_type", TEXT | STORED),
        nullable: builder.add_text_field("nullable", STRING | STORED),
        primary_key: builder.add_text_field("primary_key", STRING | STORED),
        foreign_key_text: builder.add_text_field("foreign_key_text", TEXT | STORED),
        cardinality: builder.add_text_field("cardinality", STRING | STORED),
        confidence: builder.add_text_field("confidence", STRING | STORED),
        formula_text: builder.add_text_field("formula_text", TEXT | STORED),
        sql_expression_text: builder.add_text_field("sql_expression_text", TEXT | STORED),
        validation_rules_text: builder.add_text_field("validation_rules_text", TEXT | STORED),
    };

    CatalogIndexSchema {
        schema: builder.build(),
        fields,
    }
}

impl CatalogIndexFields {
    pub fn text_search_fields(&self) -> Vec<Field> {
        vec![
            self.name_text,
            self.qualified_name_text,
            self.description_text,
            self.tags_text,
            self.synonyms_text,
            self.context_text,
            self.document_body_text,
            self.enum_value_text,
            self.enum_display_value_text,
            self.relation_type,
            self.foreign_key_text,
            self.formula_text,
            self.sql_expression_text,
            self.validation_rules_text,
        ]
    }
}
