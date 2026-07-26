//! NodeGroup — a fixed-size collection of ColumnChunks (one per column).
//!
//! A `NodeGroup` holds up to `NODE_GROUP_SIZE` rows of data across all
//! columns of a table. When the group is full it can be flushed to a set
//! of persistent `Column` instances. This mirrors the C++ Akar
//! `ChunkedNodeGroup` / `NodeGroup` concept.
//!
//! # Row → Column mapping
//!
//! Each row is a `Vec<Value>` with one element per column. The NodeGroup
//! distributes values column-wise: `columns[col_idx].append(values[col_idx])`.

use crate::column::Column;
use crate::column_chunk::{ColumnChunk, NODE_GROUP_SIZE};
use crate::spiller::{MultiWayStreamMerge, SpillFile, Spiller};
use crate::version_info::VersionInfo;
use akar_common::error::StorageError;
use akar_common::types::Value;
use std::sync::Arc;

/// A node group stores up to `NODE_GROUP_SIZE` rows in columnar format.
///
/// `start_offset` is the global row index within the owning table where
/// this group's data begins. `num_nodes` counts how many rows have been
/// appended so far (≤ `NODE_GROUP_SIZE`).
///
/// `version_info` tracks MVCC insert/delete visibility for concurrent
/// writers. It is `None` for single-writer mode (backward compat).
///
/// # Disk Spilling
///
/// When `spiller` is set and the memory threshold is exceeded, the group
/// automatically spills its contents to temp files during `append_row()`.
/// After ingestion is complete, call `flush_with_spiller()` instead of
/// `flush()` to merge all spills + final in-memory data into the columns.
#[derive(Debug, Clone)]
pub struct NodeGroup {
    /// One in-memory ColumnChunk per column of the table.
    pub columns: Vec<ColumnChunk>,
    /// Global row offset within the owning table.
    pub start_offset: u64,
    /// Number of rows currently stored in this group.
    pub num_nodes: u64,
    /// Optional MVCC version tracker for this group.
    pub version_info: Option<VersionInfo>,
    /// Optional spiller for disk-based memory management.
    spiller: Option<Arc<Spiller>>,
    /// List of spill files created during append operations.
    spill_files: Vec<SpillFile>,
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
            version_info: None,
            spiller: None,
            spill_files: Vec::new(),
        }
    }

    /// Create a new node group with a custom chunk capacity per column.
    pub fn with_capacity(num_columns: usize, start_offset: u64, capacity: usize) -> Self {
        let columns = (0..num_columns).map(|_| ColumnChunk::with_capacity(capacity)).collect();
        Self {
            columns,
            start_offset,
            num_nodes: 0,
            version_info: None,
            spiller: None,
            spill_files: Vec::new(),
        }
    }

    /// Attach a spiller to this node group for disk-based memory management.
    ///
    /// When a spiller is attached, `append_row()` automatically spills the
    /// current buffer to disk when the memory threshold is exceeded, then
    /// continues appending. Call `flush_with_spiller()` instead of `flush()`
    /// to merge all spill files + final in-memory data.
    pub fn with_spiller(mut self, spiller: Arc<Spiller>) -> Self {
        self.spiller = Some(spiller);
        self
    }

    /// Set the spiller on an existing node group.
    pub fn set_spiller(&mut self, spiller: Arc<Spiller>) {
        self.spiller = Some(spiller);
    }

    /// Enable MVCC version tracking for this node group.
    /// Must be called before any inserts if concurrent writes are expected.
    pub fn enable_version_info(&mut self) {
        if self.version_info.is_none() {
            self.version_info = Some(VersionInfo::new(NODE_GROUP_SIZE));
        }
    }

    // ------------------------------------------------------------------
    // Public API
    // ------------------------------------------------------------------

    /// Append a single row (one value per column) to the group.
    ///
    /// Returns an error if the number of values does not match the number
    /// of columns, or if the group is already full.
    ///
    /// If `txn_id` is `Some(...)`, the insert is recorded in the version
    /// info for MVCC visibility tracking.
    pub fn append_row(&mut self, row: Vec<Value>) -> Result<(), StorageError> {
        self.append_row_with_txn(row, None)
    }

    /// Append a row with an optional transaction ID for MVCC tracking.
    ///
    /// If a spiller is attached and the in-memory data exceeds the configured
    /// memory threshold, the current buffer is automatically spilled to disk
    /// before appending the new row. This keeps memory usage bounded during
    /// large batch operations like `COPY FROM`.
    pub fn append_row_with_txn(&mut self, row: Vec<Value>, txn_id: Option<u64>) -> Result<(), StorageError> {
        if row.len() != self.columns.len() {
            return Err(StorageError::Page(format!(
                "column count mismatch: expected {} values, got {}",
                self.columns.len(),
                row.len()
            )));
        }
        if self.is_full() {
            return Err(StorageError::Page("node group is already full".to_string()));
        }

        // Auto-spill if the memory threshold is exceeded
        if let Some(ref spiller) = self.spiller
            && !self.columns.is_empty()
            && spiller.should_spill(&self.columns[0])
        {
            self.spill_and_clear()?;
        }

        for (col_idx, value) in row.into_iter().enumerate() {
            self.columns[col_idx].append(value);
        }
        // Record insert in version info if MVCC tracking is enabled
        if let Some(ref vi) = self.version_info
            && let Some(txn) = txn_id
        {
            vi.insert(txn, self.num_nodes as u32);
        }
        self.num_nodes += 1;
        Ok(())
    }

    /// Spill all column chunks to disk and reset the group to empty.
    ///
    /// The spill file is tracked so that `flush_with_spiller()` can later
    /// merge all spilled data back into the persistent columns.
    pub fn spill_and_clear(&mut self) -> Result<(), StorageError> {
        let spiller = self
            .spiller
            .as_ref()
            .ok_or_else(|| StorageError::Spiller("No spiller attached to NodeGroup".to_string()))?;

        if self.is_empty() {
            return Ok(());
        }

        let spill = spiller.spill_columns(&mut self.columns)?;
        if let Some(sf) = spill {
            self.spill_files.push(sf);
        }
        self.num_nodes = 0;
        Ok(())
    }

    /// Flush all data to persistent columns, merging any spilled data.
    ///
    /// This is the spill-aware alternative to `flush()`. It merges all
    /// previously spilled files + the current in-memory buffer into the
    /// target columns using a streaming merge. If no spilling occurred,
    /// this falls back to the regular `flush()`.
    ///
    /// The optional `sort_key_column` is the column index to use for
    /// merge ordering and PK deduplication. Pass `None` for unordered
    /// append (no dedup).
    pub fn flush_with_spiller(
        &mut self,
        columns: &mut [Column],
        sort_key_column: Option<usize>,
        dedup: bool,
    ) -> std::io::Result<usize> {
        if self.spill_files.is_empty() {
            // No spilling occurred — regular flush
            return self.flush(columns);
        }

        assert_eq!(
            columns.len(),
            self.columns.len(),
            "NodeGroup::flush_with_spiller: column count mismatch"
        );

        // Capture in-memory rows before clearing
        let in_memory_rows = self.scan();
        self.clear();

        // Build the merger
        let sort_col = sort_key_column.unwrap_or(0);
        let mut merger = MultiWayStreamMerge::new(&self.spill_files, Some(in_memory_rows), sort_col, dedup)
            .map_err(std::io::Error::other)?;

        // Stream all merged rows into the target columns
        let mut total: usize = 0;
        while let Some(row) = merger.next_tuple() {
            for (col_idx, value) in row.into_iter().enumerate() {
                if col_idx < columns.len() {
                    columns[col_idx].append_value(&value)?;
                }
            }
            total += 1;
        }

        // Clean up spill files
        if let Some(ref spiller) = self.spiller {
            let files = std::mem::take(&mut self.spill_files);
            for sf in &files {
                let _ = spiller.cleanup(sf);
            }
        }

        Ok(total)
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

    /// Access a single value with MVCC snapshot isolation.
    ///
    /// Checks `VersionInfo` for insert/delete visibility first. If the row
    /// is not visible at `snapshot_ts`, returns `None`. Then checks
    /// `UpdateInfo` version chain on the column chunk for versioned updates.
    pub fn get_value_with_snapshot(
        &self,
        local_row: usize,
        col_idx: usize,
        snapshot_ts: Option<u64>,
        commit_history: &[(u64, u64)],
    ) -> Option<&Value> {
        // Check version info visibility (inserts/deletes)
        if let Some(ts) = snapshot_ts
            && !self.is_row_visible(local_row, ts, commit_history)
        {
            return None;
        }
        // Get value with update version chain check
        self.columns
            .get(col_idx)
            .and_then(|chunk| chunk.get_value_with_snapshot(local_row, snapshot_ts, commit_history))
    }

    /// Access a single value with MVCC snapshot isolation (owned variant).
    ///
    /// Like `get_value_with_snapshot` but returns `Option<Value>` instead of
    /// `Option<&Value>`, enabling proper version chain traversal with
    /// deserialized old values from `UpdateInfo`.
    pub fn get_value_owned_with_snapshot(
        &self,
        local_row: usize,
        col_idx: usize,
        snapshot_ts: Option<u64>,
        commit_history: &[(u64, u64)],
    ) -> Option<Value> {
        // Check version info visibility (inserts/deletes)
        if let Some(ts) = snapshot_ts
            && !self.is_row_visible(local_row, ts, commit_history)
        {
            return None;
        }
        // Get value with update version chain check (owned)
        self.columns
            .get(col_idx)
            .and_then(|chunk| chunk.get_value_owned_with_snapshot(local_row, snapshot_ts, commit_history))
    }

    /// Check whether a row is visible at the given snapshot timestamp.
    /// Returns `true` if no version tracking is active (backward compat).
    pub fn is_row_visible(&self, local_row: usize, snapshot_ts: u64, commit_history: &[(u64, u64)]) -> bool {
        match &self.version_info {
            Some(vi) => vi.is_visible(local_row as u32, snapshot_ts, commit_history),
            None => true, // No version tracking → always visible
        }
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
    use akar_common::memory::MemoryManager;
    use akar_common::types::LogicalTypeID;
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
                    db_path,
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
        assert!(result.unwrap_err().to_string().contains("column count mismatch"));
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
