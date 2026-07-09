//! Auto-extracted from physical_operator.rs
use kuzu_common::types::{PhysicalTypeID, Value};
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_parser::ast::Constant;
use kuzu_storage::table::TableCatalog;
use std::sync::Arc;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};

// ==================== Delete ====================

/// Physical operator for DELETE — removes rows from a node or rel table.
pub struct PhysicalDelete {
    pub table_name: String,
    pub table_id: u64,
    pub primary_key_column: String,
    pub is_node: bool,
    pub detach: bool,
    /// Row indices to delete (found by the scan/filter pipeline).
    pub row_indices: Vec<u64>,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalDelete {
    fn operator_type(&self) -> &str {
        "delete"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        // Collect row indices from input chunks
        let mut rows_to_delete: Vec<u64> = self.row_indices.clone();

        // If input has data, extract row indices from it
        for chunk in &input {
            for row in 0..chunk.size {
                if let Some(field) = chunk.fields.first()
                    && let Some(val) = field.get_i64(row)
                {
                    rows_to_delete.push(val as u64);
                }
            }
        }

        if rows_to_delete.is_empty() {
            // No rows to delete - still return success
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
            v.resize(1);
            v.set_i64(0, 0);
            return Ok(vec![DataChunk::new(vec![v])]);
        }

        // Delete rows from the table
        let mut deleted = 0u64;
        if self.is_node {
            for &row_idx in &rows_to_delete {
                if !self.detach && self.table_catalog.has_incident_edges(self.table_id, row_idx) {
                    return Err(format!(
                        "Cannot delete node {} because it has incident edges (use DETACH DELETE)",
                        row_idx
                    ));
                }
            }
            if self.detach {
                for &row_idx in &rows_to_delete {
                    self.table_catalog.detach_node(self.table_id, row_idx);
                }
            }
            if let Some(mut table) = self.table_catalog.get_node_table_by_name_mut(&self.table_name) {
                for &row_idx in &rows_to_delete {
                    if table.delete_row(row_idx).is_ok() {
                        deleted += 1;
                    }
                }
            } else {
                return Err(format!("Node table '{}' not found for DELETE", self.table_name));
            }
        } else {
            if let Some(mut table) = self.table_catalog.get_rel_table_by_name_mut(&self.table_name) {
                for &edge_idx in &rows_to_delete {
                    if table.delete_edge(edge_idx as usize).is_ok() {
                        deleted += 1;
                    }
                }
            } else {
                return Err(format!("Rel table '{}' not found for DELETE", self.table_name));
            }
        }

        tracing::info!("DELETE: removed {deleted} rows from '{}'", self.table_name);

        let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
        v.resize(1);
        v.set_i64(0, deleted as i64);
        Ok(vec![DataChunk::new(vec![v])])
    }
}

/// Convert an AST Constant to a Value.
pub fn ast_constant_to_value(c: &Constant) -> Value {
    match c {
        Constant::Null => Value::Null,
        Constant::Bool(b) => Value::Bool(*b),
        Constant::Integer(i) => Value::Int64(*i),
        Constant::Float(f) => Value::Double(*f),
        Constant::String(s) => Value::String(s.clone()),
    }
}

