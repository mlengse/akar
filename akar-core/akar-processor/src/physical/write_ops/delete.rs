//! Auto-extracted from physical_operator.rs
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use akar_common::types::{PhysicalTypeID, Value};
use akar_common::vector::{DataChunk, ValueVector};
use akar_parser::ast::Constant;
use akar_storage::table::TableCatalog;
use akar_transaction::UndoRecord;
use std::sync::{Arc, Mutex};

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
    /// Active transaction id — deletes are recorded in `VersionInfo` for MVCC (P52.18).
    pub txn_id: Option<u64>,
    /// Undo sink for rollback records (P52.18).
    pub undo_sink: Option<Arc<Mutex<Vec<UndoRecord>>>>,
}

impl PhysicalOperatorExec for PhysicalDelete {
    fn operator_type(&self) -> &str {
        "delete"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        // Collect row indices from input chunks
        let mut rows_to_delete: Vec<u64> = self.row_indices.clone();

        // If input has data, extract row indices from it.
        // The scan emits the physical row index as the `<alias>._id` column
        // (last column); reading column 0 would treat the first *property*
        // value as a row index (wrong row or an out-of-range no-op).
        for chunk in &input {
            let row_id_col = row_id_column_index(chunk);
            for row in 0..chunk.size {
                if !chunk.fields.is_empty() {
                    if let Some(akar_common::types::Value::Int64(val)) =
                        chunk.get_value(row_id_col.unwrap_or(0), row)
                    {
                        rows_to_delete.push(val as u64);
                    }
                }
            }
        }

        if rows_to_delete.is_empty() {
            // No rows to delete - still return success
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
            v.resize(1);
            v.set_i64(0, 0);
            let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&v).array;
            return Ok(vec![DataChunk::new(vec![arr], vec![PhysicalTypeID::Int64])]);
        }

        // Delete rows from the table
        let mut deleted = 0u64;
        if self.is_node {
            for &row_idx in &rows_to_delete {
                if !self.detach && self.table_catalog.has_incident_edges(self.table_id, row_idx) {
                    return Err(format!(
                        "Cannot delete node {} because it has incident edges (use DETACH DELETE)",
                        row_idx
                    )
                    .into());
                }
            }
            if self.detach {
                for &row_idx in &rows_to_delete {
                    self.table_catalog.detach_node(self.table_id, row_idx);
                }
            }
            if let Some(mut table) = self.table_catalog.get_node_table_by_name_mut(&self.table_name) {
                for &row_idx in &rows_to_delete {
                    // Capture pre-delete row data for rollback (P52.18) and
                    // record the delete in VersionInfo for MVCC isolation.
                    if let Some(sink) = self.undo_sink.as_ref()
                        && let Ok(mut u) = sink.lock()
                    {
                        let old_data = table.row_undo_bytes(row_idx);
                        u.push(UndoRecord::delete(self.table_id, row_idx, old_data));
                    }
                    if table.delete_row_with_txn(row_idx, self.txn_id).is_ok() {
                        deleted += 1;
                    }
                }
            } else {
                return Err(format!("Node table '{}' not found for DELETE", self.table_name).into());
            }
        } else {
            if let Some(mut table) = self.table_catalog.get_rel_table_by_name_mut(&self.table_name) {
                for &edge_idx in &rows_to_delete {
                    if let Some(sink) = self.undo_sink.as_ref()
                        && let Ok(mut u) = sink.lock()
                    {
                        let old_data = table.edge_undo_bytes(edge_idx as usize);
                        u.push(UndoRecord::delete(self.table_id, edge_idx, old_data));
                    }
                    if table.delete_edge(edge_idx as usize).is_ok() {
                        deleted += 1;
                    }
                }
            } else {
                return Err(format!("Rel table '{}' not found for DELETE", self.table_name).into());
            }
        }

        tracing::info!("DELETE: removed {deleted} rows from '{}'", self.table_name);

        let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
        v.resize(1);
        v.set_i64(0, deleted as i64);
        let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&v).array;
        Ok(vec![DataChunk::new(vec![arr], vec![PhysicalTypeID::Int64])])
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

/// Locate the physical row index column in a scan-produced chunk.
///
/// Node scans append an internal node id column (`<alias>._id` = row offset)
/// as the last column of each chunk (see `resolve_scan_arrow_data` and
/// `resolve_scan_data`). Write operators (DELETE/SET) must read row indices
/// from that column; reading column 0 would use the first *property* value
/// as a row index. Falls back to `None` when the chunk carries no `_id`
/// column (e.g. rel table scans), letting callers keep the legacy behaviour.
pub fn row_id_column_index(chunk: &DataChunk) -> Option<usize> {
    chunk
        .field_names
        .iter()
        .position(|n| n == "_id" || n.ends_with("._id"))
}
