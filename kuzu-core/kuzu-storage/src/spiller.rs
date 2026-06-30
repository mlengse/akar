//! Spiller — disk spilling and stream-merge for memory-constrained batch ingestion.
//!
//! When a `ColumnChunk` or `NodeGroup` exceeds the configured memory threshold
//! during `COPY FROM` or bulk inserts, the spiller serializes the in-memory data
//! to a temporary file. Once all rows are ingested, a multi-way stream-merge
//! reads back all spill files plus the final in-memory buffer, deduplicates by
//! primary key, and writes the merged result to the persistent `Column` via
//! `BufferManager`.
//!
//! # Strategy
//!
//! This is a simple `Vec<Value>` → JSON-lines → disk approach. Each spill file
//! contains one JSON object per row. This is intentionally not Arrow-CSR format
//! — that can be a future optimization.
//!
//! # Usage
//!
//! ```ignore
//! let spiller = Spiller::new("/tmp/kuzu_spill", 1024 * 1024);
//! let mut chunk = ColumnChunk::new();
//! // ... append values ...
//! let spill_file = spiller.spill(&mut chunk)?;
//! // ... later ...
//! let restored = spiller.restore(&spill_file)?;
//! ```

use crate::column_chunk::ColumnChunk;
use kuzu_common::types::Value;
use serde::{Deserialize, Serialize};
use serde_json;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Metadata for a single spill file on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpillFile {
    /// Absolute path to the temporary spill file.
    pub path: PathBuf,
    /// Number of rows in this spill file.
    pub row_count: usize,
    /// Optional sort key column index (for PK-ordered merge).
    pub sort_key_column: Option<usize>,
}

/// The spiller manages temporary files for memory-constrained batch operations.
///
/// Each `spill()` call serializes a `ColumnChunk` to a new temp file and clears
/// the chunk's in-memory buffer. Spill files are named `spill_00001.jsonl` etc.
#[derive(Debug)]
pub struct Spiller {
    /// Directory where spill files are written.
    tmp_dir: PathBuf,
    /// Monotonically increasing counter for unique spill file names.
    spill_counter: AtomicU64,
    /// Maximum in-memory bytes per ColumnChunk before spilling triggers.
    /// When a chunk's estimated memory exceeds this, `spill()` is called.
    pub memory_threshold: u64,
}

impl Spiller {
    /// Create a new spiller that writes to `tmp_dir`.
    ///
    /// `memory_threshold` is the estimated byte limit for a `ColumnChunk`
    /// before triggering a spill. Use 0 to disable spilling entirely.
    pub fn new(tmp_dir: impl Into<PathBuf>, memory_threshold: u64) -> Self {
        let tmp_dir = tmp_dir.into();
        let _ = fs::create_dir_all(&tmp_dir);
        Self {
            tmp_dir,
            spill_counter: AtomicU64::new(1),
            memory_threshold,
        }
    }

    /// Return the next unique spill file path.
    fn next_spill_path(&self) -> PathBuf {
        let n = self.spill_counter.fetch_add(1, Ordering::Relaxed);
        self.tmp_dir.join(format!("spill_{n:05}.jsonl"))
    }

    /// Estimate the in-memory size of a `ColumnChunk`'s buffered values.
    ///
    /// This is a rough estimate: each `Value` variant has different sizes.
    /// We use a conservative estimate of 64 bytes per value on average.
    pub fn estimated_chunk_size(chunk: &ColumnChunk) -> u64 {
        chunk.num_values() as u64 * 64
    }

    /// Spill a `ColumnChunk` to disk: serialize its values to a JSON-lines file
    /// and clear the in-memory buffer.
    ///
    /// Returns the `SpillFile` metadata on success.
    ///
    /// If the chunk is empty, returns `None`.
    pub fn spill(&self, chunk: &mut ColumnChunk) -> Result<Option<SpillFile>, String> {
        if chunk.is_empty() {
            return Ok(None);
        }

        let path = self.next_spill_path();
        let values: Vec<Value> = chunk.drain();
        let row_count = values.len();

        // Write as JSON-lines: one line per row, each line is a JSON array of column values
        // For a single-column chunk, each line is just one value.
        // Multi-column chunks are handled by the caller (NodeGroup) which spills
        // all columns together in a coordinated way.
        let mut file = fs::File::create(&path)
            .map_err(|e| format!("Failed to create spill file {:?}: {e}", path))?;

        for value in &values {
            let line = serde_json::to_string(value)
                .map_err(|e| format!("Failed to serialize value: {e}"))?;
            writeln!(file, "{line}")
                .map_err(|e| format!("Failed to write spill file: {e}"))?;
        }

        // Re-allocate the chunk's buffer
        let _ = chunk; // chunk is already drained by `drain()`

        Ok(Some(SpillFile {
            path,
            row_count,
            sort_key_column: None,
        }))
    }

    /// Spill all columns of a NodeGroup-style set of column chunks together.
    ///
    /// Each row is serialized as a JSON array `[col0, col1, ..., colN]`.
    /// This is used by `NodeGroup` to spill multi-column data.
    pub fn spill_columns(
        &self,
        chunks: &mut [ColumnChunk],
    ) -> Result<Option<SpillFile>, String> {
        if chunks.is_empty() || chunks[0].is_empty() {
            return Ok(None);
        }

        let path = self.next_spill_path();
        let num_rows = chunks[0].num_values();
        let num_cols = chunks.len();

        // Drain all columns
        let mut drained: Vec<Vec<Value>> = chunks.iter_mut().map(|c| c.drain()).collect();

        let mut file = fs::File::create(&path)
            .map_err(|e| format!("Failed to create spill file {:?}: {e}", path))?;

        for row in 0..num_rows {
            let mut row_values = Vec::with_capacity(num_cols);
            for col in 0..num_cols {
                let val = if row < drained[col].len() {
                    std::mem::replace(&mut drained[col][row], Value::Null)
                } else {
                    Value::Null
                };
                row_values.push(val);
            }
            let line = serde_json::to_string(&row_values)
                .map_err(|e| format!("Failed to serialize row: {e}"))?;
            writeln!(file, "{line}")
                .map_err(|e| format!("Failed to write spill file: {e}"))?;
        }

        Ok(Some(SpillFile {
            path,
            row_count: num_rows,
            sort_key_column: None,
        }))
    }

    /// Restore a single-column `ColumnChunk` from a spill file.
    ///
    /// Each line of the JSON-lines file is deserialized as a single `Value`.
    /// Returns a new `ColumnChunk` with the restored values.
    pub fn restore(&self, spill: &SpillFile) -> Result<ColumnChunk, String> {
        let file = fs::File::open(&spill.path)
            .map_err(|e| format!("Failed to open spill file {:?}: {e}", spill.path))?;
        let reader = BufReader::new(file);
        let mut values = Vec::with_capacity(spill.row_count);

        for line in reader.lines() {
            let line = line.map_err(|e| format!("Failed to read spill file: {e}"))?;
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(&line)
                .map_err(|e| format!("Failed to deserialize value from spill file: {e}"))?;
            values.push(value);
        }

        let chunk = ColumnChunk::from(values);
        Ok(chunk)
    }

    /// Restore a multi-column result from a spill file containing JSON arrays.
    ///
    /// Returns one `ColumnChunk` per column.
    pub fn restore_columns(&self, spill: &SpillFile, num_cols: usize) -> Result<Vec<ColumnChunk>, String> {
        let file = fs::File::open(&spill.path)
            .map_err(|e| format!("Failed to open spill file {:?}: {e}", spill.path))?;
        let reader = BufReader::new(file);

        let mut columns: Vec<Vec<Value>> = (0..num_cols).map(|_| Vec::new()).collect();

        for line in reader.lines() {
            let line = line.map_err(|e| format!("Failed to read spill file: {e}"))?;
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }
            let row: Vec<Value> = serde_json::from_str(&line)
                .map_err(|e| format!("Failed to deserialize row from spill file: {e}"))?;
            for (col, value) in row.into_iter().enumerate() {
                if col < num_cols {
                    columns[col].push(value);
                }
            }
        }

        Ok(columns
            .into_iter()
            .map(|vals| ColumnChunk::from(vals))
            .collect())
    }

    /// Remove a spill file from disk.
    pub fn cleanup(&self, spill: &SpillFile) -> Result<(), String> {
        fs::remove_file(&spill.path)
            .map_err(|e| format!("Failed to remove spill file {:?}: {e}", spill.path))
    }

    /// Remove all spill files in the temp directory.
    pub fn cleanup_all(&self) -> Result<(), String> {
        if self.tmp_dir.exists() {
            fs::remove_dir_all(&self.tmp_dir)
                .map_err(|e| format!("Failed to remove spill dir {:?}: {e}", self.tmp_dir))?;
        }
        Ok(())
    }

    /// Check whether a chunk's estimated size exceeds the memory threshold.
    pub fn should_spill(&self, chunk: &ColumnChunk) -> bool {
        if self.memory_threshold == 0 {
            return false;
        }
        Self::estimated_chunk_size(chunk) > self.memory_threshold
    }
}

impl Drop for Spiller {
    fn drop(&mut self) {
        // Best-effort cleanup of the temp directory
        let _ = fs::remove_dir_all(&self.tmp_dir);
    }
}

// ---------------------------------------------------------------------------
// Multi-way stream-merge
// ---------------------------------------------------------------------------

/// A file handle paired with the next buffered row, for streaming merge.
struct MergeCursor {
    /// Source spill file path (for logging).
    _source: PathBuf,
    /// Reader over the JSON-lines file.
    reader: BufReader<fs::File>,
    /// The next buffered row (None if exhausted).
    current: Option<Vec<Value>>,
    /// Column index to use as sort key (for ordering).
    sort_key_col: usize,
}

impl MergeCursor {
    fn new(path: &Path, sort_key_col: usize) -> Result<Self, String> {
        let file = fs::File::open(path)
            .map_err(|e| format!("Failed to open merge source {:?}: {e}", path))?;
        let mut reader = BufReader::new(file);
        let current = Self::read_next_row(&mut reader);
        Ok(Self {
            _source: path.to_path_buf(),
            reader,
            current,
            sort_key_col,
        })
    }

    fn read_next_row(reader: &mut BufReader<fs::File>) -> Option<Vec<Value>> {
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => return None, // EOF
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<Vec<Value>>(trimmed) {
                        Ok(row) => return Some(row),
                        Err(_) => {
                            // Try as single value (single-column spill files)
                            match serde_json::from_str::<Value>(trimmed) {
                                Ok(val) => return Some(vec![val]),
                                Err(_) => continue,
                            }
                        }
                    }
                }
                Err(_) => return None,
            }
        }
    }

    fn advance(&mut self) {
        self.current = Self::read_next_row(&mut self.reader);
    }

    fn is_exhausted(&self) -> bool {
        self.current.is_none()
    }

    fn sort_key(&self) -> Option<i64> {
        self.current.as_ref().and_then(|row| {
            if self.sort_key_col < row.len() {
                match &row[self.sort_key_col] {
                    Value::Int64(v) => Some(*v),
                    Value::Int32(v) => Some(*v as i64),
                    Value::Int16(v) => Some(*v as i64),
                    Value::Int8(v) => Some(*v as i64),
                    Value::UInt64(v) => Some(*v as i64),
                    Value::UInt32(v) => Some(*v as i64),
                    Value::UInt16(v) => Some(*v as i64),
                    Value::UInt8(v) => Some(*v as i64),
                    _ => None,
                }
            } else {
                None
            }
        })
    }
}

/// Multi-way streaming merge of multiple spill files.
///
/// Reads N spill files + one optional in-memory buffer, merges them in
/// sort-key order (ascending), and optionally deduplicates by primary key.
///
/// # Usage
///
/// ```ignore
/// let merger = MultiWayStreamMerge::new(spill_files, None, 0)?;
/// while let Some(row) = merger.next() {
///     // Write row to Column via BufferManager
/// }
/// ```
pub struct MultiWayStreamMerge {
    cursors: Vec<MergeCursor>,
    /// The in-memory buffer (last "run" to merge).
    in_memory_rows: Vec<Vec<Value>>,
    in_memory_idx: usize,
    sort_key_col: usize,
    dedup: bool,
    /// Last emitted sort key (for dedup).
    last_key: Option<i64>,
}

impl MultiWayStreamMerge {
    /// Create a new multi-way stream merge.
    ///
    /// * `spill_files` — list of spill files to read from disk.
    /// * `in_memory` — optional final in-memory buffer (rows as `Vec<Vec<Value>>`).
    /// * `sort_key_col` — column index to use as the sort/merge key.
    /// * `dedup` — if true, consecutive duplicate sort keys are skipped.
    pub fn new(
        spill_files: &[SpillFile],
        in_memory: Option<Vec<Vec<Value>>>,
        sort_key_col: usize,
        dedup: bool,
    ) -> Result<Self, String> {
        let mut cursors = Vec::new();
        for sf in spill_files {
            if sf.row_count > 0 {
                cursors.push(MergeCursor::new(&sf.path, sort_key_col)?);
            }
        }
        Ok(Self {
            cursors,
            in_memory_rows: in_memory.unwrap_or_default(),
            in_memory_idx: 0,
            sort_key_col,
            dedup,
            last_key: None,
        })
    }

    /// Get the next row from the merge.
    ///
    /// Returns `None` when all sources are exhausted.
    pub fn next(&mut self) -> Option<Vec<Value>> {
        loop {
            // Find the cursor with the smallest sort key
            let smallest = self.find_smallest();
            let row = match smallest {
                MergeSource::Cursor(idx) => {
                    let row = self.cursors[idx].current.take()?;
                    self.cursors[idx].advance();
                    row
                }
                MergeSource::InMemory => {
                    if self.in_memory_idx < self.in_memory_rows.len() {
                        let row = self.in_memory_rows[self.in_memory_idx].clone();
                        self.in_memory_idx += 1;
                        row
                    } else {
                        return None;
                    }
                }
                MergeSource::None => return None,
            };

            // Dedup: skip if same sort key as last emitted row
            if self.dedup {
                let key = if self.sort_key_col < row.len() {
                    match &row[self.sort_key_col] {
                        Value::Int64(v) => Some(*v),
                        Value::Int32(v) => Some(*v as i64),
                        Value::Int16(v) => Some(*v as i64),
                        Value::Int8(v) => Some(*v as i64),
                        Value::UInt64(v) => Some(*v as i64),
                        Value::UInt32(v) => Some(*v as i64),
                        Value::UInt16(v) => Some(*v as i64),
                        Value::UInt8(v) => Some(*v as i64),
                        _ => None,
                    }
                } else {
                    None
                };

                if let Some(k) = key {
                    if self.last_key == Some(k) {
                        // Duplicate — skip and continue
                        continue;
                    }
                    self.last_key = Some(k);
                }
            }

            return Some(row);
        }
    }

    /// Find the source with the smallest sort key among all cursors and the in-memory buffer.
    fn find_smallest(&self) -> MergeSource {
        let mut best: Option<(MergeSource, i64)> = None;

        for (idx, cursor) in self.cursors.iter().enumerate() {
            if cursor.is_exhausted() {
                continue;
            }
            if let Some(key) = cursor.sort_key() {
                match best {
                    Some((_, best_key)) if key < best_key => {
                        best = Some((MergeSource::Cursor(idx), key));
                    }
                    None => {
                        best = Some((MergeSource::Cursor(idx), key));
                    }
                    _ => {}
                }
            }
        }

        // Check in-memory buffer
        if self.in_memory_idx < self.in_memory_rows.len() {
            let row = &self.in_memory_rows[self.in_memory_idx];
            let key = if self.sort_key_col < row.len() {
                match &row[self.sort_key_col] {
                    Value::Int64(v) => Some(*v),
                    Value::Int32(v) => Some(*v as i64),
                    Value::Int16(v) => Some(*v as i64),
                    Value::Int8(v) => Some(*v as i64),
                    Value::UInt64(v) => Some(*v as i64),
                    Value::UInt32(v) => Some(*v as i64),
                    Value::UInt16(v) => Some(*v as i64),
                    Value::UInt8(v) => Some(*v as i64),
                    _ => None,
                }
            } else {
                None
            };

            if let Some(k) = key {
                match best {
                    Some((_, best_key)) if k < best_key => {
                        best = Some((MergeSource::InMemory, k));
                    }
                    None => {
                        best = Some((MergeSource::InMemory, k));
                    }
                    _ => {}
                }
            }
        }

        best.map_or(MergeSource::None, |(src, _)| src)
    }
}

enum MergeSource {
    Cursor(usize),
    InMemory,
    None,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_spill_and_restore_single_column() {
        let tmp = tempfile::tempdir().unwrap();
        let spiller = Spiller::new(tmp.path(), 1024);

        let mut chunk = ColumnChunk::new();
        for i in 0..10 {
            chunk.append(Value::Int64(i));
        }
        assert_eq!(chunk.num_values(), 10);

        let spill = spiller.spill(&mut chunk).unwrap().unwrap();
        assert!(chunk.is_empty());
        assert_eq!(spill.row_count, 10);
        assert!(spill.path.exists());

        let restored = spiller.restore(&spill).unwrap();
        assert_eq!(restored.num_values(), 10);
        assert_eq!(restored.get(0), Some(&Value::Int64(0)));
        assert_eq!(restored.get(9), Some(&Value::Int64(9)));

        spiller.cleanup(&spill).unwrap();
    }

    #[test]
    fn test_spill_and_restore_columns() {
        let tmp = tempfile::tempdir().unwrap();
        let spiller = Spiller::new(tmp.path(), 1024);

        let mut chunk0 = ColumnChunk::new();
        let mut chunk1 = ColumnChunk::new();
        for i in 0..5 {
            chunk0.append(Value::Int64(i));
            chunk1.append(Value::String(format!("val_{i}")));
        }

        let spill = spiller.spill_columns(&mut [chunk0, chunk1]).unwrap().unwrap();
        assert_eq!(spill.row_count, 5);

        let restored = spiller.restore_columns(&spill, 2).unwrap();
        assert_eq!(restored.len(), 2);
        assert_eq!(restored[0].num_values(), 5);
        assert_eq!(restored[0].get(0), Some(&Value::Int64(0)));
        assert_eq!(restored[1].get(4), Some(&Value::String("val_4".into())));

        spiller.cleanup(&spill).unwrap();
    }

    #[test]
    fn test_should_spill() {
        let tmp = tempfile::tempdir().unwrap();
        let spiller = Spiller::new(tmp.path(), 128); // Very low threshold

        let mut chunk = ColumnChunk::new();
        // 100 values * 64 bytes = 6400 bytes → should spill
        for i in 0..100 {
            chunk.append(Value::Int64(i));
        }
        assert!(spiller.should_spill(&chunk));

        // Disabled spilling
        let spiller_disabled = Spiller::new(tmp.path(), 0);
        assert!(!spiller_disabled.should_spill(&chunk));
    }

    #[test]
    fn test_spill_empty_chunk() {
        let tmp = tempfile::tempdir().unwrap();
        let spiller = Spiller::new(tmp.path(), 1024);
        let mut chunk = ColumnChunk::new();
        let result = spiller.spill(&mut chunk).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_multi_way_merge() {
        let tmp = tempfile::tempdir().unwrap();
        let spiller = Spiller::new(tmp.path(), 1024);

        // Create 3 spill files with sorted data
        let mut files = Vec::new();

        // File 1: values [1, 4, 7]
        let mut c1 = ColumnChunk::new();
        for v in [1i64, 4, 7] {
            c1.append(Value::Int64(v));
        }
        files.push(spiller.spill(&mut c1).unwrap().unwrap());

        // File 2: values [2, 5, 8]
        let mut c2 = ColumnChunk::new();
        for v in [2i64, 5, 8] {
            c2.append(Value::Int64(v));
        }
        files.push(spiller.spill(&mut c2).unwrap().unwrap());

        // File 3: values [3, 6, 9]
        let mut c3 = ColumnChunk::new();
        for v in [3i64, 6, 9] {
            c3.append(Value::Int64(v));
        }
        files.push(spiller.spill(&mut c3).unwrap().unwrap());

        // Merge with no dedup
        let mut merger = MultiWayStreamMerge::new(&files, None, 0, false).unwrap();
        let mut merged = Vec::new();
        while let Some(row) = merger.next() {
            merged.push(row[0].clone());
        }

        assert_eq!(merged.len(), 9);
        // Verify sorted order
        for i in 0..9 {
            assert_eq!(merged[i], Value::Int64((i + 1) as i64), "Position {i}");
        }
    }

    #[test]
    fn test_multi_way_merge_with_dedup() {
        let tmp = tempfile::tempdir().unwrap();
        let spiller = Spiller::new(tmp.path(), 1024);

        let mut files = Vec::new();

        // File 1: [1, 2, 3]
        let mut c1 = ColumnChunk::new();
        for v in [1i64, 2, 3] {
            c1.append(Value::Int64(v));
        }
        files.push(spiller.spill(&mut c1).unwrap().unwrap());

        // File 2: [2, 3, 4] (overlaps with file 1)
        let mut c2 = ColumnChunk::new();
        for v in [2i64, 3, 4] {
            c2.append(Value::Int64(v));
        }
        files.push(spiller.spill(&mut c2).unwrap().unwrap());

        // Merge WITH dedup
        let mut merger = MultiWayStreamMerge::new(&files, None, 0, true).unwrap();
        let mut merged = Vec::new();
        while let Some(row) = merger.next() {
            merged.push(row[0].clone());
        }

        assert_eq!(merged.len(), 4);
        assert_eq!(merged[0], Value::Int64(1));
        assert_eq!(merged[1], Value::Int64(2));
        assert_eq!(merged[2], Value::Int64(3));
        assert_eq!(merged[3], Value::Int64(4));
    }

    #[test]
    fn test_merge_with_in_memory() {
        let tmp = tempfile::tempdir().unwrap();
        let spiller = Spiller::new(tmp.path(), 1024);

        // Spill file: [1, 3, 5]
        let mut c1 = ColumnChunk::new();
        for v in [1i64, 3, 5] {
            c1.append(Value::Int64(v));
        }
        let files = vec![spiller.spill(&mut c1).unwrap().unwrap()];

        // In-memory: [2, 4, 6]
        let in_memory: Vec<Vec<Value>> = vec![
            vec![Value::Int64(2)],
            vec![Value::Int64(4)],
            vec![Value::Int64(6)],
        ];

        let mut merger = MultiWayStreamMerge::new(&files, Some(in_memory), 0, false).unwrap();
        let mut merged = Vec::new();
        while let Some(row) = merger.next() {
            merged.push(row[0].clone());
        }

        assert_eq!(merged.len(), 6);
        for i in 0..6 {
            assert_eq!(merged[i], Value::Int64((i + 1) as i64));
        }
    }

    #[test]
    fn test_cleanup_all() {
        let tmp = tempfile::tempdir().unwrap();
        let spiller = Spiller::new(tmp.path(), 1024);

        let mut c = ColumnChunk::new();
        c.append(Value::Int64(42));
        let spill = spiller.spill(&mut c).unwrap().unwrap();
        assert!(spill.path.exists());

        spiller.cleanup_all().unwrap();
        assert!(!spill.path.exists());
    }

    #[test]
    fn test_cleanup_on_drop() {
        let tmp = tempfile::tempdir().unwrap();
        let spill_dir = tmp.path().join("spill_test");
        {
            let spiller = Spiller::new(&spill_dir, 1024);
            let mut c = ColumnChunk::new();
            c.append(Value::Int64(42));
            spiller.spill(&mut c).unwrap();
            assert!(spill_dir.exists());
        }
        // After Spiller is dropped, the temp directory should be cleaned up
        assert!(!spill_dir.exists());
    }
}
