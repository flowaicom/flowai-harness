//! Star schema DDL generation for ETL target databases.

use super::aggregation_parser::ProductSchema;

/// Generate DDL for the star schema.
///
/// Creates 9 core tables:
/// - dim_segments, dim_subsegments, dim_brands, dim_sub_brands
/// - dim_channels, dim_time_periods
/// - dim_products, dim_coordinates
/// - fact_scenario
///
/// Dynamic product attribute columns are added to dim_products.
pub fn create_star_schema_ddl(schema: &ProductSchema) -> Vec<String> {
    let mut statements = Vec::new();

    // Dimension tables
    statements.push(
        "CREATE TABLE IF NOT EXISTS dim_segments (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        code TEXT NOT NULL UNIQUE,
        name TEXT
    )"
        .to_string(),
    );

    statements.push(
        "CREATE TABLE IF NOT EXISTS dim_subsegments (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        code TEXT NOT NULL UNIQUE,
        name TEXT,
        segment_id INTEGER REFERENCES dim_segments(id)
    )"
        .to_string(),
    );

    statements.push(
        "CREATE TABLE IF NOT EXISTS dim_brands (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        code TEXT NOT NULL UNIQUE,
        name TEXT
    )"
        .to_string(),
    );

    statements.push(
        "CREATE TABLE IF NOT EXISTS dim_sub_brands (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        code TEXT NOT NULL UNIQUE,
        name TEXT,
        brand_id INTEGER REFERENCES dim_brands(id)
    )"
        .to_string(),
    );

    statements.push(
        "CREATE TABLE IF NOT EXISTS dim_channels (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        code TEXT NOT NULL UNIQUE,
        name TEXT
    )"
        .to_string(),
    );

    statements.push(
        "CREATE TABLE IF NOT EXISTS dim_time_periods (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        code TEXT NOT NULL UNIQUE,
        name TEXT,
        start_date TEXT,
        end_date TEXT
    )"
        .to_string(),
    );

    // dim_products with dynamic attribute columns
    let mut product_cols = vec![
        "id INTEGER PRIMARY KEY AUTOINCREMENT".to_string(),
        "product_code TEXT NOT NULL UNIQUE".to_string(),
        "segment_id INTEGER REFERENCES dim_segments(id)".to_string(),
        "subsegment_id INTEGER REFERENCES dim_subsegments(id)".to_string(),
        "brand_id INTEGER REFERENCES dim_brands(id)".to_string(),
        "sub_brand_id INTEGER REFERENCES dim_sub_brands(id)".to_string(),
    ];

    for col in &schema.columns {
        product_cols.push(format!(
            "\"{}\" {}",
            col.sanitized_name,
            col.sql_type.as_sql()
        ));
    }

    statements.push(format!(
        "CREATE TABLE IF NOT EXISTS dim_products (\n    {}\n)",
        product_cols.join(",\n    ")
    ));

    statements.push(
        "CREATE TABLE IF NOT EXISTS dim_coordinates (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        product_id INTEGER NOT NULL REFERENCES dim_products(id),
        channel_id INTEGER NOT NULL REFERENCES dim_channels(id),
        UNIQUE(product_id, channel_id)
    )"
        .to_string(),
    );

    // Fact table
    statements.push(
        "CREATE TABLE IF NOT EXISTS fact_scenario (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        scenario_name TEXT NOT NULL,
        coordinate_id INTEGER NOT NULL REFERENCES dim_coordinates(id),
        period_id INTEGER NOT NULL REFERENCES dim_time_periods(id),
        value REAL NOT NULL,
        UNIQUE(scenario_name, coordinate_id, period_id)
    )"
        .to_string(),
    );

    // Indexes
    statements.push(
        "CREATE INDEX IF NOT EXISTS idx_fact_scenario_coord ON fact_scenario(coordinate_id)"
            .to_string(),
    );
    statements.push(
        "CREATE INDEX IF NOT EXISTS idx_fact_scenario_period ON fact_scenario(period_id)"
            .to_string(),
    );
    statements.push(
        "CREATE INDEX IF NOT EXISTS idx_fact_scenario_name ON fact_scenario(scenario_name)"
            .to_string(),
    );

    statements
}

/// Generate a denormalized view joining all dimension tables with the fact table.
pub fn create_denormalized_view_ddl() -> String {
    "CREATE VIEW IF NOT EXISTS v_scenario_denormalized AS
    SELECT
        f.scenario_name,
        f.value,
        p.product_code,
        s.code AS segment,
        ss.code AS subsegment,
        b.code AS brand,
        sb.code AS sub_brand,
        ch.code AS channel,
        tp.code AS period
    FROM fact_scenario f
    JOIN dim_coordinates c ON f.coordinate_id = c.id
    JOIN dim_products p ON c.product_id = p.id
    JOIN dim_channels ch ON c.channel_id = ch.id
    JOIN dim_time_periods tp ON f.period_id = tp.id
    LEFT JOIN dim_segments s ON p.segment_id = s.id
    LEFT JOIN dim_subsegments ss ON p.subsegment_id = ss.id
    LEFT JOIN dim_brands b ON p.brand_id = b.id
    LEFT JOIN dim_sub_brands sb ON p.sub_brand_id = sb.id"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::super::aggregation_parser::*;
    use super::*;

    #[test]
    fn creates_core_tables() {
        let schema = ProductSchema {
            columns: vec![],
            total_products: 0,
        };
        let ddl = create_star_schema_ddl(&schema);
        assert!(ddl.len() >= 9); // 9 tables + indexes
        assert!(ddl.iter().any(|s| s.contains("dim_segments")));
        assert!(ddl.iter().any(|s| s.contains("fact_scenario")));
    }

    #[test]
    fn adds_dynamic_columns() {
        let schema = ProductSchema {
            columns: vec![DiscoveredColumn {
                original_name: "flavor".into(),
                sanitized_name: SanitizedName::new("flavor"),
                sql_type: SqlColumnType::Text,
                display_name: "Flavor".into(),
                sample_values: vec!["vanilla".into()],
                distinct_count: 3,
            }],
            total_products: 10,
        };
        let ddl = create_star_schema_ddl(&schema);
        let products_ddl = ddl.iter().find(|s| s.contains("dim_products")).unwrap();
        assert!(products_ddl.contains("\"flavor\" TEXT"));
    }

    #[test]
    fn denormalized_view() {
        let view = create_denormalized_view_ddl();
        assert!(view.contains("v_scenario_denormalized"));
        assert!(view.contains("JOIN dim_coordinates"));
    }
}
