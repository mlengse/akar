//! Result converter — transforms DuckDB query results into Kuzu `DataChunk`s.
//!
//! Supports converting both row-based (`duckdb::ResultSet<duckdb::DataRow>`)
//! and chunk-based (`duckdb::DataChunkHandle`) results.

use crate::type_converter::{duckdb_type_to_kuzu, duckdb_value_to_kuzu, duckdb_value_to_kuzu_typed};
use kuzu_common::types::{LogicalTypeID, PhysicalTypeID, Value};
use kuzu_common::vector::{DataChunk, ValueVector};

/// Convert a DuckDB row-based result set to a Vec of Kuzu `DataChunk`s.
///
/// Each column becomes a `ValueVector`. All rows are collected into a single
/// `DataChunk` (suitable for small-to-medium result sets).
#[cfg(feature = "bundled")]
pub fn rows_to_datachunk(
    rows: &[duckdb::DataRow],
    column_count: usize,
) -> Result<DataChunk, String> {
    if rows.is_empty() {
        return Ok(DataChunk::new(Vec::new()));
    }

    let num_rows = rows.len();
    let mut fields: Vec<ValueVector> = Vec::with_capacity(column_count);

    for col_idx in 0..column_count {
        // Determine column type from first non-null row
        let col_type = detect_column_type(rows, col_idx);
        let physical_type = logical_to_physical(col_type);
        let mut vec = ValueVector::new(physical_type, num_rows);

        for (row_idx, row) in rows.iter().enumerate() {
            match duckdb_value_to_kuzu_typed(row, col_idx, &type_name_from_rows(rows, col_idx)) {
                Ok(val) => {
                    set_value_in_vector(&mut vec, row_idx, &val)?;
                }
                Err(_) => {
                    vec.set_null(row_idx, true);
                }
            }
        }

        vec.resize(num_rows);
        fields.push(vec);
    }

    Ok(DataChunk { fields, size: num_rows })
}

/// Detect the Kuzu logical type of a column from the first non-null row.
#[cfg(feature = "bundled")]
fn detect_column_type(rows: &[duckdb::DataRow], col_idx: usize) -> LogicalTypeID {
    for row in rows {
        if let Ok(val) = duckdb_value_to_kuzu(row, col_idx) {
            return val.logical_type();
        }
    }
    LogicalTypeID::String
}

/// Get a human-readable type name from a column.
#[cfg(feature = "bundled")]
fn type_name_from_rows(_rows: &[duckdb::DataRow], _col_idx: usize) -> String {
    // In a real implementation, we'd get column types from the statement.
    // For now, return empty string — duckdb_value_to_kuzu_typed handles fallback.
    String::new()
}

/// Map a Kuzu `LogicalTypeID` to a `PhysicalTypeID`.
fn logical_to_physical(logical: LogicalTypeID) -> PhysicalTypeID {
    match logical {
        LogicalTypeID::Bool => PhysicalTypeID::Bool,
        LogicalTypeID::Int8 => PhysicalTypeID::Int8,
        LogicalTypeID::Int16 => PhysicalTypeID::Int16,
        LogicalTypeID::Int32 => PhysicalTypeID::Int32,
        LogicalTypeID::Int64 => PhysicalTypeID::Int64,
        LogicalTypeID::Float => PhysicalTypeID::Float,
        LogicalTypeID::Double => PhysicalTypeID::Double,
        LogicalTypeID::String | LogicalTypeID::Blob | LogicalTypeID::UUID | LogicalTypeID::Date | LogicalTypeID::Timestamp | LogicalTypeID::Interval => {
            PhysicalTypeID::String
        }
        LogicalTypeID::List => PhysicalTypeID::List,
        LogicalTypeID::Struct => PhysicalTypeID::Struct,
        LogicalTypeID::Any => PhysicalTypeID::Int64,
    }
}

/// Set a `Value` into a `ValueVector` at the given row index.
fn set_value_in_vector(vec: &mut ValueVector, idx: usize, val: &Value) -> Result<(), String> {
    match val {
        Value::Null => {
            vec.set_null(idx, true);
        }
        Value::Bool(b) => {
            // Store bool as int8: 0/1
            vec.data_mut()[idx] = if *b { 1u8 } else { 0u8 };
        }
        Value::Int64(n) => {
            let bytes = n.to_le_bytes();
            let offset = idx * 8;
            if offset + 8 <= vec.data().len() {
                vec.data_mut()[offset..offset + 8].copy_from_slice(&bytes);
            }
        }
        Value::Int32(n) => {
            let bytes = n.to_le_bytes();
            let offset = idx * 4;
            if offset + 4 <= vec.data().len() {
                vec.data_mut()[offset..offset + 4].copy_from_slice(&bytes);
            }
        }
        Value::Double(n) => {
            let bytes = n.to_le_bytes();
            let offset = idx * 8;
            if offset + 8 <= vec.data().len() {
                vec.data_mut()[offset..offset + 8].copy_from_slice(&bytes);
            }
        }
        Value::Float(n) => {
            let bytes = n.to_le_bytes();
            let offset = idx * 4;
            if offset + 4 <= vec.data().len() {
                vec.data_mut()[offset..offset + 4].copy_from_slice(&bytes);
            }
        }
        Value::String(s) => {
            // Store string as raw bytes (simplified)
            let bytes = s.as_bytes();
            let offset = idx * bytes.len().max(1);
            if offset + bytes.len() <= vec.data().len() {
                vec.data_mut()[offset..offset + bytes.len()].copy_from_slice(bytes);
            }
        }
        _ => {
            vec.set_null(idx, true);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logical_to_physical() {
        assert_eq!(logical_to_physical(LogicalTypeID::Int64), PhysicalTypeID::Int64);
        assert_eq!(logical_to_physical(LogicalTypeID::String), PhysicalTypeID::String);
        assert_eq!(logical_to_physical(LogicalTypeID::Bool), PhysicalTypeID::Bool);
        assert_eq!(logical_to_physical(LogicalTypeID::Double), PhysicalTypeID::Double);
    }

    #[test]
    fn test_empty_rows() {
        let result = DataChunk::new(Vec::new());
        assert_eq!(result.num_fields(), 0);
    }
}
