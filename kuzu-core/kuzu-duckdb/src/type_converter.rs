//! Type converter between DuckDB logical types / values and Kuzu types / values.
//!
//! Maps DuckDB types (INTEGER, BIGINT, VARCHAR, DOUBLE, BOOLEAN, etc.)
//! to Kuzu `LogicalTypeID` and `Value` types.

use kuzu_common::types::{LogicalTypeID, Value};

/// Convert a DuckDB logical type string to a Kuzu `LogicalTypeID`.
///
/// This is used when DuckDB returns query results and we need to know
/// the Kuzu type of each column.
pub fn duckdb_type_to_kuzu(duckdb_type: &str) -> LogicalTypeID {
    match duckdb_type.to_uppercase().as_str() {
        "BOOLEAN" | "BOOL" => LogicalTypeID::Bool,
        "TINYINT" => LogicalTypeID::Int8,
        "SMALLINT" | "INT2" => LogicalTypeID::Int16,
        "INTEGER" | "INT" | "INT4" | "INT32" => LogicalTypeID::Int32,
        "BIGINT" | "INT8" | "INT64" | "LONG" => LogicalTypeID::Int64,
        "FLOAT" | "REAL" | "FLOAT4" => LogicalTypeID::Float,
        "DOUBLE" | "FLOAT8" | "DECIMAL" | "NUMERIC" => LogicalTypeID::Double,
        "VARCHAR" | "TEXT" | "STRING" | "CHAR" | "BPCHAR" => LogicalTypeID::String,
        "DATE" => LogicalTypeID::Date,
        "TIMESTAMP" | "TIMESTAMP_SEC" | "TIMESTAMP_MS" | "TIMESTAMP_NS" => LogicalTypeID::Timestamp,
        "INTERVAL" => LogicalTypeID::Interval,
        "BLOB" | "BYTEA" => LogicalTypeID::Blob,
        "UUID" => LogicalTypeID::UUID,
        "INTEGER[]" | "BIGINT[]" | "VARCHAR[]" | "DOUBLE[]" => LogicalTypeID::List,
        "STRUCT" | "MAP" => LogicalTypeID::Struct,
        _ => {
            tracing::warn!("Unknown DuckDB type '{}', defaulting to String", duckdb_type);
            LogicalTypeID::String
        }
    }
}

/// Convert a DuckDB value reference to a Kuzu `Value`.
///
/// Handles the common DuckDB types by matching on the dynamic type
/// information available through DuckDB's type system.
#[cfg(feature = "bundled")]
pub fn duckdb_value_to_kuzu(
    row: &duckdb::DataRow,
    idx: usize,
) -> Result<Value, String> {
    // Try each type in order of likelihood
    if let Ok(val) = row.get::<_, i64>(idx) {
        return Ok(Value::Int64(val));
    }
    if let Ok(val) = row.get::<_, i32>(idx) {
        return Ok(Value::Int32(val));
    }
    if let Ok(val) = row.get::<_, f64>(idx) {
        return Ok(Value::Double(val));
    }
    if let Ok(val) = row.get::<_, f32>(idx) {
        return Ok(Value::Float(val));
    }
    if let Ok(val) = row.get::<_, bool>(idx) {
        return Ok(Value::Bool(val));
    }
    if let Ok(val) = row.get::<_, String>(idx) {
        return Ok(Value::String(val));
    }
    if let Ok(val) = row.get::<_, Vec<i64>>(idx) {
        return Ok(Value::List(val.into_iter().map(Value::Int64).collect()));
    }
    if let Ok(val) = row.get::<_, Vec<String>>(idx) {
        return Ok(Value::List(val.into_iter().map(Value::String).collect()));
    }
    // Try i64 as fallback
    Err(format!("Cannot convert DuckDB value at index {idx}"))
}

/// Convert a DuckDB value to a Kuzu Value using the column type name.
#[cfg(feature = "bundled")]
pub fn duckdb_value_to_kuzu_typed(
    row: &duckdb::DataRow,
    idx: usize,
    col_type: &str,
) -> Result<Value, String> {
    match col_type.to_uppercase().as_str() {
        "BOOLEAN" | "BOOL" => {
            let val: bool = row.get(idx).map_err(|e| format!("Bool conversion: {e}"))?;
            Ok(Value::Bool(val))
        }
        "TINYINT" => {
            let val: i8 = row.get(idx).map_err(|e| format!("TinyInt conversion: {e}"))?;
            Ok(Value::Int64(val as i64))
        }
        "SMALLINT" | "INT2" => {
            let val: i16 = row.get(idx).map_err(|e| format!("SmallInt conversion: {e}"))?;
            Ok(Value::Int64(val as i64))
        }
        "INTEGER" | "INT" | "INT4" | "INT32" => {
            let val: i32 = row.get(idx).map_err(|e| format!("Int conversion: {e}"))?;
            Ok(Value::Int32(val))
        }
        "BIGINT" | "INT8" | "INT64" | "LONG" => {
            let val: i64 = row.get(idx).map_err(|e| format!("BigInt conversion: {e}"))?;
            Ok(Value::Int64(val))
        }
        "FLOAT" | "REAL" | "FLOAT4" => {
            let val: f32 = row.get(idx).map_err(|e| format!("Float conversion: {e}"))?;
            Ok(Value::Float(val))
        }
        "DOUBLE" | "FLOAT8" => {
            let val: f64 = row.get(idx).map_err(|e| format!("Double conversion: {e}"))?;
            Ok(Value::Double(val))
        }
        "VARCHAR" | "TEXT" | "STRING" | "CHAR" | "BPCHAR" => {
            let val: String = row.get(idx).map_err(|e| format!("String conversion: {e}"))?;
            Ok(Value::String(val))
        }
        _ => {
            // Fallback
            duckdb_value_to_kuzu(row, idx)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_duckdb_type_to_kuzu() {
        assert_eq!(duckdb_type_to_kuzu("INTEGER"), LogicalTypeID::Int32);
        assert_eq!(duckdb_type_to_kuzu("BIGINT"), LogicalTypeID::Int64);
        assert_eq!(duckdb_type_to_kuzu("VARCHAR"), LogicalTypeID::String);
        assert_eq!(duckdb_type_to_kuzu("DOUBLE"), LogicalTypeID::Double);
        assert_eq!(duckdb_type_to_kuzu("BOOLEAN"), LogicalTypeID::Bool);
        assert_eq!(duckdb_type_to_kuzu("FLOAT"), LogicalTypeID::Float);
        assert_eq!(duckdb_type_to_kuzu("DATE"), LogicalTypeID::Date);
        assert_eq!(duckdb_type_to_kuzu("TIMESTAMP"), LogicalTypeID::Timestamp);
        assert_eq!(duckdb_type_to_kuzu("BLOB"), LogicalTypeID::Blob);
        assert_eq!(duckdb_type_to_kuzu("UUID"), LogicalTypeID::UUID);
    }

    #[test]
    fn test_duckdb_type_to_kuzu_unknown() {
        assert_eq!(duckdb_type_to_kuzu("UNKNOWN_TYPE"), LogicalTypeID::String);
    }
}
