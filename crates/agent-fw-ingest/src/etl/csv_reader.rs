//! CSV file reading for products and scenarios.
//!
//! Mirrors the parquet reader API surface: same `ProductRow` / `ScenarioRow`
//! output types, same detection and validation patterns.

use std::collections::BTreeMap;
use std::path::Path;

use super::parquet_reader::{ProductRow, ScenarioRow};
use super::EtlError;

/// Detected CSV file type (from header row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsvFileType {
    Products,
    Scenarios,
    Unknown,
}

/// Detect the type of a CSV file from its header row.
pub fn detect_file_type(path: &Path) -> Result<CsvFileType, EtlError> {
    let mut rdr = csv::Reader::from_path(path)?;
    let headers: Vec<String> = rdr.headers()?.iter().map(|h| h.to_string()).collect();
    let refs: Vec<&str> = headers.iter().map(|s| s.as_str()).collect();
    Ok(detect_file_type_from_columns(&refs))
}

/// Detect file type from a list of column names.
fn detect_file_type_from_columns(field_names: &[&str]) -> CsvFileType {
    let has_product_id = field_names.iter().any(|n| *n == "product_id");
    let has_scenario_name = field_names.iter().any(|n| *n == "scenario_name");
    let has_value = field_names.iter().any(|n| *n == "value");

    if has_scenario_name && has_value {
        CsvFileType::Scenarios
    } else if has_product_id {
        CsvFileType::Products
    } else {
        CsvFileType::Unknown
    }
}

/// Validate that headers contain all required columns.
fn validate_required_columns(
    headers: &[&str],
    required: &[&str],
    context: &str,
) -> Result<(), EtlError> {
    let missing: Vec<&str> = required
        .iter()
        .filter(|r| !headers.iter().any(|h| h == *r))
        .copied()
        .collect();
    if !missing.is_empty() {
        return Err(EtlError::Parsing(format!(
            "{context}: missing required column(s): {}",
            missing.join(", ")
        )));
    }
    Ok(())
}

/// Read product rows from a CSV file.
pub fn read_products(path: &Path) -> Result<Vec<ProductRow>, EtlError> {
    let mut rdr = csv::Reader::from_path(path)?;
    let headers: Vec<String> = rdr.headers()?.iter().map(|h| h.to_string()).collect();
    let header_refs: Vec<&str> = headers.iter().map(|s| s.as_str()).collect();
    validate_required_columns(&header_refs, &["product_id"], "read_products_csv")?;

    let known_fields = ["product_id", "segment", "subsegment", "brand", "sub_brand"];
    let mut products = Vec::new();

    for (row_idx, result) in rdr.records().enumerate() {
        let record = result
            .map_err(|e| EtlError::Parsing(format!("CSV parse error at row {row_idx}: {e}")))?;

        let get = |col: &str| -> Option<String> {
            let idx = headers.iter().position(|h| h == col)?;
            let val = record.get(idx)?.trim();
            if val.is_empty() {
                None
            } else {
                Some(val.to_string())
            }
        };

        let product_id = get("product_id").ok_or_else(|| {
            EtlError::Parsing(format!(
                "product_id is required but missing or empty at row {row_idx}"
            ))
        })?;
        let segment = get("segment").unwrap_or_default();
        let subsegment = get("subsegment").unwrap_or_default();
        let brand = get("brand").unwrap_or_default();
        let sub_brand = get("sub_brand").unwrap_or_default();

        let mut attributes = BTreeMap::new();
        for (i, header) in headers.iter().enumerate() {
            if !known_fields.contains(&header.as_str()) {
                if let Some(val) = record.get(i) {
                    let val = val.trim();
                    if !val.is_empty() {
                        attributes.insert(header.clone(), val.to_string());
                    }
                }
            }
        }

        products.push(ProductRow {
            product_id,
            segment,
            subsegment,
            brand,
            sub_brand,
            attributes,
        });
    }

    Ok(products)
}

/// Read scenario rows from a CSV file.
pub fn read_scenarios(path: &Path) -> Result<Vec<ScenarioRow>, EtlError> {
    let mut rdr = csv::Reader::from_path(path)?;
    let headers: Vec<String> = rdr.headers()?.iter().map(|h| h.to_string()).collect();
    let header_refs: Vec<&str> = headers.iter().map(|s| s.as_str()).collect();
    validate_required_columns(
        &header_refs,
        &["scenario_name", "value"],
        "read_scenarios_csv",
    )?;

    let mut scenarios = Vec::new();

    for (row_idx, result) in rdr.records().enumerate() {
        let record = result
            .map_err(|e| EtlError::Parsing(format!("CSV parse error at row {row_idx}: {e}")))?;

        let get = |col: &str| -> Option<String> {
            let idx = headers.iter().position(|h| h == col)?;
            let val = record.get(idx)?.trim();
            if val.is_empty() {
                None
            } else {
                Some(val.to_string())
            }
        };

        let scenario_name = get("scenario_name").ok_or_else(|| {
            EtlError::Parsing(format!(
                "scenario_name is required but missing or empty at row {row_idx}"
            ))
        })?;
        let product_id = get("product_id").unwrap_or_default();
        let channel = get("channel").unwrap_or_default();
        let period = get("period").unwrap_or_default();
        let value_str = get("value").ok_or_else(|| {
            EtlError::Parsing(format!(
                "value is required but missing or empty at row {row_idx}"
            ))
        })?;
        let value: f64 = value_str.parse().map_err(|e| {
            EtlError::Parsing(format!("value is not a valid number at row {row_idx}: {e}"))
        })?;

        scenarios.push(ScenarioRow {
            scenario_name,
            product_id,
            channel,
            period,
            value,
        });
    }

    Ok(scenarios)
}

// Convert csv::Error to EtlError via io::Error
impl From<csv::Error> for EtlError {
    fn from(e: csv::Error) -> Self {
        EtlError::Parsing(format!("CSV error: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_csv(content: &str) -> NamedTempFile {
        let mut tmp = NamedTempFile::new().expect("create temp file");
        tmp.write_all(content.as_bytes())
            .expect("write CSV content");
        tmp.flush().expect("flush");
        tmp
    }

    // -------------------------------------------------------------------
    // detect_file_type tests
    // -------------------------------------------------------------------

    #[test]
    fn detect_file_type_products() {
        let cols = &["product_id", "segment", "brand"];
        assert_eq!(detect_file_type_from_columns(cols), CsvFileType::Products);
    }

    #[test]
    fn detect_file_type_scenarios() {
        let cols = &["scenario_name", "value", "channel"];
        assert_eq!(detect_file_type_from_columns(cols), CsvFileType::Scenarios);
    }

    #[test]
    fn detect_file_type_unknown() {
        let cols = &["foo", "bar"];
        assert_eq!(detect_file_type_from_columns(cols), CsvFileType::Unknown);
    }

    #[test]
    fn detect_file_type_scenarios_wins_when_both() {
        let cols = &["product_id", "scenario_name", "value"];
        assert_eq!(detect_file_type_from_columns(cols), CsvFileType::Scenarios);
    }

    #[test]
    fn detect_file_type_from_csv_file() {
        let tmp = write_csv("product_id,segment\nP001,Premium\n");
        let result = detect_file_type(tmp.path()).unwrap();
        assert_eq!(result, CsvFileType::Products);
    }

    // -------------------------------------------------------------------
    // read_products tests
    // -------------------------------------------------------------------

    #[test]
    fn read_products_valid() {
        let csv = "product_id,segment,subsegment,brand,sub_brand,color\n\
                   P001,Premium,Sub1,BrandA,SubA,Red\n\
                   P002,Economy,Sub2,BrandB,SubB,Blue\n";
        let tmp = write_csv(csv);
        let products = read_products(tmp.path()).unwrap();
        assert_eq!(products.len(), 2);
        assert_eq!(products[0].product_id, "P001");
        assert_eq!(products[0].segment, "Premium");
        assert_eq!(products[0].brand, "BrandA");
        assert_eq!(
            products[0].attributes.get("color"),
            Some(&"Red".to_string())
        );
        assert_eq!(products[1].product_id, "P002");
    }

    #[test]
    fn read_products_missing_required_column() {
        let csv = "segment,brand\nPremium,BrandA\n";
        let tmp = write_csv(csv);
        let err = read_products(tmp.path()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("product_id"),
            "error should mention product_id: {msg}"
        );
    }

    #[test]
    fn read_products_empty_required_field() {
        let csv = "product_id,segment\n,Premium\n";
        let tmp = write_csv(csv);
        let err = read_products(tmp.path()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("product_id"),
            "error should mention product_id: {msg}"
        );
    }

    #[test]
    fn read_products_optional_fields_default() {
        let csv = "product_id\nP001\n";
        let tmp = write_csv(csv);
        let products = read_products(tmp.path()).unwrap();
        assert_eq!(products.len(), 1);
        assert_eq!(products[0].product_id, "P001");
        assert_eq!(products[0].segment, "");
        assert_eq!(products[0].brand, "");
    }

    // -------------------------------------------------------------------
    // read_scenarios tests
    // -------------------------------------------------------------------

    #[test]
    fn read_scenarios_valid() {
        let csv = "scenario_name,product_id,channel,period,value\n\
                   Base,P001,Online,2024-Q1,100.5\n\
                   High,P002,Retail,2024-Q2,200.75\n";
        let tmp = write_csv(csv);
        let scenarios = read_scenarios(tmp.path()).unwrap();
        assert_eq!(scenarios.len(), 2);
        assert_eq!(scenarios[0].scenario_name, "Base");
        assert_eq!(scenarios[0].value, 100.5);
        assert_eq!(scenarios[1].scenario_name, "High");
        assert_eq!(scenarios[1].channel, "Retail");
    }

    #[test]
    fn read_scenarios_missing_value_column() {
        let csv = "scenario_name,channel\nBase,Online\n";
        let tmp = write_csv(csv);
        let err = read_scenarios(tmp.path()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("value"), "error should mention value: {msg}");
    }

    #[test]
    fn read_scenarios_empty_required_field() {
        let csv = "scenario_name,value\n,42.0\n";
        let tmp = write_csv(csv);
        let err = read_scenarios(tmp.path()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("scenario_name"),
            "error should mention scenario_name: {msg}"
        );
    }

    #[test]
    fn read_scenarios_invalid_value() {
        let csv = "scenario_name,value\nBase,not_a_number\n";
        let tmp = write_csv(csv);
        let err = read_scenarios(tmp.path()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("not a valid number"),
            "error should mention parse failure: {msg}"
        );
    }

    #[test]
    fn read_scenarios_empty_value() {
        let csv = "scenario_name,value\nBase,\n";
        let tmp = write_csv(csv);
        let err = read_scenarios(tmp.path()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("value"), "error should mention value: {msg}");
    }
}
