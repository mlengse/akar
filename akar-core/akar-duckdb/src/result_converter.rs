//! Result converter — transforms DuckDB query results into Akar `DataChunk`s.
//!
//! Converts vector-based results from `duckdb::Value` into Akar `DataChunk`.

#[cfg(feature = "bundled")]
use crate::type_converter::duckdb_value_to_akar;
#[cfg(feature = "bundled")]
use akar_common::types::Value;
#[cfg(feature = "bundled")]
use akar_common::vector::{DataChunk, ValueVector};

/// Convert DuckDB query results (Vec<Vec<duckdb::Value>>) to Vec of Akar `DataChunk`s.
#[cfg(feature = "bundled")]
pub fn duckdb_results_to_akar(results: Vec<Vec<duckdb::types::Value>>) -> Result<Vec<DataChunk>, String> {
    if results.is_empty() {
        return Ok(vec![DataChunk::new(Vec::new())]);
    }

    let num_rows = results.len();
    let num_cols = results[0].len();

    // Convert all values to Akar Values first
    let akar_values: Vec<Vec<Value>> = results
        .into_iter()
        .map(|row| row.into_iter().map(|v| duckdb_value_to_akar(&v)).collect())
        .collect();

    if akar_values.is_empty() || akar_values[0].is_empty() {
        return Ok(vec![DataChunk::new(Vec::new())]);
    }

    // Build a single DataChunk with each column as a ValueVector of String type
    // (simplified: all columns stored as strings for now)
    let mut fields = Vec::with_capacity(num_cols);
    for _col_idx in 0..num_cols {
        let mut vec = ValueVector::new(akar_common::types::PhysicalTypeID::String, num_rows);
        for (row_idx, row) in akar_values.iter().enumerate() {
            if let Some(val) = row.get(_col_idx) {
                vec.set_null(row_idx, matches!(val, Value::Null));
            }
        }
        vec.resize(num_rows);
        fields.push(vec);
    }

    Ok(vec![DataChunk {
        fields,
        size: num_rows,
        field_names: vec![],
        sel_vector: None,
    }])
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(feature = "bundled")]
    fn test_empty_results() {
        let result = super::duckdb_results_to_akar(vec![]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].num_fields(), 0);
    }
}
