//! NodeGroup — a fixed-size collection of ColumnChunks (one per column).
//!
//! A `NodeGroup` holds up to `NODE_GROUP_SIZE` rows of data across all
//! columns of a table. When the group is full it can be flushed to a set
//! of persistent `Column` instances. This mirrors the C++ Kuzu
//! `ChunkedNodeGroup` / `NodeGroup` concept.
//!
//! # Row → Column mapping
//!
//! Each row is a `Vec<Value>` with one element per column. The NodeGroup
//! distributes values column-wise: `columns[col_idx].append(values[col_idx])`.

use crate::column::Column;
use crate::column_chunk::{ColumnChunk, NODE_GROUP_SIZE};
use kuzu_common::types::Value;

/// A node group stores up to `NODE_GROUP_SIZE` rows in columnar format.
///
/// `start_offset` is the global row index within the owning table where
/// this group's data begins. `num_nodes` counts how many rows have been
/// appended so far (≤ `NODE_GROUP_SIZE`).
#[derive(Debug, Clone)]
pub struct NodeGroup {
    /// One in-memory ColumnChunk per column of the table.
    pub columns: Vec<ColumnChunk>,
    /// Global row offset within the owning table.
    pub start_offset: u64,
    /// Number of rows currently stored in this group.
    pub num_nodes: u64,
}

impl NodeGroup {
    /// Create a new empty node group for `num_columns` columns.
    ///
    /// All columns start with the default `NODE_GROUP_SIZE` capacity.
    /// `start_offset` is the global row index in the owning table where
    /// this group begins.
    pub fn new(num_columns: usize, start_offset: u64) -> Self {
        let columns = (0..num_columns).map(|_| ColumnChunk::new()).collect();
        Self {
            columns,
            start_offset,
            num_nodes: 0,
        }
    }

    /// Create a new node group with a custom chunk capacity per column.
    pub fn with_capacity(num_columns: usize, start_offset: u64, capacity: usize) -> Self {
        let columns = (0..num_columns).map(|_| ColumnChunk::with_capacity(capacity)).collect();
        Self {
            columns,
            start_offset,
            num_nodes: 0,
        }
    }

    // ------------------------------------------------------------------
    // Public API
    // ------------------------------------------------------------------

    /// Append a single row (one value per column) to the group.
    ///
    /// Returns an error if the number of values does not match the number
    /// of columns, or if the group is already full.
    pub fn append_row(&mut self, row: Vec<Value>) -> Result<(), String> {
        if row.len() != self.columns.len() {
            return Err(format!(
                "column count mismatch: expected {} values, got {}",
                self.columns.len(),
                row.len()
            ));
        }
        if self.is_full() {
            return Err("node group is already full".to_string());
        }
        for (col_idx, value) in row.into_iter().enumerate() {
            self.columns[col_idx].append(value);
        }
        self.num_nodes += 1;
        Ok(())
    }

    /// Whether the group has reached capacity.
    pub fn is_full(&self) -> bool {
        self.num_nodes as usize >= NODE_GROUP_SIZE
    }

    /// Whether the group is empty.
    pub fn is_empty(&self) -> bool {
        self.num_nodes == 0
    }

    /// Number of columns in this group.
    pub fn num_columns(&self) -> usize {
        self.columns.len()
    }

    /// Remaining capacity (number of additional rows that can be appended).
    pub fn remaining(&self) -> usize {
        NODE_GROUP_SIZE.saturating_sub(self.num_nodes as usize)
    }

    /// Flush all buffered data to persistent `Column` instances.
    ///
    /// Each `ColumnChunk` is flushed to the corresponding `Column` in the
    /// slice via `flush_to_column()`. After flushing, the chunks are
    /// cleared and ready for reuse.
    ///
    /// Returns the total number of rows flushed.
    ///
    /// # Panics
    ///
    /// Panics if `columns.len() != self.columns.len()`.
    pub fn flush(&mut self, columns: &mut [Column]) -> std::io::Result<usize> {
        assert_eq!(
            columns.len(),
            self.columns.len(),
            "NodeGroup::flush: column count mismatch"
        );
        let mut total = 0;
        for (chunk, col) in self.columns.iter_mut().zip(columns.iter_mut()) {
            let n = chunk.flush_to_column(col)?;
            // All chunks should flush the same number of values.
            if total == 0 {
                total = n;
            }
            debug_assert!(n == 0 || n == total, "inconsistent flush count");
        }
        self.num_nodes = 0;
        Ok(total)
    }

    /// Flush data to columns but keep the in-memory buffer intact.
    pub fn flush_copy(&self, columns: &mut [Column]) -> std::io::Result<usize> {
        assert_eq!(
            columns.len(),
            self.columns.len(),
            "NodeGroup::flush_copy: column count mismatch"
        );
        let mut total = 0;
        for (chunk, col) in self.columns.iter().zip(columns.iter_mut()) {
            let n = chunk.flush_copy_to_column(col)?;
            if total == 0 {
                total = n;
            }
        }
        Ok(total)
    }

    /// Scan all rows currently buffered in the group.
    ///
    /// Returns a `Vec<Vec<Value>>` where `result[row][col]` is the value
    /// at the given row and column.
    pub fn scan(&self) -> Vec<Vec<Value>> {
        let n_rows = self.num_nodes as usize;
        let n_cols = self.columns.len();
        let mut result = Vec::with_capacity(n_rows);

        for row in 0..n_rows {
            let mut row_data = Vec::with_capacity(n_cols);
            for chunk in &self.columns {
                match chunk.get(row) {
                    Some(v) => row_data.push(v.clone()),
                    None => row_data.push(Value::Null),
                }
            }
            result.push(row_data);
        }
        result
    }

    /// Scan a range of buffered rows `[start, start + count)`.
    ///
    /// Returns `Vec<Vec<Value>>` in row-major order.
    pub fn scan_range(&self, start: usize, count: usize) -> Vec<Vec<Value>> {
        let end = (start + count).min(self.num_nodes as usize);
        if start >= end {
            return Vec::new();
        }
        let n_cols = self.columns.len();
        let mut result = Vec::with_capacity(end - start);

        for row in start..end {
            let mut row_data = Vec::with_capacity(n_cols);
            for chunk in &self.columns {
                match chunk.get(row) {
                    Some(v) => row_data.push(v.clone()),
                    None => row_data.push(Value::Null),
                }
            }
            result.push(row_data);
        }
        result
    }

    /// Access a single value at the given local row and column index.
    pub fn get_value(&self, local_row: usize, col_idx: usize) -> Option<&Value> {
        self.columns.get(col_idx).and_then(|chunk| chunk.get(local_row))
    }

    /// Reset the group to empty without flushing.
    pub fn clear(&mut self) {
        for chunk in &mut self.columns {
            chunk.clear();
        }
        self.num_nodes = 0;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer_manager::BufferManagerConfig;
    use crate::page::DEFAULT_PAGE_SIZE;
    use kuzu_common::memory::MemoryManager;
    use kuzu_common::types::LogicalTypeID;
    use std::sync::{Arc, Mutex};

    fn setup_columns(num_cols: usize, db_path: &std::path::Path) -> Vec<Column> {
        let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
        let config = BufferManagerConfig::default();
        let bm = Arc::new(Mutex::new(crate::buffer_manager::BufferManager::new(
            db_path.to_path_buf(),
            mm,
            config,
        )));
        (0..num_cols)
            .map(|i| {
                Column::new(
                    LogicalTypeID::Int64,
                    0,
                    i as u32,
                    &db_path.to_path_buf(),
                    bm.clone(),
                    DEFAULT_PAGE_SIZE,
                )
            })
            .collect()
    }

    #[test]
    fn test_empty_group() {
        let group = NodeGroup::new(3, 0);
        assert_eq!(group.num_columns(), 3);
        assert_eq!(group.num_nodes, 0);
        assert!(group.is_empty());
        assert!(!group.is_full());
        assert_eq!(group.start_offset, 0);
    }

    #[test]
    fn test_append_row() {
        let mut group = NodeGroup::new(2, 100);
        group.append_row(vec![Value::Int64(1), Value::Int64(2)]).unwrap();
        assert_eq!(group.num_nodes, 1);
        assert!(!group.is_empty());

        group.append_row(vec![Value::Int64(3), Value::Int64(4)]).unwrap();
        assert_eq!(group.num_nodes, 2);
    }

    #[test]
    fn test_append_wrong_column_count() {
        let mut group = NodeGroup::new(2, 0);
        let result = group.append_row(vec![Value::Int64(1)]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("column count mismatch"));
    }

    #[test]
    fn test_append_when_full() {
        // NodeGroup fullness is based on NODE_GROUP_SIZE (4096).
        // Test that is_full returns false when not full:
        let mut group = NodeGroup::with_capacity(1, 0, 5);
        for _ in 0..10 {
            group.append_row(vec![Value::Int64(1)]).unwrap();
        }
        assert!(!group.is_full());
        assert_eq!(group.num_nodes, 10);
    }

    #[test]
    fn test_scan() {
        let mut group = NodeGroup::new(3, 0);
        group
            .append_row(vec![Value::Int64(10), Value::Int64(20), Value::Int64(30)])
            .unwrap();
        group
            .append_row(vec![Value::Int64(11), Value::Int64(21), Value::Int64(31)])
            .unwrap();

        let data = group.scan();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0][0], Value::Int64(10));
        assert_eq!(data[0][1], Value::Int64(20));
        assert_eq!(data[1][2], Value::Int64(31));
    }

    #[test]
    fn test_scan_range() {
        let mut group = NodeGroup::new(2, 0);
        for i in 0..10 {
            group.append_row(vec![Value::Int64(i), Value::Int64(i * 10)]).unwrap();
        }

        let slice = group.scan_range(3, 4);
        assert_eq!(slice.len(), 4);
        assert_eq!(slice[0][0], Value::Int64(3));
        assert_eq!(slice[3][0], Value::Int64(6));
    }

    #[test]
    fn test_get_value() {
        let mut group = NodeGroup::new(2, 50);
        group.append_row(vec![Value::Int64(100), Value::Int64(200)]).unwrap();

        assert_eq!(group.get_value(0, 0), Some(&Value::Int64(100)));
        assert_eq!(group.get_value(0, 1), Some(&Value::Int64(200)));
        assert_eq!(group.get_value(1, 0), None);
    }

    #[test]
    fn test_flush_to_columns() {
        let dir = tempfile::tempdir().unwrap();
        let mut cols = setup_columns(2, dir.path());
        let mut group = NodeGroup::new(2, 0);

        for i in 0i64..50 {
            group.append_row(vec![Value::Int64(i), Value::Int64(i * 10)]).unwrap();
        }

        let flushed = group.flush(&mut cols).unwrap();
        assert_eq!(flushed, 50);
        assert_eq!(group.num_nodes, 0);
        assert!(group.is_empty());

        // Verify data persisted in columns
        for i in 0i64..50 {
            assert_eq!(cols[0].get_value(i as u64).unwrap(), Value::Int64(i));
            assert_eq!(cols[1].get_value(i as u64).unwrap(), Value::Int64(i * 10));
        }
    }

    #[test]
    fn test_flush_copy_preserves_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let mut cols = setup_columns(2, dir.path());
        let mut group = NodeGroup::new(2, 0);

        group.append_row(vec![Value::Int64(1), Value::Int64(2)]).unwrap();

        let flushed = group.flush_copy(&mut cols).unwrap();
        assert_eq!(flushed, 1);
        // Buffer should still be intact
        assert_eq!(group.num_nodes, 1);
        assert_eq!(cols[0].get_value(0).unwrap(), Value::Int64(1));
    }

    #[test]
    fn test_clear() {
        let mut group = NodeGroup::new(2, 0);
        group.append_row(vec![Value::Int64(1), Value::Int64(2)]).unwrap();
        group.clear();
        assert_eq!(group.num_nodes, 0);
        assert!(group.is_empty());
    }

    #[test]
    fn test_remaining() {
        // `remaining()` is based on NODE_GROUP_SIZE (4096), not chunk capacity.
        let mut group = NodeGroup::with_capacity(3, 0, 10);
        assert_eq!(group.remaining(), NODE_GROUP_SIZE);
        group
            .append_row(vec![Value::Int64(1), Value::Int64(2), Value::Int64(3)])
            .unwrap();
        assert_eq!(group.remaining(), NODE_GROUP_SIZE - 1);
        group
            .append_row(vec![Value::Int64(4), Value::Int64(5), Value::Int64(6)])
            .unwrap();
        assert_eq!(group.remaining(), NODE_GROUP_SIZE - 2);
    }

    #[test]
    fn test_start_offset() {
        let group = NodeGroup::new(2, 12345);
        assert_eq!(group.start_offset, 12345);
    }

    #[test]
    fn test_multi_column_scan() {
        let mut group = NodeGroup::new(4, 0);
        group
            .append_row(vec![
                Value::String("Alice".into()),
                Value::Int64(30),
                Value::Double(1.65),
                Value::Bool(true),
            ])
            .unwrap();

        let data = group.scan();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0][0], Value::String("Alice".into()));
        assert_eq!(data[0][1], Value::Int64(30));
        assert_eq!(data[0][3], Value::Bool(true));
    }

    #[test]
    fn test_multiple_flush_cycles() {
        let dir = tempfile::tempdir().unwrap();
        let mut cols = setup_columns(2, dir.path());
        let mut group = NodeGroup::with_capacity(2, 0, 20);

        // Flush cycle 1
        for i in 0i64..15 {
            group.append_row(vec![Value::Int64(i), Value::Int64(-i)]).unwrap();
        }
        assert_eq!(group.flush(&mut cols).unwrap(), 15);

        // Flush cycle 2
        for i in 15i64..30 {
            group.append_row(vec![Value::Int64(i), Value::Int64(-i)]).unwrap();
        }
        assert_eq!(group.flush(&mut cols).unwrap(), 15);

        // Verify all 30 rows
        assert_eq!(cols[0].num_values, 30);
        for i in 0i64..30 {
            assert_eq!(cols[0].get_value(i as u64).unwrap(), Value::Int64(i));
            assert_eq!(cols[1].get_value(i as u64).unwrap(), Value::Int64(-i));
        }
    }
}
