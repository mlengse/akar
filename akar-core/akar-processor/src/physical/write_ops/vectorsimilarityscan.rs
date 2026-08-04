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

        // Build output columns: all table columns + distance column
        let mut output_columns: Vec<Vec<Value>> = vec![Vec::with_capacity(num_results); num_cols + 1];

        for (dist, row_id) in &results {
            // Add distance as the last column
            output_columns[num_cols].push(Value::Double(*dist));

            // Look up each column value from the node table
            for (col_idx, out_col) in output_columns.iter_mut().enumerate().take(num_cols) {
                match node_table.get_value(*row_id, col_idx) {
                    Some(val) => out_col.push(val.clone()),
                    None => out_col.push(Value::Null),
                }
            }
        }

        drop(node_table);

        // Convert column-major Vec<Vec<Value>> to DataChunks
        use akar_common::types::PhysicalTypeID;
        use akar_common::vector::{DataChunk, ValueVector};

        let mut fields = Vec::with_capacity(num_cols + 1);

        // Add table columns
        for (_col_idx, col_data) in output_columns.iter().enumerate().take(num_cols) {
            let mut v = ValueVector::new(PhysicalTypeID::Double, num_results);
            v.resize(num_results);
            for (i, val) in col_data.iter().enumerate() {
                match val {
                    Value::Double(d) => v.set_double(i, *d),
                    Value::Int64(x) => {
                        let buf = &mut v.data_mut()[i * 8..(i + 1) * 8];
                        buf.copy_from_slice(&x.to_le_bytes());
                        v.set_null(i, false);
                    }
                    Value::String(_) => {
                        v.set_value(i, val)?;
                    }
                    Value::Null => {
                        v.set_null(i, true);
                    }
                    _ => {
                        v.set_null(i, true);
                    }
                }
            }
            fields.push(v);
        }

        // Add distance column
        let dist_data = &output_columns[num_cols];
        let mut dist_v = ValueVector::new(PhysicalTypeID::Double, num_results);
        dist_v.resize(num_results);
        for (i, val) in dist_data.iter().enumerate() {
            if let Value::Double(d) = val {
                dist_v.set_double(i, *d);
            } else {
                dist_v.set_null(i, true);
            }
        }
        fields.push(dist_v);

        let arrow_fields = fields
            .iter()
            .map(|v| akar_common::arrow_vector::ArrowVector::from_legacy(v).array)
            .collect::<Vec<_>>();
        let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();

        Ok(vec![DataChunk {
            fields: arrow_fields,
            field_types: arrow_field_types,
            size: num_results,
            field_names: vec![],
            sel_vector: None,
        }])
    }
}
