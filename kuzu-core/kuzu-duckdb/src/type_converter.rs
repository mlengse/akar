//! Type converter between DuckDB values and Kuzu types/values.
//!
//! Maps DuckDB types (INTEGER, BIGINT, VARCHAR, DOUBLE, BOOLEAN, etc.)
//! to Kuzu `LogicalTypeID` and `Value` types.

use kuzu_common::types::LogicalTypeID;
#[cfg(feature = "bundled")]
use kuzu_common::types::Value;

/// Convert a DuckDB logical type string to a Kuzu `LogicalTypeID`.
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
        "UUID" => LogicalTypeID::Uuid,
        "INTEGER[]" | "BIGINT[]" | "VARCHAR[]" | "DOUBLE[]" => LogicalTypeID::List,
        "STRUCT" | "MAP" => LogicalTypeID::Struct,
        _ => {
            tracing::warn!("Unknown DuckDB type '{}', defaulting to String", duckdb_type);
            LogicalTypeID::String
        }
    }
}

/// Convert a duckdb::types::Value to a Kuzu Value.
#[cfg(feature = "bundled")]
pub fn duckdb_value_to_kuzu(duck_val: &duckdb::types::Value) -> Value {
    use duckdb::types::Value as DuckValue;
    match duck_val {
        DuckValue::Null => Value::Null,
        DuckValue::Boolean(b) => Value::Bool(*b),
        DuckValue::TinyInt(n) => Value::Int64(*n as i64),
        DuckValue::SmallInt(n) => Value::Int64(*n as i64),
        DuckValue::Int(n) => Value::Int32(*n),
        DuckValue::BigInt(n) => Value::Int64(*n),
        DuckValue::HugeInt(n) => Value::String(n.to_string()),
        DuckValue::Float(n) => Value::Float(*n),
        DuckValue::Double(n) => Value::Double(*n),
        DuckValue::Text(s) => Value::String(s.clone()),
        DuckValue::Date32(_) => Value::String(format!("{:?}", duck_val)),
        _ => Value::String(format!("{:?}", duck_val)),
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
        assert_eq!(duckdb_type_to_kuzu("UUID"), LogicalTypeID::Uuid);
    }

    #[test]
    fn test_duckdb_type_to_kuzu_unknown() {
        assert_eq!(duckdb_type_to_kuzu("UNKNOWN_TYPE"), LogicalTypeID::String);
    }
}
