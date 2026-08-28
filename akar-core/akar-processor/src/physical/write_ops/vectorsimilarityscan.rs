//! Auto-extracted from physical_operator.rs
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use akar_common::types::Value;
use akar_common::vector::DataChunk;
use akar_storage::table::TableCatalog;
use std::sync::Arc;

// ==================== VectorSimilarityScan ====================

/// Physical operator for vector similarity search using an HNSW index.
///
/// Searches the `VectorIndexTable` for the top-K nearest neighbours and
/// looks up the corresponding rows from the `NodeTable` to produce
/// output columns including a `distance` column.
pub struct PhysicalVectorSimilarityScan {
    pub index_name: String,
    pub index_id: u64,
    pub query_vector: Vec<f64>,
    pub top_k: u64,
    pub table_name: String,
    pub table_catalog: Option<Arc<TableCatalog>>,
}

impl PhysicalOperatorExec for PhysicalVectorSimilarityScan {
    fn operator_type(&self) -> &str {
        "vector_similarity_scan"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        let tc = self
            .table_catalog
            .clone()
            .ok_or_else(|| "No table catalog available for VectorSimilarityScan".to_string())?;

        // Resolve the vector index — by name if given, or find first index on the table
        let vi = if self.index_name.is_empty() {
            // Find the first vector index on this table
            // Scan all vector indexes to find one matching this table
            let index_name = {
                let mut found_name = String::new();
                for entry in tc.all_vector_indexes() {
                    if entry.table_name == self.table_name {
                        found_name = entry.name.clone();
                        break;
                    }
                }
                if found_name.is_empty() {
                    return Err(format!("No vector index found on table '{}'", self.table_name).into());
                }
                found_name
            };
            tc.get_vector_index_by_name(&index_name)
                .ok_or_else(|| format!("Vector index '{}' not found", index_name))?
        } else {
            tc.get_vector_index_by_name(&self.index_name)
                .ok_or_else(|| format!("Vector index '{}' not found", self.index_name))?
        };

        // Search the HNSW index for top-K nearest neighbours
        let results = vi.hnsw().search(&self.query_vector, self.top_k as usize);
        drop(vi); // Release the DashMap reference

        if results.is_empty() {
            return Ok(vec![DataChunk::new(vec![], vec![])]);
        }

        // Look up rows from the node table
        let node_table = tc
            .get_node_table_by_name(&self.table_name)
            .ok_or_else(|| format!("Node table '{}' not found", self.table_name))?;

        let num_cols = node_table.columns.len();
        let num_results = results.len();

        // Build output columns: all table columns + distance + internal `_id`
        // (physical row offset, the identity column other operators resolve).
        // Convention matches `PhysicalArtIndexRangeScan` (copyfrom.rs).
        let mut output_columns: Vec<Vec<Value>> = vec![Vec::with_capacity(num_results); num_cols + 2];

        for (dist, row_id) in &results {
            // Add distance (second-to-last) and `_id` (last) columns.
            output_columns[num_cols].push(Value::Double(*dist));
            output_columns[num_cols + 1].push(Value::Int64(*row_id as i64));

            // Look up each column value from the node table
            for (col_idx, out_col) in output_columns.iter_mut().enumerate().take(num_cols) {
                match node_table.get_value(*row_id, col_idx) {
                    Some(val) => out_col.push(val.clone()),
                    None => out_col.push(Value::Null),
                }
            }
        }

        // Convert column-major Vec<Vec<Value>> into an Arrow-native DataChunk.
        // Every column is built via arrow_array_from_values, which encodes
        // `Value::List` into a proper ListArray. The previous ValueVector path
        // collapsed FLOAT[] (List) columns to NULL when read back downstream,
        // breaking the cosine threshold filter / ORDER BY re-evaluation (P71.4).
        use akar_common::arrow_vector::{ArrowVector, arrow_array_from_values};
        use akar_common::types::{PhysicalTypeID, physical_type_from_logical};

        let mut fields = Vec::with_capacity(num_cols + 2);
        let mut field_types = Vec::with_capacity(num_cols + 2);

        // Add table columns — typed per column from the node table schema
        // (P52.40: forcing every column to Double corrupted Int64/String data).
        for col_idx in 0..num_cols {
            let phys_type = physical_type_from_logical(node_table.columns[col_idx].logical_type);
            fields.push(ArrowVector::new(
                arrow_array_from_values(&output_columns[col_idx]),
                phys_type,
            ));
            field_types.push(phys_type);
        }

        // Add the distance column (Double).
        fields.push(ArrowVector::new(
            arrow_array_from_values(&output_columns[num_cols]),
            PhysicalTypeID::Double,
        ));
        field_types.push(PhysicalTypeID::Double);

        // Add internal `_id` (physical row offset) column, found by name by
        // DELETE/SET/INSERT/extend machinery (row_id_column_index).
        fields.push(ArrowVector::new(
            arrow_array_from_values(&output_columns[num_cols + 1]),
            PhysicalTypeID::Int64,
        ));
        field_types.push(PhysicalTypeID::Int64);

        let arrow_fields = fields.iter().map(|v| v.array.clone()).collect::<Vec<_>>();
        let arrow_field_types = field_types;

        let mut field_names: Vec<String> = node_table.columns.iter().map(|c| c.name.clone()).collect();
        field_names.push("distance".to_string());
        field_names.push("_id".to_string());

        Ok(vec![DataChunk {
            fields: arrow_fields,
            field_types: arrow_field_types,
            size: num_results,
            field_names,
            sel_vector: None,
        }])
    }
}
