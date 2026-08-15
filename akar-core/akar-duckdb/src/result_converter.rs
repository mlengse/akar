//! Result converter — transforms DuckDB query results into Akar `DataChunk`s.
//!
//! Converts row-based results from `duckdb::types::Value` into Akar `DataChunk`s
//! of Arrow `StringArray` columns (every column is rendered as text for now).

#[cfg(feature = "bundled")]
use akar_common::types::PhysicalTypeID;
#[cfg(feature = "bundled")]
use akar_common::vector::DataChunk;
#[cfg(feature = "bundled")]
use std::sync::Arc;

/// Render a single `duckdb::types::Value` as its textual representation.
///
/// Unlike `{:?}` debugging output, this produces a stable SQL-ish literal so
/// the value can be returned from `duckdb_query`. NULL stays "NULL".
#[cfg(feature = "bundled")]
pub fn duckdb_value_to_string(val: &duckdb::types::Value) -> String {
    use duckdb::types::Value as DuckValue;
    match val {
        DuckValue::Null => "NULL".into(),
        DuckValue::Boolean(b) => b.to_string(),
        DuckValue::TinyInt(n) => n.to_string(),
        DuckValue::SmallInt(n) => n.to_string(),
        DuckValue::Int(n) => n.to_string(),
        DuckValue::BigInt(n) => n.to_string(),
        DuckValue::HugeInt(n) => n.to_string(),
        DuckValue::Float(n) => n.to_string(),
        DuckValue::Double(n) => n.to_string(),
        DuckValue::Text(s) => s.clone(),
        DuckValue::Blob(b) => format!("<blob:{} bytes>", b.len()),
        _ => format!("{:?}", val),
    }
}

/// Convert DuckDB query results (Vec<Vec<duckdb::types::Value>>) to one Akar `DataChunk`.
///
/// Returns a single chunk with one `StringArray` per result column; NULL cells
/// become null entries in the array. The previous implementation only wrote the
/// null flags and never the values (and did not even compile: it stored
/// `ValueVector`s where `ArrayRef`s are required).
#[cfg(feature = "bundled")]
pub fn duckdb_results_to_akar(results: Vec<Vec<duckdb::types::Value>>) -> Result<Vec<DataChunk>, String> {
    if results.is_empty() || results[0].is_empty() {
        return Ok(vec![DataChunk::new(Vec::new(), Vec::new())]);
    }

    let num_cols = results[0].len();
    let mut arrays: Vec<arrow::array::ArrayRef> = Vec::with_capacity(num_cols);
    for col in 0..num_cols {
        let values: Vec<Option<String>> = results
            .iter()
            .map(|row| row.get(col).map(duckdb_value_to_string).filter(|s| s != "NULL"))
            .collect();
        arrays.push(Arc::new(arrow::array::StringArray::from_iter(values.into_iter())));
    }

    let field_types = vec![PhysicalTypeID::String; num_cols];
    Ok(vec![DataChunk::new(arrays, field_types)])
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(feature = "bundled")]
    fn test_empty_results() {
        let result = super::duckdb_results_to_akar(vec![]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].num_fields(), 0);
        assert_eq!(result[0].size, 0);
    }

    #[test]
    #[cfg(feature = "bundled")]
    fn test_converts_rows_to_columns() {
        use duckdb::types::Value;
        let results = vec![
            vec![Value::Text("alice".into()), Value::Int(30)],
            vec![Value::Text("bob".into()), Value::Int(25)],
        ];
        let chunks = super::duckdb_results_to_akar(results).unwrap();
        let chunk = &chunks[0];
        assert_eq!(chunk.num_fields(), 2);
        assert_eq!(chunk.size, 2);
        assert_eq!(chunk.field_names, Vec::<String>::new());
    }
}
