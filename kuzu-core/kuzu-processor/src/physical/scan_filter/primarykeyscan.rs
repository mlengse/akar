//! Auto-extracted from physical_operator.rs
use kuzu_common::types::Value;
use kuzu_common::vector::{DataChunk, ValueVector};
use std::sync::Arc;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};

// ==================== PrimaryKeyScan ====================

/// Physical operator for scanning a table by primary key lookup from an input key column.
///
/// Reads keys from `key_column_idx` in the input `DataChunk`s, performs
/// batched point lookups using the hash index on a node table, and produces
/// a contiguous output `DataChunk` containing the retrieved rows.
pub struct PhysicalPrimaryKeyScan {
    pub table_name: String,
    pub table_id: u64,
    pub key_column_idx: usize,
    pub table_catalog: Arc<kuzu_storage::table::TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalPrimaryKeyScan {
    fn operator_type(&self) -> &str {
        "primary_key_scan"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let node_table = self
            .table_catalog
            .get_node_table_by_name(&self.table_name)
            .ok_or_else(|| format!("Node table '{}' not found for PrimaryKeyScan", self.table_name))?;

        let num_cols = node_table.columns.len();
        let mut all_row_ids: Vec<usize> = Vec::new();

        for chunk in &input {
            if chunk.size == 0 {
                continue;
            }
            if self.key_column_idx >= chunk.fields.len() {
                return Err("PrimaryKeyScan key column index out of bounds".to_string());
            }

            let key_field = &chunk.fields[self.key_column_idx];

            // Extract all keys from the input chunk into a flat vector
            let keys: Vec<Value> = (0..chunk.size)
                .filter_map(|i| {
                    if key_field.is_null(i) {
                        None
                    } else {
                        key_field.get_value(i)
                    }
                })
                .collect();

            if keys.is_empty() {
                continue;
            }

            // Batch lookup: single pass over the hash index for all keys.
            // Explicitly deref through the DashMap Ref guard to reach NodeTable.
            let table: &kuzu_storage::NodeTable = &*node_table;
            let lookups = table.lookup_by_pk_batch(&keys);

            for lookup in lookups {
                if let Some(row_id) = lookup {
                    all_row_ids.push(row_id as usize);
                }
            }
        }

        if all_row_ids.is_empty() {
            return Ok(vec![DataChunk {
                fields: vec![],
                size: 0,
                field_names: vec![],
            }]);
        }

        // Build a single contiguous output DataChunk from all matched row IDs
        let total = all_row_ids.len();
        let mut new_fields = Vec::with_capacity(num_cols);
        let mut field_names = Vec::with_capacity(num_cols);

        for col_idx in 0..num_cols {
            let phys_type = kuzu_common::types::physical_type_from_logical(
                node_table.columns[col_idx].logical_type,
            );
            let mut v = ValueVector::new(phys_type, total);
            v.resize(total);

            for (out_idx, &row_id) in all_row_ids.iter().enumerate() {
                let val = node_table
                    .get_value(row_id, col_idx)
                    .cloned()
                    .unwrap_or(Value::Null);
                if matches!(val, Value::Null) {
                    v.set_null(out_idx, true);
                } else {
                    crate::physical::common::store_value_in_vector(&mut v, out_idx, &val);
                }
            }
            new_fields.push(v);
            field_names.push(node_table.columns[col_idx].name.clone());
        }

        Ok(vec![DataChunk {
            fields: new_fields,
            size: total,
            field_names,
        }])
    }
}

