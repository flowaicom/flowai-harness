//! Parquet file reading for products and scenarios.

use arrow::array::{Array, AsArray, RecordBatch};
use arrow::compute;
use arrow::datatypes::DataType;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;

use super::EtlError;

/// A product row extracted from parquet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductRow {
    pub product_id: String,
    pub segment: String,
    pub subsegment: String,
    pub brand: String,
    pub sub_brand: String,
    pub attributes: BTreeMap<String, String>,
}

/// A scenario row extracted from parquet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioRow {
    pub scenario_name: String,
    pub product_id: String,
    pub channel: String,
    pub period: String,
    pub value: f64,
}

/// Detected parquet file type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParquetFileType {
    Products,
    Scenarios,
    Unknown,
}

/// Detect the type of a parquet file from its column names.
pub fn detect_file_type(path: &Path) -> Result<ParquetFileType, EtlError> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let schema = builder.schema();
    let field_names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();

    detect_file_type_from_columns(&field_names)
}

/// Detect file type from a list of column names (extracted for testability).
fn detect_file_type_from_columns(field_names: &[&str]) -> Result<ParquetFileType, EtlError> {
    let has_product_id = field_names.iter().any(|n| *n == "product_id");
    let has_scenario_name = field_names.iter().any(|n| *n == "scenario_name");
    let has_value = field_names.iter().any(|n| *n == "value");

    // Scenarios require BOTH scenario_name AND value
    if has_scenario_name && has_value {
        Ok(ParquetFileType::Scenarios)
    // Products require product_id
    } else if has_product_id {
        Ok(ParquetFileType::Products)
    } else {
        Ok(ParquetFileType::Unknown)
    }
}

/// Validate that a schema contains all required columns, returning an error listing any missing ones.
fn validate_required_columns(
    field_names: &[&str],
    required: &[&str],
    context: &str,
) -> Result<(), EtlError> {
    let missing: Vec<&str> = required
        .iter()
        .filter(|r| !field_names.iter().any(|f| f == *r))
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

/// Read product rows from a parquet file.
pub fn read_products(path: &Path) -> Result<Vec<ProductRow>, EtlError> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let schema = builder.schema().clone();

    // Upfront column validation
    let field_names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    validate_required_columns(&field_names, &["product_id"], "read_products")?;

    let reader = builder.build()?;

    let known_fields = ["product_id", "segment", "subsegment", "brand", "sub_brand"];
    let mut products = Vec::new();

    for batch_result in reader {
        let batch: RecordBatch = batch_result?;
        let num_rows = batch.num_rows();

        for row_idx in 0..num_rows {
            let product_id = get_string_value(&batch, "product_id", row_idx).ok_or_else(|| {
                EtlError::Parsing(format!(
                    "product_id is required but missing or null at row {row_idx}"
                ))
            })?;
            let segment = get_string_value(&batch, "segment", row_idx).unwrap_or_default();
            let subsegment = get_string_value(&batch, "subsegment", row_idx).unwrap_or_default();
            let brand = get_string_value(&batch, "brand", row_idx).unwrap_or_default();
            let sub_brand = get_string_value(&batch, "sub_brand", row_idx).unwrap_or_default();

            // Collect extra attributes
            let mut attributes = BTreeMap::new();
            for field in schema.fields() {
                let name = field.name().as_str();
                if !known_fields.contains(&name) {
                    if let Some(val) = get_string_value(&batch, name, row_idx) {
                        attributes.insert(name.to_string(), val);
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
    }

    Ok(products)
}

/// Read scenario rows from a parquet file.
pub fn read_scenarios(path: &Path) -> Result<Vec<ScenarioRow>, EtlError> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let schema = builder.schema().clone();

    // Upfront column validation
    let field_names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    validate_required_columns(&field_names, &["scenario_name", "value"], "read_scenarios")?;

    let reader = builder.build()?;

    let mut scenarios = Vec::new();

    for batch_result in reader {
        let batch: RecordBatch = batch_result?;
        let num_rows = batch.num_rows();

        for row_idx in 0..num_rows {
            let scenario_name =
                get_string_value(&batch, "scenario_name", row_idx).ok_or_else(|| {
                    EtlError::Parsing(format!(
                        "scenario_name is required but missing or null at row {row_idx}"
                    ))
                })?;
            let product_id = get_string_value(&batch, "product_id", row_idx).unwrap_or_default();
            let channel = get_string_value(&batch, "channel", row_idx).unwrap_or_default();
            let period = get_string_value(&batch, "period", row_idx).unwrap_or_default();
            let value = get_f64_value(&batch, "value", row_idx).ok_or_else(|| {
                EtlError::Parsing(format!(
                    "value is required but missing or null at row {row_idx}"
                ))
            })?;

            scenarios.push(ScenarioRow {
                scenario_name,
                product_id,
                channel,
                period,
                value,
            });
        }
    }

    Ok(scenarios)
}

/// Extract a string value from a record batch column.
fn get_string_value(batch: &RecordBatch, col_name: &str, row: usize) -> Option<String> {
    let col_idx = batch.schema().index_of(col_name).ok()?;
    let array = batch.column(col_idx);

    if array.is_null(row) {
        return None;
    }

    match array.data_type() {
        DataType::Utf8 => {
            let arr = array.as_string::<i32>();
            Some(arr.value(row).to_string())
        }
        DataType::LargeUtf8 => {
            let arr = array.as_string::<i64>();
            Some(arr.value(row).to_string())
        }
        DataType::Utf8View => {
            let arr = array
                .as_any()
                .downcast_ref::<arrow::array::StringViewArray>()?;
            Some(arr.value(row).to_string())
        }
        DataType::Int32 => {
            let arr = array.as_primitive::<arrow::datatypes::Int32Type>();
            Some(arr.value(row).to_string())
        }
        DataType::Int64 => {
            let arr = array.as_primitive::<arrow::datatypes::Int64Type>();
            Some(arr.value(row).to_string())
        }
        DataType::Float64 => {
            let arr = array.as_primitive::<arrow::datatypes::Float64Type>();
            Some(arr.value(row).to_string())
        }
        DataType::Boolean => {
            let arr = array
                .as_any()
                .downcast_ref::<arrow::array::BooleanArray>()?;
            Some(if arr.value(row) { "true" } else { "false" }.to_string())
        }
        DataType::Date32 => {
            // Date32 stores days since Unix epoch
            let arr = array.as_primitive::<arrow::datatypes::Date32Type>();
            let days = arr.value(row);
            let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1)?;
            let date = epoch.checked_add_signed(chrono::Duration::days(days as i64))?;
            Some(date.format("%Y-%m-%d").to_string())
        }
        DataType::Timestamp(unit, _tz) => {
            // All units use the same pattern: try to parse, fall back to
            // the raw numeric value as a string (total function, never None).
            match unit {
                arrow::datatypes::TimeUnit::Second => {
                    let arr = array.as_primitive::<arrow::datatypes::TimestampSecondType>();
                    let secs = arr.value(row);
                    Some(match chrono::DateTime::from_timestamp(secs, 0) {
                        Some(dt) => dt.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                        None => secs.to_string(),
                    })
                }
                arrow::datatypes::TimeUnit::Millisecond => {
                    let arr = array.as_primitive::<arrow::datatypes::TimestampMillisecondType>();
                    Some(format_timestamp_millis(arr.value(row)))
                }
                arrow::datatypes::TimeUnit::Microsecond => {
                    let arr = array.as_primitive::<arrow::datatypes::TimestampMicrosecondType>();
                    Some(format_timestamp_micros(arr.value(row)))
                }
                arrow::datatypes::TimeUnit::Nanosecond => {
                    let arr = array.as_primitive::<arrow::datatypes::TimestampNanosecondType>();
                    Some(format_timestamp_nanos(arr.value(row)))
                }
            }
        }
        DataType::Decimal128(_precision, scale) => {
            let arr = array.as_primitive::<arrow::datatypes::Decimal128Type>();
            let raw = arr.value(row);
            Some(format_decimal128(raw, *scale))
        }
        DataType::Dictionary(_, _) => {
            // Cast the dictionary to Utf8 regardless of value type.
            // If the cast fails, this column simply has no string representation.
            let casted = compute::cast(array, &DataType::Utf8).ok()?;
            let string_arr = casted.as_string::<i32>();
            if string_arr.is_null(row) {
                return None;
            }
            Some(string_arr.value(row).to_string())
        }
        _ => None,
    }
}

/// Format a Decimal128 raw value with the given scale.
///
/// Total function: returns a string representation for all inputs.
/// Uses checked arithmetic to avoid overflow panics.
fn format_decimal128(raw: i128, scale: i8) -> String {
    if scale <= 0 {
        // Negative scale means multiply by 10^|scale|.
        // Guard against both huge exponents and multiplication overflow.
        let abs_scale = (-scale) as u32;
        if abs_scale > 38 {
            // 10^39 exceeds i128::MAX; fall back to scientific notation
            return format!("{}e{}", raw, abs_scale);
        }
        let factor = 10i128.pow(abs_scale);
        return match raw.checked_mul(factor) {
            Some(v) => v.to_string(),
            None => format!("{}e{}", raw, abs_scale),
        };
    }
    let scale = scale as u32;
    let divisor = 10i128.pow(scale);
    let whole = raw / divisor;
    let frac = (raw % divisor).unsigned_abs();
    format!("{whole}.{frac:0>width$}", width = scale as usize)
}

/// Format a timestamp from milliseconds since epoch as ISO string.
fn format_timestamp_millis(ms: i64) -> String {
    let secs = ms / 1000;
    let nanos = ((ms % 1000).unsigned_abs() as u32) * 1_000_000;
    match chrono::DateTime::from_timestamp(secs, nanos) {
        Some(dt) => dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        None => ms.to_string(),
    }
}

/// Format a timestamp from microseconds since epoch as ISO string.
fn format_timestamp_micros(us: i64) -> String {
    let secs = us / 1_000_000;
    let nanos = ((us % 1_000_000).unsigned_abs() as u32) * 1_000;
    match chrono::DateTime::from_timestamp(secs, nanos) {
        Some(dt) => dt.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string(),
        None => us.to_string(),
    }
}

/// Format a timestamp from nanoseconds since epoch as ISO string.
fn format_timestamp_nanos(ns: i64) -> String {
    let secs = ns / 1_000_000_000;
    let nanos = (ns % 1_000_000_000).unsigned_abs() as u32;
    match chrono::DateTime::from_timestamp(secs, nanos) {
        Some(dt) => dt.format("%Y-%m-%dT%H:%M:%S%.9fZ").to_string(),
        None => ns.to_string(),
    }
}

/// Extract a float value from a record batch column.
fn get_f64_value(batch: &RecordBatch, col_name: &str, row: usize) -> Option<f64> {
    let col_idx = batch.schema().index_of(col_name).ok()?;
    let array = batch.column(col_idx);

    if array.is_null(row) {
        return None;
    }

    match array.data_type() {
        DataType::Float64 => {
            let arr = array.as_primitive::<arrow::datatypes::Float64Type>();
            Some(arr.value(row))
        }
        DataType::Float32 => {
            let arr = array.as_primitive::<arrow::datatypes::Float32Type>();
            Some(arr.value(row) as f64)
        }
        DataType::Int32 => {
            let arr = array.as_primitive::<arrow::datatypes::Int32Type>();
            Some(arr.value(row) as f64)
        }
        DataType::Int64 => {
            let arr = array.as_primitive::<arrow::datatypes::Int64Type>();
            Some(arr.value(row) as f64)
        }
        DataType::UInt32 => {
            let arr = array.as_primitive::<arrow::datatypes::UInt32Type>();
            Some(arr.value(row) as f64)
        }
        DataType::UInt64 => {
            let arr = array.as_primitive::<arrow::datatypes::UInt64Type>();
            Some(arr.value(row) as f64)
        }
        DataType::Decimal128(_precision, scale) => {
            let arr = array.as_primitive::<arrow::datatypes::Decimal128Type>();
            let raw = arr.value(row);
            let divisor = 10f64.powi(*scale as i32);
            Some(raw as f64 / divisor)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::*;
    use arrow::datatypes::{DataType, Field, Schema};
    use parquet::arrow::ArrowWriter;
    use std::sync::Arc;
    use tempfile::NamedTempFile;

    /// Helper: write a RecordBatch to a temporary Parquet file and return the path.
    fn write_parquet(batch: &RecordBatch) -> NamedTempFile {
        let tmp = NamedTempFile::new().expect("create temp file");
        let file = tmp.as_file().try_clone().expect("clone file handle");
        let mut writer =
            ArrowWriter::try_new(file, batch.schema(), None).expect("create ArrowWriter");
        writer.write(batch).expect("write batch");
        writer.close().expect("close writer");
        tmp
    }

    // -----------------------------------------------------------------------
    // detect_file_type tests
    // -----------------------------------------------------------------------

    #[test]
    fn detect_file_type_products() {
        let cols = &["product_id", "segment", "brand"];
        let result = detect_file_type_from_columns(cols).unwrap();
        assert_eq!(result, ParquetFileType::Products);
    }

    #[test]
    fn detect_file_type_products_minimal() {
        // Only product_id, no segment/brand
        let cols = &["product_id", "extra_col"];
        let result = detect_file_type_from_columns(cols).unwrap();
        assert_eq!(result, ParquetFileType::Products);
    }

    #[test]
    fn detect_file_type_scenarios() {
        let cols = &["scenario_name", "value", "channel", "period"];
        let result = detect_file_type_from_columns(cols).unwrap();
        assert_eq!(result, ParquetFileType::Scenarios);
    }

    #[test]
    fn detect_file_type_scenarios_requires_both() {
        // Only scenario_name without value => Unknown
        let cols = &["scenario_name", "channel"];
        let result = detect_file_type_from_columns(cols).unwrap();
        assert_eq!(result, ParquetFileType::Unknown);
    }

    #[test]
    fn detect_file_type_scenarios_only_value() {
        // Only value without scenario_name => Unknown
        let cols = &["value", "channel"];
        let result = detect_file_type_from_columns(cols).unwrap();
        assert_eq!(result, ParquetFileType::Unknown);
    }

    #[test]
    fn detect_file_type_unknown() {
        let cols = &["foo", "bar", "baz"];
        let result = detect_file_type_from_columns(cols).unwrap();
        assert_eq!(result, ParquetFileType::Unknown);
    }

    #[test]
    fn detect_file_type_scenarios_beats_products_when_both_present() {
        // Has both product_id and (scenario_name + value) => Scenarios wins
        let cols = &["product_id", "scenario_name", "value"];
        let result = detect_file_type_from_columns(cols).unwrap();
        assert_eq!(result, ParquetFileType::Scenarios);
    }

    #[test]
    fn detect_file_type_via_parquet_file() {
        let schema = Schema::new(vec![
            Field::new("product_id", DataType::Utf8, false),
            Field::new("segment", DataType::Utf8, true),
        ]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(StringArray::from(vec!["p1"])),
                Arc::new(StringArray::from(vec!["s1"])),
            ],
        )
        .unwrap();
        let tmp = write_parquet(&batch);
        let result = detect_file_type(tmp.path()).unwrap();
        assert_eq!(result, ParquetFileType::Products);
    }

    // -----------------------------------------------------------------------
    // read_products tests
    // -----------------------------------------------------------------------

    #[test]
    fn read_products_valid() {
        let schema = Schema::new(vec![
            Field::new("product_id", DataType::Utf8, false),
            Field::new("segment", DataType::Utf8, true),
            Field::new("subsegment", DataType::Utf8, true),
            Field::new("brand", DataType::Utf8, true),
            Field::new("sub_brand", DataType::Utf8, true),
            Field::new("color", DataType::Utf8, true), // extra attribute
        ]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(StringArray::from(vec!["P001", "P002"])),
                Arc::new(StringArray::from(vec!["Premium", "Economy"])),
                Arc::new(StringArray::from(vec!["Sub1", "Sub2"])),
                Arc::new(StringArray::from(vec!["BrandA", "BrandB"])),
                Arc::new(StringArray::from(vec!["SubA", "SubB"])),
                Arc::new(StringArray::from(vec!["Red", "Blue"])),
            ],
        )
        .unwrap();
        let tmp = write_parquet(&batch);
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
        // File with no product_id column at all
        let schema = Schema::new(vec![
            Field::new("segment", DataType::Utf8, true),
            Field::new("brand", DataType::Utf8, true),
        ]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(StringArray::from(vec!["seg"])),
                Arc::new(StringArray::from(vec!["br"])),
            ],
        )
        .unwrap();
        let tmp = write_parquet(&batch);
        let err = read_products(tmp.path()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("product_id"),
            "error should mention product_id: {msg}"
        );
    }

    #[test]
    fn read_products_null_required_field() {
        let schema = Schema::new(vec![
            Field::new("product_id", DataType::Utf8, true),
            Field::new("segment", DataType::Utf8, true),
        ]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(StringArray::from(vec![None::<&str>])),
                Arc::new(StringArray::from(vec!["seg"])),
            ],
        )
        .unwrap();
        let tmp = write_parquet(&batch);
        let err = read_products(tmp.path()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("product_id"),
            "error should mention product_id: {msg}"
        );
    }

    // -----------------------------------------------------------------------
    // read_scenarios tests
    // -----------------------------------------------------------------------

    #[test]
    fn read_scenarios_valid() {
        let schema = Schema::new(vec![
            Field::new("scenario_name", DataType::Utf8, false),
            Field::new("product_id", DataType::Utf8, true),
            Field::new("channel", DataType::Utf8, true),
            Field::new("period", DataType::Utf8, true),
            Field::new("value", DataType::Float64, false),
        ]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(StringArray::from(vec!["Base", "High"])),
                Arc::new(StringArray::from(vec!["P001", "P002"])),
                Arc::new(StringArray::from(vec!["Online", "Retail"])),
                Arc::new(StringArray::from(vec!["2024-Q1", "2024-Q2"])),
                Arc::new(Float64Array::from(vec![100.5, 200.75])),
            ],
        )
        .unwrap();
        let tmp = write_parquet(&batch);
        let scenarios = read_scenarios(tmp.path()).unwrap();
        assert_eq!(scenarios.len(), 2);
        assert_eq!(scenarios[0].scenario_name, "Base");
        assert_eq!(scenarios[0].value, 100.5);
        assert_eq!(scenarios[1].scenario_name, "High");
        assert_eq!(scenarios[1].channel, "Retail");
    }

    #[test]
    fn read_scenarios_missing_value_column() {
        let schema = Schema::new(vec![
            Field::new("scenario_name", DataType::Utf8, false),
            Field::new("channel", DataType::Utf8, true),
        ]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(StringArray::from(vec!["Base"])),
                Arc::new(StringArray::from(vec!["Online"])),
            ],
        )
        .unwrap();
        let tmp = write_parquet(&batch);
        let err = read_scenarios(tmp.path()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("value"), "error should mention value: {msg}");
    }

    #[test]
    fn read_scenarios_null_scenario_name() {
        let schema = Schema::new(vec![
            Field::new("scenario_name", DataType::Utf8, true),
            Field::new("value", DataType::Float64, false),
        ]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(StringArray::from(vec![None::<&str>])),
                Arc::new(Float64Array::from(vec![42.0])),
            ],
        )
        .unwrap();
        let tmp = write_parquet(&batch);
        let err = read_scenarios(tmp.path()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("scenario_name"),
            "error should mention scenario_name: {msg}"
        );
    }

    #[test]
    fn read_scenarios_null_value() {
        let schema = Schema::new(vec![
            Field::new("scenario_name", DataType::Utf8, false),
            Field::new("value", DataType::Float64, true),
        ]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(StringArray::from(vec!["Base"])),
                Arc::new(Float64Array::from(vec![None::<f64>])),
            ],
        )
        .unwrap();
        let tmp = write_parquet(&batch);
        let err = read_scenarios(tmp.path()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("value"), "error should mention value: {msg}");
    }

    // -----------------------------------------------------------------------
    // get_string_value tests
    // -----------------------------------------------------------------------

    /// Build a single-row RecordBatch with the given array and column name.
    fn single_col_batch(name: &str, array: Arc<dyn Array>) -> RecordBatch {
        let field = Field::new(name, array.data_type().clone(), true);
        RecordBatch::try_new(Arc::new(Schema::new(vec![field])), vec![array]).unwrap()
    }

    #[test]
    fn get_string_value_utf8() {
        let arr: Arc<dyn Array> = Arc::new(StringArray::from(vec!["hello"]));
        let batch = single_col_batch("col", arr);
        assert_eq!(
            get_string_value(&batch, "col", 0),
            Some("hello".to_string())
        );
    }

    #[test]
    fn get_string_value_large_utf8() {
        let arr: Arc<dyn Array> = Arc::new(LargeStringArray::from(vec!["large"]));
        let batch = single_col_batch("col", arr);
        assert_eq!(
            get_string_value(&batch, "col", 0),
            Some("large".to_string())
        );
    }

    #[test]
    fn get_string_value_utf8view() {
        let arr: Arc<dyn Array> = Arc::new(StringViewArray::from(vec!["view_str"]));
        let batch = single_col_batch("col", arr);
        assert_eq!(
            get_string_value(&batch, "col", 0),
            Some("view_str".to_string())
        );
    }

    #[test]
    fn get_string_value_int32() {
        let arr: Arc<dyn Array> = Arc::new(Int32Array::from(vec![42]));
        let batch = single_col_batch("col", arr);
        assert_eq!(get_string_value(&batch, "col", 0), Some("42".to_string()));
    }

    #[test]
    fn get_string_value_int64() {
        let arr: Arc<dyn Array> = Arc::new(Int64Array::from(vec![9999999999i64]));
        let batch = single_col_batch("col", arr);
        assert_eq!(
            get_string_value(&batch, "col", 0),
            Some("9999999999".to_string())
        );
    }

    #[test]
    fn get_string_value_float64() {
        let arr: Arc<dyn Array> = Arc::new(Float64Array::from(vec![3.14]));
        let batch = single_col_batch("col", arr);
        assert_eq!(get_string_value(&batch, "col", 0), Some("3.14".to_string()));
    }

    #[test]
    fn get_string_value_boolean_true() {
        let arr: Arc<dyn Array> = Arc::new(BooleanArray::from(vec![true]));
        let batch = single_col_batch("col", arr);
        assert_eq!(get_string_value(&batch, "col", 0), Some("true".to_string()));
    }

    #[test]
    fn get_string_value_boolean_false() {
        let arr: Arc<dyn Array> = Arc::new(BooleanArray::from(vec![false]));
        let batch = single_col_batch("col", arr);
        assert_eq!(
            get_string_value(&batch, "col", 0),
            Some("false".to_string())
        );
    }

    #[test]
    fn get_string_value_date32() {
        // 2024-01-15 = 19737 days since 1970-01-01
        let days = chrono::NaiveDate::from_ymd_opt(2024, 1, 15)
            .unwrap()
            .signed_duration_since(chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
            .num_days() as i32;
        let arr: Arc<dyn Array> = Arc::new(Date32Array::from(vec![days]));
        let batch = single_col_batch("col", arr);
        assert_eq!(
            get_string_value(&batch, "col", 0),
            Some("2024-01-15".to_string())
        );
    }

    #[test]
    fn get_string_value_timestamp_seconds() {
        // 2024-01-15T10:30:00Z
        let ts = chrono::DateTime::parse_from_rfc3339("2024-01-15T10:30:00Z")
            .unwrap()
            .timestamp();
        let arr: Arc<dyn Array> =
            Arc::new(TimestampSecondArray::from(vec![ts]).with_timezone("UTC"));
        let batch = single_col_batch("col", arr);
        let result = get_string_value(&batch, "col", 0).unwrap();
        assert!(
            result.contains("2024-01-15"),
            "timestamp should contain date: {result}"
        );
        assert!(
            result.contains("10:30:00"),
            "timestamp should contain time: {result}"
        );
    }

    #[test]
    fn get_string_value_timestamp_millis() {
        let ts_ms = chrono::DateTime::parse_from_rfc3339("2024-01-15T10:30:00Z")
            .unwrap()
            .timestamp_millis();
        let arr: Arc<dyn Array> =
            Arc::new(TimestampMillisecondArray::from(vec![ts_ms]).with_timezone("UTC"));
        let batch = single_col_batch("col", arr);
        let result = get_string_value(&batch, "col", 0).unwrap();
        assert!(
            result.contains("2024-01-15"),
            "timestamp should contain date: {result}"
        );
    }

    #[test]
    fn get_string_value_decimal128() {
        // 12345 with scale 2 => 123.45
        let arr = Decimal128Array::from(vec![12345i128])
            .with_precision_and_scale(10, 2)
            .unwrap();
        let arr: Arc<dyn Array> = Arc::new(arr);
        let batch = single_col_batch("col", arr);
        let result = get_string_value(&batch, "col", 0).unwrap();
        assert_eq!(result, "123.45");
    }

    #[test]
    fn get_string_value_dictionary() {
        let keys = Int32Array::from(vec![0, 1, 0]);
        let values = StringArray::from(vec!["alpha", "beta"]);
        let dict_arr = DictionaryArray::try_new(keys, Arc::new(values)).unwrap();
        let arr: Arc<dyn Array> = Arc::new(dict_arr);
        let schema = Schema::new(vec![Field::new("col", arr.data_type().clone(), true)]);
        let batch = RecordBatch::try_new(Arc::new(schema), vec![arr]).unwrap();
        assert_eq!(
            get_string_value(&batch, "col", 0),
            Some("alpha".to_string())
        );
        assert_eq!(get_string_value(&batch, "col", 1), Some("beta".to_string()));
        assert_eq!(
            get_string_value(&batch, "col", 2),
            Some("alpha".to_string())
        );
    }

    #[test]
    fn get_string_value_null_returns_none() {
        let arr: Arc<dyn Array> = Arc::new(StringArray::from(vec![None::<&str>]));
        let batch = single_col_batch("col", arr);
        assert_eq!(get_string_value(&batch, "col", 0), None);
    }

    #[test]
    fn get_string_value_missing_column_returns_none() {
        let arr: Arc<dyn Array> = Arc::new(StringArray::from(vec!["hi"]));
        let batch = single_col_batch("col", arr);
        assert_eq!(get_string_value(&batch, "nonexistent", 0), None);
    }

    // -----------------------------------------------------------------------
    // get_f64_value tests
    // -----------------------------------------------------------------------

    #[test]
    fn get_f64_value_float64() {
        let arr: Arc<dyn Array> = Arc::new(Float64Array::from(vec![3.14]));
        let batch = single_col_batch("col", arr);
        assert_eq!(get_f64_value(&batch, "col", 0), Some(3.14));
    }

    #[test]
    fn get_f64_value_float32() {
        let arr: Arc<dyn Array> = Arc::new(Float32Array::from(vec![2.5f32]));
        let batch = single_col_batch("col", arr);
        let val = get_f64_value(&batch, "col", 0).unwrap();
        assert!((val - 2.5).abs() < 1e-6);
    }

    #[test]
    fn get_f64_value_int32() {
        let arr: Arc<dyn Array> = Arc::new(Int32Array::from(vec![42]));
        let batch = single_col_batch("col", arr);
        assert_eq!(get_f64_value(&batch, "col", 0), Some(42.0));
    }

    #[test]
    fn get_f64_value_int64() {
        let arr: Arc<dyn Array> = Arc::new(Int64Array::from(vec![100i64]));
        let batch = single_col_batch("col", arr);
        assert_eq!(get_f64_value(&batch, "col", 0), Some(100.0));
    }

    #[test]
    fn get_f64_value_uint32() {
        let arr: Arc<dyn Array> = Arc::new(UInt32Array::from(vec![255u32]));
        let batch = single_col_batch("col", arr);
        assert_eq!(get_f64_value(&batch, "col", 0), Some(255.0));
    }

    #[test]
    fn get_f64_value_uint64() {
        let arr: Arc<dyn Array> = Arc::new(UInt64Array::from(vec![1000u64]));
        let batch = single_col_batch("col", arr);
        assert_eq!(get_f64_value(&batch, "col", 0), Some(1000.0));
    }

    #[test]
    fn get_f64_value_decimal128() {
        // 12345 with scale 2 => 123.45
        let arr = Decimal128Array::from(vec![12345i128])
            .with_precision_and_scale(10, 2)
            .unwrap();
        let arr: Arc<dyn Array> = Arc::new(arr);
        let batch = single_col_batch("col", arr);
        let val = get_f64_value(&batch, "col", 0).unwrap();
        assert!((val - 123.45).abs() < 1e-10);
    }

    #[test]
    fn get_f64_value_null_returns_none() {
        let arr: Arc<dyn Array> = Arc::new(Float64Array::from(vec![None::<f64>]));
        let batch = single_col_batch("col", arr);
        assert_eq!(get_f64_value(&batch, "col", 0), None);
    }

    #[test]
    fn get_f64_value_missing_column_returns_none() {
        let arr: Arc<dyn Array> = Arc::new(Float64Array::from(vec![1.0]));
        let batch = single_col_batch("col", arr);
        assert_eq!(get_f64_value(&batch, "nonexistent", 0), None);
    }

    // -----------------------------------------------------------------------
    // format_decimal128 edge cases (totality)
    // -----------------------------------------------------------------------

    #[test]
    fn format_decimal128_positive_scale() {
        assert_eq!(format_decimal128(12345, 2), "123.45");
        assert_eq!(format_decimal128(1, 3), "0.001");
        assert_eq!(format_decimal128(-12345, 2), "-123.45");
    }

    #[test]
    fn format_decimal128_zero_scale() {
        assert_eq!(format_decimal128(42, 0), "42");
    }

    #[test]
    fn format_decimal128_negative_scale() {
        // scale -2 means raw * 100
        assert_eq!(format_decimal128(5, -2), "500");
    }

    #[test]
    fn format_decimal128_overflow_falls_back_to_scientific() {
        // i128::MAX * 10 would overflow — should not panic
        let result = format_decimal128(i128::MAX, -1);
        assert!(
            result.contains('e'),
            "overflow should produce scientific notation, got: {result}"
        );
    }

    #[test]
    fn format_decimal128_huge_negative_scale() {
        // scale -50 exceeds 10^38 range — should not panic
        let result = format_decimal128(1, -50);
        assert!(
            result.contains('e'),
            "huge scale should produce scientific notation, got: {result}"
        );
    }

    // -----------------------------------------------------------------------
    // timestamp edge cases (totality)
    // -----------------------------------------------------------------------

    #[test]
    fn get_string_value_timestamp_seconds_invalid() {
        // i64::MAX seconds is far beyond valid range — should not panic or return None
        let arr: Arc<dyn Array> = Arc::new(TimestampSecondArray::from(vec![i64::MAX]));
        let batch = single_col_batch("col", arr);
        let result = get_string_value(&batch, "col", 0);
        assert!(
            result.is_some(),
            "invalid timestamp should return raw value, not None"
        );
    }

    #[test]
    fn format_timestamp_millis_invalid() {
        // i64::MAX millis — should fall back to numeric string
        let result = format_timestamp_millis(i64::MAX);
        assert_eq!(result, i64::MAX.to_string());
    }

    #[test]
    fn format_timestamp_nanos_extreme_value_is_total() {
        // i64::MAX nanos happens to be a valid date (2262-04-11).
        // The point: the function is total — it never panics.
        let result = format_timestamp_nanos(i64::MAX);
        assert!(!result.is_empty());

        // i64::MIN nanos is also valid but negative epoch — still total
        let result_neg = format_timestamp_nanos(i64::MIN);
        assert!(!result_neg.is_empty());
    }
}
