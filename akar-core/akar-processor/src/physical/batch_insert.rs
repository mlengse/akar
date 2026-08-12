//! PhysicalBatchInsert — dedicated batch insert operator.
//!
//! Wraps `NodeTable::insert_rows_batch()` / `RelTable::insert_rels_batch()`
//! for use in query plans where multiple CREATE statements can be fused
//! into a single efficient batch operation.

use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use akar_common::types::{PhysicalTypeID, Value};
use akar_common::vector::{DataChunk, ValueVector};
use akar_storage::table::TableCatalog;
use akar_transaction::UndoRecord;
use std::sync::{Arc, Mutex};

/// Physical operator for BATCH INSERT — inserts pre-collected rows/rels
/// into a table using batch APIs for maximum throughput.
pub struct PhysicalBatchInsert {
    pub table_name: String,
    pub table_id: u64,
    /// Rows to insert: each row is a Vec<Value> matching column order.
    pub rows: Vec<Vec<Value>>,
    pub table_catalog: Arc<TableCatalog>,
    /// Active transaction id (P52.18).
    pub txn_id: Option<u64>,
    /// Undo sink for rollback records (P52.18).
    pub undo_sink: Option<Arc<Mutex<Vec<UndoRecord>>>>,
}

impl PhysicalOperatorExec for PhysicalBatchInsert {
    fn operator_type(&self) -> &str {
        "batch_insert"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        let num_rows = self.rows.len();
        if num_rows == 0 {
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
            v.resize(1);
            v.set_i64(0, 0);
            let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&v).array;
            return Ok(vec![DataChunk::new(vec![arr], vec![PhysicalTypeID::Int64])]);
        }

        // Try node table first, then rel table
        if let Some(mut table) = self.table_catalog.get_node_table_by_name_mut(&self.table_name) {
            let start = table.num_rows;
            let count = table
                .insert_rows_batch_with_txn(&self.rows, self.txn_id)
                .map_err(|e| format!("BatchInsert node error: {e}"))?;
            if let Some(sink) = self.undo_sink.as_ref()
                && let Ok(mut u) = sink.lock()
            {
                for row in start..start + count {
                    u.push(UndoRecord::insert(self.table_id, row));
                }
            }
            tracing::info!(
                "BATCH INSERT: inserted {count} rows into node table '{}'",
                self.table_name
            );
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
            v.resize(1);
            v.set_i64(0, count as i64);
            let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&v).array;
            return Ok(vec![DataChunk::new(vec![arr], vec![PhysicalTypeID::Int64])]);
        }

        if let Some(mut table) = self.table_catalog.get_rel_table_by_name_mut(&self.table_name) {
            let rels: Vec<(u64, u64, Vec<Value>)> = self
                .rows
                .iter()
                .map(|row| {
                    let from = match &row[0] {
                        Value::Int64(v) => *v as u64,
                        _ => 0,
                    };
                    let to = match &row[1] {
                        Value::Int64(v) => *v as u64,
                        _ => 0,
                    };
                    let props = row[2..].to_vec();
                    (from, to, props)
                })
                .collect();
            let start = table.edges.len();
            let count = table
                .insert_rels_batch(&rels)
                .map_err(|e| format!("BatchInsert rel error: {e}"))?;
            if let Some(sink) = self.undo_sink.as_ref()
                && let Ok(mut u) = sink.lock()
            {
                for idx in start..start + count as usize {
                    u.push(UndoRecord::insert(self.table_id, idx as u64));
                }
            }
            tracing::info!(
                "BATCH INSERT: inserted {count} rels into rel table '{}'",
                self.table_name
            );
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
            v.resize(1);
            v.set_i64(0, count as i64);
            let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&v).array;
            return Ok(vec![DataChunk::new(vec![arr], vec![PhysicalTypeID::Int64])]);
        }

        Err(format!(
            "Table '{}' not found in storage catalog for BatchInsert",
            self.table_name
        )
        .into())
    }
}
