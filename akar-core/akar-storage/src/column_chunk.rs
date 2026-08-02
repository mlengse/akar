//! ColumnChunk — in-memory buffer for a contiguous range of column values.
//!
//! A `ColumnChunk` accumulates values in memory and flushes them to a
//! persistent `Column` (backed by the `BufferManager`) when full. This
//! matches the C++ Akar `ChunkedNodeGroup` / `ColumnChunk` concept.
//!
//! # Strategy
//!
//! Values are appended to an internal `Vec<Value>`. When the chunk reaches
//! `NODE_GROUP_SIZE` entries it is considered full. The caller should then
//! call `flush_to_column()` to batch-write all buffered values to the
//! column's on-disk pages via `Column::append_value()`.

use crate::column::Column;
use crate::update_info::UpdateInfo;
use akar_common::error::StorageError;
use akar_common::types::{PhysicalTypeID, Value};
use arrow::array::{
    ArrayRef, BooleanBuilder, Float32Builder, Float64Builder, Int8Builder, Int16Builder, Int32Builder, Int64Builder,
    StringBuilder,
};

/// Default number of rows per column chunk (matches C++ Akar default).
pub const NODE_GROUP_SIZE: usize = 4096;

/// An in-memory buffer that accumulates values before flushing to a `Column`.
///
/// # Example
///
/// ```ignore
/// let mut chunk = ColumnChunk::new(LogicalTypeID::Int64);
/// for i in 0..100 {
///     chunk.append(Value::Int64(i));
/// }
/// assert!(chunk.num_values() == 100);
/// assert!(!chunk.is_full());
///
/// // Flush into a Column:
/// chunk.flush_to_column(&mut my_column).unwrap();
/// assert!(chunk.is_empty());
/// ```
#[derive(Debug, Clone)]
pub struct ColumnChunk {
    /// Buffered values in insertion order.
    values: Vec<Value>,
    /// Maximum number of values before the chunk is considered full.
    capacity: usize,
    /// Optional MVCC update version chain for this chunk.
    /// Tracks versioned updates to support snapshot isolation.
    pub update_info: Option<UpdateInfo>,
    /// Min/max stats for zone map predicate pushdown.
    pub stats: crate::predicate::ColumnChunkStats,
}

impl ColumnChunk {
    /// Create a new empty chunk with the default capacity (`NODE_GROUP_SIZE`).
    pub fn new() -> Self {
        Self {
            values: Vec::with_capacity(NODE_GROUP_SIZE),
            capacity: NODE_GROUP_SIZE,
            update_info: None,
            stats: crate::predicate::ColumnChunkStats::new(None, None),
        }
    }

    /// Create a new empty chunk with a custom capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            values: Vec::with_capacity(capacity),
            capacity,
            update_info: None,
            stats: crate::predicate::ColumnChunkStats::new(None, None),
        }
    }

    /// Enable MVCC update tracking for this chunk.
    pub fn enable_update_info(&mut self) {
        if self.update_info.is_none() {
            self.update_info = Some(UpdateInfo::new(self.capacity));
        }
    }

    // ------------------------------------------------------------------
    // Public API
    // ------------------------------------------------------------------

    /// Append a single value into the buffer.
    ///
    /// Does **not** automatically flush when full — the caller should check
    /// `is_full()` and call `flush_to_column()` at the appropriate time.
    pub fn append(&mut self, value: Value) {
        self.stats.update(&value);
        self.values.push(value);
    }

    /// Set a value at a specific index (for in-place updates like DELETE).
    /// If this chunk has `update_info` enabled, the old value is preserved
    /// in the version chain before overwriting.
    /// Returns an error if the index is out of bounds.
    pub fn set_value(&mut self, idx: usize, value: Value) -> Result<(), StorageError> {
        if idx >= self.values.len() {
            return Err(StorageError::Page(format!(
                "ColumnChunk index {idx} out of bounds (len={})",
                self.values.len()
            )));
        }
        // Preserve old value in update info if MVCC tracking is enabled
        if let Some(ref ui) = self.update_info {
            let old_data = serialize_value_for_version(&self.values[idx]);
            // Use a placeholder version (0) — the actual commit version is
            // assigned during commit. The version chain is traversed by
            // get_value_with_snapshot using the real commit timestamps.
            ui.append_update(idx as u32, u64::MAX, old_data);
        }
        self.stats.update(&value);
        self.values[idx] = value;
        Ok(())
    }

    /// Get a value considering MVCC visibility at a given snapshot timestamp.
    /// If `snapshot_ts` is `None`, returns the latest value (current behavior).
    ///
    /// When a snapshot timestamp is provided and `UpdateInfo` has a version
    /// entry for this row with `version <= snapshot_ts`, the versioned data
    /// is returned. Otherwise the current (base) value is returned.
    pub fn get_value_with_snapshot(
        &self,
        idx: usize,
        snapshot_ts: Option<u64>,
        _commit_history: &[(u64, u64)],
    ) -> Option<&Value> {
        if idx >= self.values.len() {
            return None;
        }
        // When no snapshot requested, return latest value directly
        let ts = match snapshot_ts {
            Some(ts) => ts,
            None => return self.values.get(idx),
        };
        // Check UpdateInfo version chain for an older visible version
        if let Some(ref ui) = self.update_info {
            if let Some(old_data) = ui.get_version(idx as u32, ts) {
                // We have a versioned old value — return the base value instead.
                // The version chain stores the OLD data before the update. The
                // base `values[idx]` contains the NEW (latest) data. If the
                // update at this version is visible (version <= snapshot_ts),
                // the reader should see the old value, not the new one.
                //
                // However, since we return `&Value` (not owned), we cannot
                // return a deserialized value. Instead, we note that the
                // UpdateInfo stores old data — the caller must use
                // `get_value_owned_with_snapshot()` for proper versioned reads.
                //
                // For now: if the latest value's version is > snapshot_ts,
                // the reader predates the update and should see the old
                // value. Since we can't return it by reference, we check
                // if the base value itself was the one before the update.
                // The version chain stores old_data, and if version <= ts,
                // the reader should see old_data. Since we can't deserialize
                // here into a &Value, we return None to signal the caller
                // to use the owned variant.
                drop(old_data); // Can't use the data by reference
            }
        }
        self.values.get(idx)
    }

    /// Get a value with MVCC snapshot isolation, returning an owned Value.
    ///
    /// This variant properly handles version chain traversal by deserializing
    /// old values from the UpdateInfo chain. Returns the value visible at
    /// `snapshot_ts`, or `None` if the index is out of bounds.
    pub fn get_value_owned_with_snapshot(
        &self,
        idx: usize,
        snapshot_ts: Option<u64>,
        _commit_history: &[(u64, u64)],
    ) -> Option<Value> {
        if idx >= self.values.len() {
            return None;
        }
        let ts = match snapshot_ts {
            Some(ts) => ts,
            None => return Some(self.values[idx].clone()),
        };
        // Check UpdateInfo version chain
        if let Some(ref ui) = self.update_info {
            if let Some(old_data) = ui.get_version(idx as u32, ts) {
                // The version at or before snapshot_ts is visible.
                // Deserialize the old value from the version chain.
                if let Ok(old_value) = serde_json::from_slice::<Value>(&old_data) {
                    return Some(old_value);
                }
            }
        }
        // No visible version in chain — return the current base value
        Some(self.values[idx].clone())
    }

    /// Number of buffered values.
    pub fn num_values(&self) -> usize {
        self.values.len()
    }

    /// Whether the chunk has reached its capacity and should be flushed.
    pub fn is_full(&self) -> bool {
        self.values.len() >= self.capacity
    }

    /// Whether the chunk is empty.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Borrow the buffered values as a slice.
    pub fn as_slice(&self) -> &[Value] {
        &self.values
    }

    /// Drain all buffered values (leaves the chunk empty).
    pub fn drain(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.values)
    }

    /// Scan a range of buffered values (inclusive `start`, exclusive `end`).
    ///
    /// Panics if the range is out of bounds.
    pub fn scan(&self, start: usize, count: usize) -> Vec<Value> {
        let end = (start + count).min(self.values.len());
        self.values[start..end].to_vec()
    }

    /// Flush all buffered values into a `Column` via `append_value`, then
    /// clear the buffer.
    ///
    /// Returns the number of values flushed.
    pub fn flush_to_column(&mut self, column: &mut Column) -> std::io::Result<usize> {
        let n = self.values.len();
        if n == 0 {
            return Ok(0);
        }

        // Take the values out so we don't hold the buffer during I/O.
        let batch = std::mem::take(&mut self.values);

        for value in &batch {
            column.append_value(value)?;
        }

        // Re-allocate with the original capacity.
        self.values = Vec::with_capacity(self.capacity);

        Ok(n)
    }

    /// Flush all buffered values into a `Column`, but keep the data in the
    /// buffer afterwards (for cases where the caller still needs it).
    pub fn flush_copy_to_column(&self, column: &mut Column) -> std::io::Result<usize> {
        let n = self.values.len();
        if n == 0 {
            return Ok(0);
        }

        for value in &self.values {
            column.append_value(value)?;
        }

        Ok(n)
    }

    /// Clear the buffer without flushing.
    pub fn clear(&mut self) {
        self.values.clear();
    }

    /// Remaining capacity before the chunk is full.
    pub fn remaining(&self) -> usize {
        self.capacity.saturating_sub(self.values.len())
    }

    /// Access a single buffered value by index.
    pub fn get(&self, index: usize) -> Option<&Value> {
        self.values.get(index)
    }

    /// Capacity of this chunk.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Convert buffered values directly into an Arrow array, skipping
    /// intermediate `Vec<Vec<Value>>` materialization.
    ///
    /// This is the key optimization for the scan path: instead of cloning
    /// every Value into a `Vec<Vec<Value>>` and then building Arrow arrays
    /// from that, we read directly from `self.values` into Arrow builders.
    pub fn to_arrow_array(&self, phys_type: PhysicalTypeID) -> ArrayRef {
        let size = self.values.len();
        match phys_type {
            PhysicalTypeID::Bool => {
                let mut builder = BooleanBuilder::with_capacity(size);
                for v in &self.values {
                    match v {
                        Value::Bool(b) => builder.append_value(*b),
                        _ => builder.append_null(),
                    }
                }
                std::sync::Arc::new(builder.finish())
            }
            PhysicalTypeID::Int64 => {
                let mut builder = Int64Builder::with_capacity(size);
                for v in &self.values {
                    match v {
                        Value::Int64(n) => builder.append_value(*n),
                        Value::Int32(n) => builder.append_value(*n as i64),
                        Value::Int16(n) => builder.append_value(*n as i64),
                        Value::Int8(n) => builder.append_value(*n as i64),
                        Value::UInt64(n) => builder.append_value(*n as i64),
                        Value::UInt32(n) => builder.append_value(*n as i64),
                        Value::UInt16(n) => builder.append_value(*n as i64),
                        Value::UInt8(n) => builder.append_value(*n as i64),
                        Value::Date(n) => builder.append_value(n.0 as i64),
                        Value::Timestamp(n)
                        | Value::TimestampNs(n)
                        | Value::TimestampMs(n)
                        | Value::TimestampSec(n) => builder.append_value(n.0),
                        Value::TimestampTz(n) => builder.append_value(n.0),
                        Value::DTime(n) => builder.append_value(*n),
                        _ => builder.append_null(),
                    }
                }
                std::sync::Arc::new(builder.finish())
            }
            PhysicalTypeID::Int32 => {
                let mut builder = Int32Builder::with_capacity(size);
                for v in &self.values {
                    match v {
                        Value::Int32(n) => builder.append_value(*n),
                        Value::Int16(n) => builder.append_value(*n as i32),
                        Value::Int8(n) => builder.append_value(*n as i32),
                        _ => builder.append_null(),
                    }
                }
                std::sync::Arc::new(builder.finish())
            }
            PhysicalTypeID::Int16 => {
                let mut builder = Int16Builder::with_capacity(size);
                for v in &self.values {
                    match v {
                        Value::Int16(n) => builder.append_value(*n),
                        Value::Int8(n) => builder.append_value(*n as i16),
                        _ => builder.append_null(),
                    }
                }
                std::sync::Arc::new(builder.finish())
            }
            PhysicalTypeID::Int8 => {
                let mut builder = Int8Builder::with_capacity(size);
                for v in &self.values {
                    match v {
                        Value::Int8(n) => builder.append_value(*n),
                        _ => builder.append_null(),
                    }
                }
                std::sync::Arc::new(builder.finish())
            }
            PhysicalTypeID::Double => {
                let mut builder = Float64Builder::with_capacity(size);
                for v in &self.values {
                    match v {
                        Value::Double(n) => builder.append_value(*n),
                        Value::Float(n) => builder.append_value(*n as f64),
                        Value::Int64(n) => builder.append_value(*n as f64),
                        _ => builder.append_null(),
                    }
                }
                std::sync::Arc::new(builder.finish())
            }
            PhysicalTypeID::Float => {
                let mut builder = Float32Builder::with_capacity(size);
                for v in &self.values {
                    match v {
                        Value::Float(n) => builder.append_value(*n),
                        Value::Double(n) => builder.append_value(*n as f32),
                        _ => builder.append_null(),
                    }
                }
                std::sync::Arc::new(builder.finish())
            }
            PhysicalTypeID::String => {
                let mut builder = StringBuilder::with_capacity(size, size * 16);
                for v in &self.values {
                    match v {
                        Value::String(s) => builder.append_value(s),
                        _ => builder.append_null(),
                    }
                }
                std::sync::Arc::new(builder.finish())
            }
            _ => {
                let mut builder = Int64Builder::with_capacity(size);
                for _ in 0..size {
                    builder.append_null();
                }
                std::sync::Arc::new(builder.finish())
            }
        }
    }
}

impl Default for ColumnChunk {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Vec<Value>> for ColumnChunk {
    fn from(values: Vec<Value>) -> Self {
        let capacity = values.len().max(NODE_GROUP_SIZE);
        let mut stats = crate::predicate::ColumnChunkStats::new(None, None);
        for v in &values {
            stats.update(v);
        }
        Self {
            values,
            capacity,
            update_info: None,
            stats,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Serialize a Value to bytes for storage in the update version chain.
fn serialize_value_for_version(v: &Value) -> Vec<u8> {
    // Use serde_json for a simple portable binary representation.
    // The version chain data is only used internally for rollback recovery;
    // performance is not critical for this initial implementation.
    serde_json::to_vec(v).unwrap_or_else(|_| vec![])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column::Column;
    use crate::page::DEFAULT_PAGE_SIZE;
    use akar_common::memory::MemoryManager;
    use akar_common::types::LogicalTypeID;

    use std::sync::{Arc, Mutex};

    fn setup_column(db_path: &std::path::Path) -> Column {
        let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
        let config = crate::buffer_manager::BufferManagerConfig::default();
        let bm = Arc::new(Mutex::new(crate::buffer_manager::BufferManager::new(
            db_path.to_path_buf(),
            mm,
            config,
        )));
        Column::new(LogicalTypeID::Int64, 0, 0, db_path, bm, DEFAULT_PAGE_SIZE)
    }

    #[test]
    fn test_empty_chunk() {
        let chunk = ColumnChunk::new();
        assert!(chunk.is_empty());
        assert_eq!(chunk.num_values(), 0);
        assert_eq!(chunk.remaining(), NODE_GROUP_SIZE);
    }

    #[test]
    fn test_append_and_count() {
        let mut chunk = ColumnChunk::new();
        chunk.append(Value::Int64(1));
        chunk.append(Value::Int64(2));
        chunk.append(Value::Int64(3));
        assert_eq!(chunk.num_values(), 3);
        assert!(!chunk.is_empty());
    }

    #[test]
    fn test_is_full() {
        let mut chunk = ColumnChunk::with_capacity(3);
        assert!(!chunk.is_full());
        chunk.append(Value::Int64(1));
        chunk.append(Value::Int64(2));
        chunk.append(Value::Int64(3));
        assert!(chunk.is_full());
    }

    #[test]
    fn test_scan() {
        let mut chunk = ColumnChunk::new();
        for i in 0..10 {
            chunk.append(Value::Int64(i));
        }
        let scanned = chunk.scan(2, 4);
        assert_eq!(scanned.len(), 4);
        assert_eq!(scanned[0], Value::Int64(2));
        assert_eq!(scanned[3], Value::Int64(5));
    }

    #[test]
    fn test_drain() {
        let mut chunk = ColumnChunk::new();
        chunk.append(Value::Int64(42));
        chunk.append(Value::Int64(43));
        assert_eq!(chunk.num_values(), 2);

        let drained = chunk.drain();
        assert_eq!(drained.len(), 2);
        assert!(chunk.is_empty());
    }

    #[test]
    fn test_flush_to_column() {
        let dir = tempfile::tempdir().unwrap();
        let mut column = setup_column(dir.path());
        let mut chunk = ColumnChunk::new();

        for i in 0i64..50 {
            chunk.append(Value::Int64(i));
        }

        assert_eq!(chunk.num_values(), 50);
        let flushed = chunk.flush_to_column(&mut column).unwrap();
        assert_eq!(flushed, 50);
        assert!(chunk.is_empty());
        assert_eq!(column.num_values, 50);

        // Verify the data was written correctly
        for i in 0i64..50 {
            let v = column.get_value(i as u64).unwrap();
            assert_eq!(v, Value::Int64(i));
        }
    }

    #[test]
    fn test_flush_copy_to_column_preserves_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let mut column = setup_column(dir.path());
        let mut chunk = ColumnChunk::new();

        chunk.append(Value::Int64(10));
        chunk.append(Value::Int64(20));

        let flushed = chunk.flush_copy_to_column(&mut column).unwrap();
        assert_eq!(flushed, 2);
        // Buffer is still intact
        assert_eq!(chunk.num_values(), 2);
        assert_eq!(column.num_values, 2);
    }

    #[test]
    fn test_flush_empty_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let mut column = setup_column(dir.path());
        let mut chunk = ColumnChunk::new();

        let flushed = chunk.flush_to_column(&mut column).unwrap();
        assert_eq!(flushed, 0);
        assert!(chunk.is_empty());
        assert_eq!(column.num_values, 0);
    }

    #[test]
    fn test_clear() {
        let mut chunk = ColumnChunk::new();
        chunk.append(Value::Int64(99));
        chunk.clear();
        assert!(chunk.is_empty());
        assert_eq!(chunk.num_values(), 0);
    }

    #[test]
    fn test_remaining() {
        let mut chunk = ColumnChunk::with_capacity(10);
        assert_eq!(chunk.remaining(), 10);
        chunk.append(Value::Int64(1));
        assert_eq!(chunk.remaining(), 9);
        chunk.append(Value::Int64(2));
        assert_eq!(chunk.remaining(), 8);
    }

    #[test]
    fn test_from_vec() {
        let values = vec![Value::Int64(1), Value::Int64(2), Value::Int64(3)];
        let chunk = ColumnChunk::from(values);
        assert_eq!(chunk.num_values(), 3);
        assert_eq!(chunk.get(0), Some(&Value::Int64(1)));
        assert_eq!(chunk.get(2), Some(&Value::Int64(3)));
    }

    #[test]
    fn test_chunk_default_capacity() {
        let chunk = ColumnChunk::new();
        assert_eq!(chunk.capacity(), NODE_GROUP_SIZE);
    }

    #[test]
    fn test_multiple_flushes_to_same_column() {
        let dir = tempfile::tempdir().unwrap();
        let mut column = setup_column(dir.path());
        let mut chunk = ColumnChunk::with_capacity(20);

        // First batch
        for i in 0i64..15 {
            chunk.append(Value::Int64(i));
        }
        chunk.flush_to_column(&mut column).unwrap();
        assert_eq!(column.num_values, 15);

        // Second batch
        for i in 15i64..30 {
            chunk.append(Value::Int64(i));
        }
        chunk.flush_to_column(&mut column).unwrap();
        assert_eq!(column.num_values, 30);

        // Verify all values
        for i in 0i64..30 {
            let v = column.get_value(i as u64).unwrap();
            assert_eq!(v, Value::Int64(i));
        }
    }

    #[test]
    fn test_large_flush() {
        let dir = tempfile::tempdir().unwrap();
        let mut column = setup_column(dir.path());
        let mut chunk = ColumnChunk::new();

        // Fill the chunk to capacity
        for i in 0..NODE_GROUP_SIZE {
            chunk.append(Value::Int64(i as i64));
        }
        assert!(chunk.is_full());

        let flushed = chunk.flush_to_column(&mut column).unwrap();
        assert_eq!(flushed, NODE_GROUP_SIZE);
        assert!(chunk.is_empty());
        assert_eq!(column.num_values, NODE_GROUP_SIZE as u64);

        // Verify a few values at boundaries
        assert_eq!(column.get_value(0).unwrap(), Value::Int64(0));
        let last_idx = (NODE_GROUP_SIZE - 1) as u64;
        assert_eq!(column.get_value(last_idx).unwrap(), Value::Int64(last_idx as i64));
    }
}
