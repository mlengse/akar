//! Auto-extracted from physical_operator.rs
use kuzu_common::types::Value;
use kuzu_common::vector::{DataChunk, ValueVector};
use std::sync::Arc;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};

// ==================== PrimaryKeyScan ====================

/// Physical operator for scanning a table by primary key lookup from an input key column.
///
/// Reads keys from `key_column_idx` in the input `DataChunk`s, performs
/// point lookups using the ART index on a node table, and produces an
/// output chunk containing the retrieved rows.
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
        let mut output_chunks = Vec::new();

        for chunk in input {
            if chunk.size == 0 {
                continue;
            }
            if self.key_column_idx >= chunk.fields.len() {
                return Err(format!("PrimaryKeyScan key column index out of bounds"));
            }

            let key_field = &chunk.fields[self.key_column_idx];
            let mut row_ids = Vec::with_capacity(chunk.size);
            
            for i in 0..chunk.size {
                if key_field.is_null(i) { continue; }
                let val = key_field.get_value(i).unwrap();
                // Use ART index via lookup_by_pk_range to match C++ architecture for index scans
                let row_ids_from_idx = node_table.lookup_by_pk_range(
                    Some(&val),
                    true,
                    Some(&val),
                    true,
                    1,
                );
                
                if let Some(&row_id) = row_ids_from_idx.first() {
                    row_ids.push(row_id as usize);
                }
            }
            
            if row_ids.is_empty() {
                continue;
            }
            
            let mut new_fields = Vec::with_capacity(num_cols);
            let mut field_names = Vec::with_capacity(num_cols);
            
            for col_idx in 0..num_cols {
                let phys_type = kuzu_common::types::physical_type_from_logical(node_table.columns[col_idx].logical_type);
                let mut v = ValueVector::new(phys_type, row_ids.len());
                v.resize(row_ids.len());
                
                for (out_idx, &row_id) in row_ids.iter().enumerate() {
                    let val = node_table.get_value(row_id, col_idx).cloned().unwrap_or(Value::Null);
                    if matches!(val, Value::Null) {
                        v.set_null(out_idx, true);
                    } else {
                        crate::physical::common::store_value_in_vector(&mut v, out_idx, &val);
                    }
                }
                new_fields.push(v);
                field_names.push(node_table.columns[col_idx].name.clone());
            }
            
            output_chunks.push(DataChunk {
                fields: new_fields,
                size: row_ids.len(),
                field_names,
            });
        }

        if output_chunks.is_empty() {
            Ok(vec![DataChunk {
                fields: vec![],
                size: 0,
                field_names: vec![],
            }])
        } else {
            Ok(output_chunks)
        }
    }
}

