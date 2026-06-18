use agent_fw_catalog::{CatalogEntry, CatalogKind, CatalogRelation, CatalogScope, CatalogWriter};
use agent_fw_interpreter::{ScopedSqliteCatalog, SqliteCatalog};
use serde_json::json;

const DATABASE_ID: &str = "fixture_business";
const SCHEMA: &str = "public";

pub fn entries() -> Vec<CatalogEntry> {
    let mut entries = Vec::new();

    let table_specs = [
        (
            "fact_sales",
            "Sales transaction facts with product, location, promotion, status, and net amount.",
            false,
            vec![
                "sale_id",
                "product_id",
                "location_id",
                "promotion_id",
                "order_status",
                "net_amount",
            ],
            vec![
                relation(table_id("dim_products"), "references_table", "product_id -> product_id"),
                relation(
                    table_id("dim_locations"),
                    "references_table",
                    "location_id -> location_id",
                ),
                relation(
                    table_id("dim_promotions"),
                    "references_table",
                    "promotion_id -> promotion_id",
                ),
            ],
        ),
        (
            "dim_products",
            "Product dimension with brand, category, product line, and packaging attributes.",
            false,
            vec![
                "product_id",
                "brand_id",
                "category_id",
                "product_line",
                "package_material",
            ],
            vec![
                relation(table_id("dim_brands"), "references_table", "brand_id -> brand_id"),
                relation(
                    table_id("dim_categories"),
                    "references_table",
                    "category_id -> category_id",
                ),
            ],
        ),
        (
            "dim_brands",
            "Brand dimension for product reporting.",
            false,
            vec!["brand_id", "brand_name"],
            vec![],
        ),
        (
            "dim_categories",
            "Category dimension for product hierarchy reporting.",
            false,
            vec!["category_id", "category_name"],
            vec![],
        ),
        (
            "dim_locations",
            "Location dimension with the sales channel used for merchant reporting.",
            false,
            vec!["location_id", "channel_id"],
            vec![relation(
                table_id("dim_sales_channels"),
                "references_table",
                "channel_id -> channel_id",
            )],
        ),
        (
            "dim_sales_channels",
            "Sales channel dimension. Merchant, distributor, retail partner, and online channel analysis.",
            false,
            vec!["channel_id", "channel_name"],
            vec![],
        ),
        (
            "dim_promotions",
            "Promotion dimension for campaign and discount reporting.",
            false,
            vec!["promotion_id", "promotion_name"],
            vec![],
        ),
        (
            "v_sales_enriched",
            "Preferred sales reporting view for merchant, product line, fiscal quarter, status, and net amount analysis.",
            true,
            vec![
                "product_line_ref",
                "channel_name",
                "fiscal_quarter",
                "net_amount",
                "order_status",
            ],
            vec![],
        ),
    ];

    for (table, content, preferred_query_surface, columns, extra_links) in table_specs {
        entries.push(table_entry(
            table,
            content,
            preferred_query_surface,
            &columns,
            extra_links,
        ));
    }

    entries.extend([
        column_entry("fact_sales", "sale_id", "uuid", false, true, None, false),
        column_entry(
            "fact_sales",
            "product_id",
            "uuid",
            false,
            false,
            Some(("dim_products", "product_id")),
            false,
        ),
        column_entry(
            "fact_sales",
            "location_id",
            "uuid",
            false,
            false,
            Some(("dim_locations", "location_id")),
            false,
        ),
        column_entry(
            "fact_sales",
            "promotion_id",
            "uuid",
            true,
            false,
            Some(("dim_promotions", "promotion_id")),
            false,
        ),
        column_entry(
            "fact_sales",
            "order_status",
            "text",
            false,
            false,
            None,
            true,
        ),
        column_entry(
            "fact_sales",
            "net_amount",
            "numeric",
            false,
            false,
            None,
            false,
        ),
        column_entry(
            "dim_products",
            "product_id",
            "uuid",
            false,
            true,
            None,
            false,
        ),
        column_entry(
            "dim_products",
            "brand_id",
            "uuid",
            false,
            false,
            Some(("dim_brands", "brand_id")),
            false,
        ),
        column_entry(
            "dim_products",
            "category_id",
            "uuid",
            false,
            false,
            Some(("dim_categories", "category_id")),
            false,
        ),
        column_entry(
            "dim_products",
            "product_line",
            "text",
            false,
            false,
            None,
            true,
        ),
        column_entry(
            "dim_products",
            "package_material",
            "text",
            false,
            false,
            None,
            true,
        ),
        column_entry("dim_brands", "brand_id", "uuid", false, true, None, false),
        column_entry(
            "dim_brands",
            "brand_name",
            "text",
            false,
            false,
            None,
            false,
        ),
        column_entry(
            "dim_categories",
            "category_id",
            "uuid",
            false,
            true,
            None,
            false,
        ),
        column_entry(
            "dim_categories",
            "category_name",
            "text",
            false,
            false,
            None,
            true,
        ),
        column_entry(
            "dim_locations",
            "location_id",
            "uuid",
            false,
            true,
            None,
            false,
        ),
        column_entry(
            "dim_locations",
            "channel_id",
            "uuid",
            false,
            false,
            Some(("dim_sales_channels", "channel_id")),
            false,
        ),
        column_entry(
            "dim_sales_channels",
            "channel_id",
            "uuid",
            false,
            true,
            None,
            false,
        ),
        column_entry(
            "dim_sales_channels",
            "channel_name",
            "text",
            false,
            false,
            None,
            true,
        ),
        column_entry(
            "dim_promotions",
            "promotion_id",
            "uuid",
            false,
            true,
            None,
            false,
        ),
        column_entry(
            "dim_promotions",
            "promotion_name",
            "text",
            false,
            false,
            None,
            false,
        ),
        column_entry(
            "v_sales_enriched",
            "product_line_ref",
            "text",
            false,
            false,
            None,
            true,
        ),
        column_entry(
            "v_sales_enriched",
            "channel_name",
            "text",
            false,
            false,
            None,
            false,
        ),
        column_entry(
            "v_sales_enriched",
            "fiscal_quarter",
            "text",
            false,
            false,
            None,
            false,
        ),
        column_entry(
            "v_sales_enriched",
            "net_amount",
            "numeric",
            false,
            false,
            None,
            false,
        ),
        column_entry(
            "v_sales_enriched",
            "order_status",
            "text",
            false,
            false,
            None,
            true,
        ),
    ]);

    entries.extend(enum_values(
        "fact_sales",
        "order_status",
        &[
            ("quoted", &["quote", "pending quote"][..]),
            ("confirmed", &["booked", "accepted"][..]),
            ("shipped", &["fulfilled", "dispatched"][..]),
            ("returned", &["refund", "return"][..]),
        ],
    ));
    entries.extend(enum_values(
        "dim_products",
        "product_line",
        &[
            (
                "sparkling_water",
                &["sparkling water", "carbonated water"][..],
            ),
            ("still_water", &["still water", "flat water"][..]),
            ("herbal_tea", &["herbal tea", "infusion"][..]),
        ],
    ));
    entries.extend(enum_values(
        "dim_products",
        "package_material",
        &[
            ("glass", &["glass bottle"][..]),
            ("aluminum", &["can", "aluminium"][..]),
            ("recycled_pet", &["recycled plastic", "rpet"][..]),
        ],
    ));
    entries.extend(enum_values(
        "dim_categories",
        "category_name",
        &[
            ("Beverage", &["drink", "beverages"][..]),
            ("Snack", &["snacks"][..]),
            ("Household", &["home goods"][..]),
        ],
    ));
    entries.extend(enum_values(
        "dim_sales_channels",
        "channel_name",
        &[
            ("Online", &["web", "direct digital"][..]),
            (
                "Retail Partner",
                &["merchant", "retailer", "store partner"][..],
            ),
            ("Distributor", &["wholesale", "merchant distributor"][..]),
        ],
    ));
    entries.extend(enum_values(
        "v_sales_enriched",
        "product_line_ref",
        &[(
            "sparkling_water",
            &["sparkling water", "carbonated water"][..],
        )],
    ));

    entries.push(relationship_entry(
        "fact_sales",
        "dim_products",
        "product_id",
        "product_id",
    ));
    entries.push(relationship_entry(
        "fact_sales",
        "dim_locations",
        "location_id",
        "location_id",
    ));
    entries.push(relationship_entry(
        "fact_sales",
        "dim_promotions",
        "promotion_id",
        "promotion_id",
    ));
    entries.push(relationship_entry(
        "dim_products",
        "dim_brands",
        "brand_id",
        "brand_id",
    ));
    entries.push(relationship_entry(
        "dim_products",
        "dim_categories",
        "category_id",
        "category_id",
    ));
    entries.push(relationship_entry(
        "dim_locations",
        "dim_sales_channels",
        "channel_id",
        "channel_id",
    ));

    entries
}

#[allow(dead_code)]
pub async fn sqlite_catalog() -> ScopedSqliteCatalog {
    let catalog = SqliteCatalog::in_memory()
        .unwrap()
        .with_scope(CatalogScope::legacy_unscoped());
    catalog.save_in_transaction(entries()).await.unwrap();
    catalog
}

#[allow(dead_code)]
pub async fn sqlite_catalog_with_relationship_vertices_only() -> ScopedSqliteCatalog {
    let catalog = SqliteCatalog::in_memory()
        .unwrap()
        .with_scope(CatalogScope::legacy_unscoped());
    catalog
        .save_in_transaction(relationship_vertices_only_entries())
        .await
        .unwrap();
    catalog
}

#[allow(dead_code)]
pub fn relationship_vertices_only_entries() -> Vec<CatalogEntry> {
    entries()
        .into_iter()
        .map(|mut entry| {
            if entry.kind == CatalogKind::Table {
                entry.links.retain(|link| {
                    link.kind != "references_table" && link.kind != "referenced_by_table"
                });
            }
            entry
        })
        .collect()
}

#[allow(dead_code)]
pub fn fact_sales_products_relationship(entries: &[CatalogEntry]) -> CatalogEntry {
    entries
        .iter()
        .find(|entry| {
            entry.kind == CatalogKind::Relationship
                && entry
                    .metadata
                    .get("sourceTable")
                    .and_then(|value| value.as_str())
                    == Some("fact_sales")
                && entry
                    .metadata
                    .get("targetTable")
                    .and_then(|value| value.as_str())
                    == Some("dim_products")
        })
        .cloned()
        .expect("fact_sales -> dim_products relationship")
}

#[allow(dead_code)]
pub fn retarget_relationship(
    mut relationship: CatalogEntry,
    target_table: &str,
    target_column: &str,
) -> CatalogEntry {
    relationship.name = format!("fact_sales_to_{target_table}");
    relationship.content =
        format!("fact_sales.{target_column} references {target_table}.{target_column}");
    for link in &mut relationship.links {
        if link.kind == "relationship_target_table" {
            link.target_id = table_id(target_table);
        }
    }
    relationship.metadata["targetTableId"] = json!(table_id(target_table));
    relationship.metadata["targetTable"] = json!(target_table);
    relationship.metadata["targetColumn"] = json!(target_column);
    relationship.metadata["toColumn"] = json!(target_column);
    relationship
}

/// Builds a catalog where the only `references_table` / `referenced_by_table`
/// edges between `fact_sales` and `dim_products` carry a deliberately
/// misleading textual description, while the relationship vertex still carries
/// the correct typed [`RelationshipMetadata`]. Used to prove that the join
/// path toolkit reads typed metadata in preference to the legacy description
/// string parsing path.
#[allow(dead_code)]
pub async fn sqlite_catalog_with_misleading_join_link_descriptions() -> ScopedSqliteCatalog {
    let catalog = SqliteCatalog::in_memory()
        .unwrap()
        .with_scope(CatalogScope::legacy_unscoped());
    catalog
        .save_in_transaction(misleading_join_link_entries())
        .await
        .unwrap();
    catalog
}

/// Distinct join column names so reverse-direction tests can actually detect
/// a swap of the from/to flip in `extract_join_info`. If both were the same
/// string, the reverse test would tautologically pass even when the
/// directionality logic was inverted.
pub const MISLEADING_FIXTURE_SOURCE_COLUMN: &str = "product_sku";
pub const MISLEADING_FIXTURE_TARGET_COLUMN: &str = "id";

/// Builds a catalog with misleading join-column descriptions on direct table
/// edges but **no relationship vertex**. Used to verify that join-path
/// extraction falls back to explicit naming-convention inference rather than
/// parsing relation description strings.
#[allow(dead_code)]
pub async fn sqlite_catalog_with_description_only_table_edges() -> ScopedSqliteCatalog {
    let catalog = SqliteCatalog::in_memory()
        .unwrap()
        .with_scope(CatalogScope::legacy_unscoped());
    catalog
        .save_in_transaction(description_only_table_edge_entries())
        .await
        .unwrap();
    catalog
}

#[allow(dead_code)]
pub fn description_only_table_edge_entries() -> Vec<CatalogEntry> {
    let fact_sales = table_entry(
        "fact_sales",
        "Sales facts referencing dim_products via product_sku.",
        false,
        &["sale_id", "product_sku"],
        vec![relation(
            table_id("dim_products"),
            "references_table",
            "wrong_from -> wrong_to",
        )],
    );
    let dim_products = table_entry(
        "dim_products",
        "Product dimension.",
        false,
        &["id"],
        vec![relation(
            table_id("fact_sales"),
            "referenced_by_table",
            "wrong_reverse_from -> wrong_reverse_to",
        )],
    );
    let fact_sale_product_sku = column_entry(
        "fact_sales",
        "product_sku",
        "uuid",
        false,
        false,
        Some(("dim_products", "id")),
        false,
    );
    let dim_product_pk = column_entry("dim_products", "id", "uuid", false, true, None, false);
    vec![
        fact_sales,
        dim_products,
        fact_sale_product_sku,
        dim_product_pk,
    ]
}

#[allow(dead_code)]
pub fn misleading_join_link_entries() -> Vec<CatalogEntry> {
    let fact_sales = table_entry(
        "fact_sales",
        "Sales facts referencing dim_products via product_sku.",
        false,
        &["sale_id", MISLEADING_FIXTURE_SOURCE_COLUMN],
        vec![relation(
            table_id("dim_products"),
            "references_table",
            "misleading_from -> misleading_to",
        )],
    );
    let dim_products = table_entry(
        "dim_products",
        "Product dimension.",
        false,
        &[MISLEADING_FIXTURE_TARGET_COLUMN],
        vec![relation(
            table_id("fact_sales"),
            "referenced_by_table",
            "misleading_rev_from -> misleading_rev_to",
        )],
    );
    let fact_sale_product_sku = column_entry(
        "fact_sales",
        MISLEADING_FIXTURE_SOURCE_COLUMN,
        "uuid",
        false,
        false,
        Some(("dim_products", MISLEADING_FIXTURE_TARGET_COLUMN)),
        false,
    );
    let dim_product_pk = column_entry(
        "dim_products",
        MISLEADING_FIXTURE_TARGET_COLUMN,
        "uuid",
        false,
        true,
        None,
        false,
    );
    let relationship = relationship_entry(
        "fact_sales",
        "dim_products",
        MISLEADING_FIXTURE_SOURCE_COLUMN,
        MISLEADING_FIXTURE_TARGET_COLUMN,
    );
    vec![
        fact_sales,
        dim_products,
        fact_sale_product_sku,
        dim_product_pk,
        relationship,
    ]
}

fn table_entry(
    table: &str,
    content: &str,
    preferred_query_surface: bool,
    columns: &[&str],
    extra_links: Vec<CatalogRelation>,
) -> CatalogEntry {
    let mut links: Vec<CatalogRelation> = columns
        .iter()
        .map(|column| relation(column_id(table, column), "has_column", "table column"))
        .collect();
    links.extend(extra_links);

    CatalogEntry {
        id: table_id(table),
        kind: CatalogKind::Table,
        name: table.to_string(),
        qualified_name: Some(qualified_table(table)),
        content: content.to_string(),
        tags: vec!["business_fixture".to_string(), "sales".to_string()],
        links,
        metadata: json!({
            "databaseId": DATABASE_ID,
            "schemaName": SCHEMA,
            "tableName": table,
            "relationType": if preferred_query_surface { "view" } else { "table" },
            "rowCount": if preferred_query_surface { 5000 } else { 1000 },
            "columnCount": columns.len(),
            "preferredQuerySurface": preferred_query_surface,
            "tableType": if preferred_query_surface { "view" } else { "table" },
            "source": { "system": "conformance_fixture" },
        }),
    }
}

fn column_entry(
    table: &str,
    column: &str,
    data_type: &str,
    nullable: bool,
    primary_key: bool,
    foreign_key: Option<(&str, &str)>,
    categorical: bool,
) -> CatalogEntry {
    let mut links = vec![relation(table_id(table), "belongs_to", "parent table")];
    if let Some((target_table, _target_column)) = foreign_key {
        links.push(relation(
            table_id(target_table),
            "references",
            "foreign key target table",
        ));
    }

    CatalogEntry {
        id: column_id(table, column),
        kind: CatalogKind::Column,
        name: column.to_string(),
        qualified_name: Some(qualified_column(table, column)),
        content: column_description(table, column),
        tags: column_tags(table, column, categorical),
        links,
        metadata: json!({
            "databaseId": DATABASE_ID,
            "schemaName": SCHEMA,
            "tableName": table,
            "qualifiedTableName": qualified_table(table),
            "columnName": column,
            "dataType": data_type,
            "nullable": nullable,
            "isPrimaryKey": primary_key,
            "primaryKey": primary_key,
            "isForeignKey": foreign_key.is_some(),
            "foreignKey": foreign_key.map(|(target_table, target_column)| json!({
                "targetTableId": table_id(target_table),
                "targetSchema": SCHEMA,
                "targetTable": target_table,
                "targetColumn": target_column,
            })),
            "isCategorical": categorical,
            "lowCardinalityEnum": categorical,
        }),
    }
}

fn enum_values(table: &str, column: &str, values: &[(&str, &[&str])]) -> Vec<CatalogEntry> {
    values
        .iter()
        .enumerate()
        .map(|(idx, (value, synonyms))| enum_entry(table, column, value, synonyms, idx + 1))
        .collect()
}

fn enum_entry(
    table: &str,
    column: &str,
    value: &str,
    synonyms: &[&str],
    rank: usize,
) -> CatalogEntry {
    let display_value = value.replace('_', " ");
    CatalogEntry {
        id: enum_id(table, column, value),
        kind: CatalogKind::Enum,
        name: value.to_string(),
        qualified_name: Some(format!("{}.{}", qualified_column(table, column), value)),
        content: format!(
            "{} value for {}.{}; synonyms: {}",
            display_value,
            table,
            column,
            synonyms.join(", ")
        ),
        tags: vec!["enum_value".to_string()],
        links: vec![relation(
            column_id(table, column),
            "enum_value_of",
            "enum value column",
        )],
        metadata: json!({
            "databaseId": DATABASE_ID,
            "schemaName": SCHEMA,
            "tableName": table,
            "qualifiedTableName": qualified_table(table),
            "columnName": column,
            "columnId": column_id(table, column),
            "value": value,
            "normalizedValue": value.to_lowercase(),
            "displayValue": display_value,
            "rank": rank,
            "synonyms": synonyms,
        }),
    }
}

fn relationship_entry(
    source_table: &str,
    target_table: &str,
    source_column: &str,
    target_column: &str,
) -> CatalogEntry {
    let id = format!(
        "relationship:{}.{source_column}->{}.{target_column}",
        qualified_table(source_table),
        qualified_table(target_table)
    );
    CatalogEntry {
        id,
        kind: CatalogKind::Relationship,
        name: format!("{source_table}_to_{target_table}"),
        qualified_name: None,
        content: format!(
            "{source_table}.{source_column} references {target_table}.{target_column}"
        ),
        tags: vec!["relationship".to_string(), "foreign_key".to_string()],
        links: vec![
            relation(
                table_id(source_table),
                "relationship_source_table",
                "relationship source table",
            ),
            relation(
                table_id(target_table),
                "relationship_target_table",
                "relationship target table",
            ),
        ],
        metadata: json!({
            "databaseId": DATABASE_ID,
            "sourceTableId": table_id(source_table),
            "targetTableId": table_id(target_table),
            "sourceSchema": SCHEMA,
            "sourceTable": source_table,
            "sourceColumn": source_column,
            "targetSchema": SCHEMA,
            "targetTable": target_table,
            "targetColumn": target_column,
            "sourceCardinality": "many",
            "targetCardinality": "one",
            "relationshipKind": "foreign_key",
            "confidence": 1.0,
            "fromColumn": source_column,
            "toColumn": target_column,
        }),
    }
}

fn relation(target_id: String, kind: &str, description: &str) -> CatalogRelation {
    CatalogRelation {
        target_id,
        kind: kind.to_string(),
        description: Some(description.to_string()),
    }
}

fn table_id(table: &str) -> String {
    format!("table:{}", qualified_table(table))
}

fn column_id(table: &str, column: &str) -> String {
    format!("column:{}", qualified_column(table, column))
}

fn enum_id(table: &str, column: &str, value: &str) -> String {
    format!("enum:{}.{}", qualified_column(table, column), value)
}

fn qualified_table(table: &str) -> String {
    format!("{SCHEMA}.{table}")
}

fn qualified_column(table: &str, column: &str) -> String {
    format!("{}.{}", qualified_table(table), column)
}

fn column_description(table: &str, column: &str) -> String {
    match (table, column) {
        ("dim_sales_channels", "channel_name") => {
            "Merchant channel name used to group online, retailer, and distributor sales."
                .to_string()
        }
        ("v_sales_enriched", "channel_name") => {
            "Merchant-facing channel name exposed by the preferred sales view.".to_string()
        }
        ("v_sales_enriched", "product_line_ref") => {
            "Product line reference such as sparkling water on the preferred sales view."
                .to_string()
        }
        _ => format!("{column} column on {SCHEMA}.{table}."),
    }
}

fn column_tags(table: &str, column: &str, categorical: bool) -> Vec<String> {
    let mut tags = vec!["business_fixture".to_string()];
    if categorical {
        tags.push("categorical".to_string());
    }
    if table == "v_sales_enriched" {
        tags.push("preferred_query_surface".to_string());
    }
    if column == "channel_name" {
        tags.push("merchant".to_string());
    }
    tags
}
