//! Local storage — per-transaction write buffer before commit.
//!
//! During a transaction, write operations are buffered in `LocalStorage`.
//! On commit, `flush_to_tables()` applies buffered inserts, deletes, and
//! updates to the actual `NodeTable`/`RelTable` via the `TableCatalog`.
//! On rollback, `clear()` discards all buffers.

use crate::table::{NodeTable, RelTable, TableCatalog};
use akar_common::error::StorageError;
use akar_transaction::UndoRecord;
use std::collections::HashMap;
use std::sync::Arc;

/// A local table insert/update buffer for an in-progress transaction.
///
/// Stores rows as serialised byte vectors (compatible with the `Value` binary
/// format used by `Column`). The `flush_to_tables()` method deserialises and
/// applies them.
#[derive(Debug, Default)]
pub struct LocalTableData {
    inserted_rows: Vec<Vec<u8>>,
    deleted_row_ids: Vec<u64>,
    updated_rows: HashMap<u64, Vec<u8>>,
}

impl LocalTableData {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, row_data: Vec<u8>) {
        self.inserted_rows.push(row_data);
    }

    pub fn delete(&mut self, row_id: u64) {
        self.deleted_row_ids.push(row_id);
    }

    pub fn update(&mut self, row_id: u64, row_data: Vec<u8>) {
        self.updated_rows.insert(row_id, row_data);
    }

    pub fn inserted_rows(&self) -> &[Vec<u8>] {
        &self.inserted_rows
    }

    pub fn deleted_row_ids(&self) -> &[u64] {
        &self.deleted_row_ids
    }

    pub fn updated_rows(&self) -> &HashMap<u64, Vec<u8>> {
        &self.updated_rows
    }

    /// Number of buffered mutations.
    pub fn len(&self) -> usize {
        self.inserted_rows.len() + self.deleted_row_ids.len() + self.updated_rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inserted_rows.is_empty() && self.deleted_row_ids.is_empty() && self.updated_rows.is_empty()
    }

    /// Flush this table's buffered data to a `NodeTable`, returning undo records.
    ///
    /// Deserialises each buffered row from binary format and calls
    /// `node_table.insert_row_with_txn()`.
    /// When `txn_id` is `Some(...)`, inserts/deletes are recorded in VersionInfo.
    /// Returns undo records so the caller can store them for potential rollback.
    pub fn flush_to_node_table(
        &self,
        table_id: u64,
        node_table: &mut NodeTable,
        txn_id: Option<u64>,
    ) -> Result<Vec<UndoRecord>, StorageError> {
        use crate::column::Column;
        let mut undo_records = Vec::new();

        for row_bytes in &self.inserted_rows {
            let values = crate::deserialize_values_from_bytes(row_bytes, node_table.columns.len());
            let row_id = node_table.insert_row_with_txn(values, txn_id)?;
            if txn_id.is_some() {
                undo_records.push(UndoRecord::insert(table_id, row_id));
            }
        }

        for row_id in &self.deleted_row_ids {
            let num_cols = node_table.columns.len();
            let mut old_row_data = Vec::new();
            for col_idx in 0..num_cols {
                let val = node_table
                    .get_value(*row_id as usize, col_idx)
                    .cloned()
                    .unwrap_or(akar_common::types::Value::Null);
                old_row_data.extend_from_slice(&Column::serialize_value(&val));
            }
            node_table.delete_row_with_txn(*row_id, txn_id)?;
            if txn_id.is_some() {
                undo_records.push(UndoRecord::delete(table_id, *row_id, old_row_data));
            }
        }

        for (row_id, row_bytes) in &self.updated_rows {
            let values = crate::deserialize_values_from_bytes(row_bytes, 1);
            if let Some(val) = values.into_iter().next() {
                node_table.update_cell(*row_id, 0, val)?;
            }
        }

        Ok(undo_records)
    }

    /// Flush this table's buffered data to a `RelTable`.
    pub fn flush_to_rel_table(&self, rel_table: &mut RelTable) -> Result<(), StorageError> {
        for row_bytes in &self.inserted_rows {
            let values = crate::deserialize_values_from_bytes(row_bytes, rel_table.columns.len());
            rel_table.insert_row(values)?;
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.inserted_rows.clear();
        self.deleted_row_ids.clear();
        self.updated_rows.clear();
    }
}

/// Per-transaction local storage for all modified tables.
#[derive(Debug, Default)]
pub struct LocalStorage {
    tables: HashMap<u64, LocalTableData>,
}

impl LocalStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_or_create_table(&mut self, table_id: u64) -> &mut LocalTableData {
        self.tables.entry(table_id).or_default()
    }

    pub fn get_table(&self, table_id: u64) -> Option<&LocalTableData> {
        self.tables.get(&table_id)
    }

    /// Number of tables with buffered data.
    pub fn len(&self) -> usize {
        self.tables.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    /// Flush all buffered writes to the actual tables via the `TableCatalog`.
    ///
    /// Called on commit. After a successful flush, the transaction's writes
    /// are visible to subsequent transactions.
    /// When `txn_id` is `Some(...)`, inserts/deletes are recorded in VersionInfo.
    /// Returns undo records for potential rollback.
    pub fn flush_to_tables(
        &self,
        catalog: &Arc<TableCatalog>,
        txn_id: Option<u64>,
    ) -> Result<Vec<UndoRecord>, StorageError> {
        let mut all_undo_records = Vec::new();
        for (&table_id, table_data) in &self.tables {
            if table_data.is_empty() {
                continue;
            }
            if let Some(mut node_table) = catalog.get_node_table_mut(table_id) {
                let undo_records = table_data.flush_to_node_table(table_id, &mut node_table, txn_id)?;
                all_undo_records.extend(undo_records);
            } else if let Some(mut rel_table) = catalog.get_rel_table_mut(table_id) {
                table_data.flush_to_rel_table(&mut rel_table)?;
            }
        }
        Ok(all_undo_records)
    }

    /// Clear all buffered data (called on rollback).
    pub fn clear(&mut self) {
        self.tables.clear();
    }
}
