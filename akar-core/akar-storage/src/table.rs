//! Table storage — columnar node/rel tables with NodeGroup-based storage.

use crate::art_index::ArtPrimaryKeyIndex;
use crate::art_key::ArtKey;
use crate::column::Column;
use crate::csr::CsrIndex;
use crate::index::HashIndex;
use crate::node_group::NodeGroup;
use crate::vector_index::VectorIndexTable;
use akar_common::error::StorageError;
use akar_common::types::{LogicalTypeID, Value};
use akar_vector::hnsw::DistanceMetric;
use dashmap::DashMap;
use std::collections::HashMap;

/// A column definition within a table.
#[derive(Debug, Clone)]
pub struct ColumnDefinition {
    pub name: String,
    pub logical_type: LogicalTypeID,
    pub is_primary_key: bool,
    pub compression: akar_common::enums::CompressionType,
}

/// A node table stores properties for a node label using NodeGroup-based
/// columnar storage. Data is held in-memory as `NodeGroup`s; when a group
/// reaches `NODE_GROUP_SIZE` rows a new group is automatically created.
///
/// Primary key uniqueness is enforced via an in-memory `HashIndex` that maps
/// PK values to row offsets. For persistent indexes, use `OnDiskHashIndex`
/// alongside the L1 cache (see `index.rs`).
///
/// Optionally, an `ArtPrimaryKeyIndex` can be attached for range-scan
/// support on the primary key column (see `create_art_index`).
#[derive(Debug, Clone)]
pub struct NodeTable {
    pub table_id: u64,
    pub name: String,
    pub columns: Vec<ColumnDefinition>,
    pub primary_key_column: usize,
    pub num_rows: u64,
    /// NodeGroup-based columnar storage. Each group holds up to
    /// `NODE_GROUP_SIZE` rows across all columns.
    pub node_groups: Vec<NodeGroup>,
    /// In-memory hash index for primary key → row lookup and dedup.
    /// Stores the PK value (as a string representation) mapped to row offset.
    pub hash_index: HashIndex<String>,
    /// Optional ART (Adaptive Radix Tree) index for PK range scans.
    /// When present, `insert_row()` also updates this index automatically.
    pub art_index: Option<ArtPrimaryKeyIndex>,
    /// Set when an UPDATE/DELETE touches the table. The durable column mirror
    /// (see `persistence.rs`) performs a full rewrite when this flag is set.
    pub persistence_dirty: bool,
}

/// Sentinel for `NodeTable::primary_key_column` when a node table has no PK
/// column. SQL always requires a PK (binder rejects `CREATE NODE TABLE` without
/// one), so via the SQL path this is unreachable; the sentinel only covers
/// internally/test-constructed tables and keeps "no PK" explicit instead of
/// silently defaulting to column 0 (P49.2).
pub const NO_PRIMARY_KEY: usize = usize::MAX;

impl NodeTable {
    pub fn new(table_id: u64, name: String, columns: Vec<ColumnDefinition>) -> Self {
        // SQL guarantees a PK exists (binder/mod.rs:861 rejects tables without
        // one), so the fallback is unreachable in production. Use an explicit
        // sentinel instead of a silent 0 so a table with no PK column never
        // accidentally dedups on column 0 (P49.2).
        let primary_key_column = columns.iter().position(|c| c.is_primary_key).unwrap_or(NO_PRIMARY_KEY);
        Self {
            table_id,
            name,
            columns,
            primary_key_column,
            num_rows: 0,
            node_groups: Vec::new(),
            hash_index: HashIndex::new(),
            art_index: None,
            persistence_dirty: false,
        }
    }

    /// Insert a row of values into the table.
    ///
    /// Appends to the current `NodeGroup`; auto-creates a new group when the
    /// current one is full (reaches `NODE_GROUP_SIZE` rows).
    ///
    /// If the table has a primary key column, checks for duplicates and rejects
    /// rows with already-existing PK values. The hash index is updated after
    /// a successful insert.
    ///
    /// When `txn_id` is `Some(...)`, the insert is recorded in VersionInfo
    /// for MVCC snapshot isolation.
    ///
    /// Returns an error if the number of values doesn't match the number of columns,
    /// or if a duplicate primary key value is detected.
    pub fn insert_row(&mut self, values: Vec<Value>) -> Result<u64, StorageError> {
        self.insert_row_with_txn(values, None)
    }

    /// Insert a row with an optional transaction ID for MVCC tracking.
    pub fn insert_row_with_txn(&mut self, mut values: Vec<Value>, txn_id: Option<u64>) -> Result<u64, StorageError> {
        if values.len() != self.columns.len() {
            return Err(StorageError::Page(format!(
                "Column count mismatch: expected {} values, got {}",
                self.columns.len(),
                values.len()
            )));
        }

        // Reject NULL primary key values
        if self.primary_key_column < self.columns.len() {
            let pk_value = &values[self.primary_key_column];
            if matches!(pk_value, Value::Null) {
                return Err(StorageError::Page(format!(
                    "NULL value not allowed for primary key column '{}' in table '{}'",
                    self.columns[self.primary_key_column].name, self.name
                )));
            }
        }

        // Coerce literal values to the declared column types (P48.12): constant
        // evaluation (e.g. CREATE `{id: 41}`) produces Int64 literals regardless
        // of the target column, and a UINT64 column must store Value::UInt64 so
        // the scan builds the correct Arrow type instead of dropping the value.
        coerce_values_to_columns(&mut values, &self.columns)?;

        // Check primary key uniqueness
        if self.primary_key_column < self.columns.len() {
            let pk_value = &values[self.primary_key_column];
            let pk_key = pk_value_to_string(pk_value);
            if self.hash_index.lookup(&pk_key).is_some() {
                return Err(StorageError::Index(format!(
                    "Duplicate primary key value: '{pk_key}' in table '{}'",
                    self.name
                )));
            }
        }

        // Get or create the current node group.
        let num_cols = self.columns.len();
        if self.node_groups.is_empty() || self.node_groups.last().unwrap().is_full() {
            let start_offset = self.num_rows;
            let mut new_group = NodeGroup::new(num_cols, start_offset);
            // Enable version info if MVCC tracking is requested
            if txn_id.is_some() {
                new_group.enable_version_info();
            }
            self.node_groups.push(new_group);
        }

        let current = self.node_groups.last_mut().unwrap();
        // Enable version info on existing group if needed
        if txn_id.is_some() {
            current.enable_version_info();
        }
        current.append_row_with_txn(values.clone(), txn_id)?;
        self.num_rows += 1;

        // Update hash index with the PK value for this row
        if self.primary_key_column < self.columns.len() {
            let pk_value = &values[self.primary_key_column];
            let pk_key = pk_value_to_string(pk_value);
            self.hash_index.insert(pk_key, self.num_rows - 1);

            // Also update ART index if present
            if let Some(ref mut art_idx) = self.art_index
                && let Some(art_key) = ArtKey::from_value(pk_value)
            {
                art_idx.insert(&art_key, self.num_rows - 1);
            }
        }

        Ok(self.num_rows - 1)
    }

    /// Batch insert multiple rows efficiently.
    /// Validates PK uniqueness, pre-allocates node groups, and bulk-appends.
    /// When `txn_id` is `Some(...)`, inserts are recorded in VersionInfo for MVCC.
    pub fn insert_rows_batch(&mut self, rows: &[Vec<Value>]) -> Result<u64, StorageError> {
        self.insert_rows_batch_with_txn(rows, None)
    }

    /// Batch insert with optional MVCC tracking.
    pub fn insert_rows_batch_with_txn(
        &mut self,
        rows: &[Vec<Value>],
        txn_id: Option<u64>,
    ) -> Result<u64, StorageError> {
        if rows.is_empty() {
            return Ok(0);
        }
        let num_cols = self.columns.len();

        // Validate all rows first
        for (i, row) in rows.iter().enumerate() {
            if row.len() != num_cols {
                return Err(StorageError::Page(format!(
                    "Row {} column count mismatch: expected {} values, got {}",
                    i,
                    num_cols,
                    row.len()
                )));
            }
            // Reject NULL primary key values
            if self.primary_key_column < num_cols {
                let pk_value = &row[self.primary_key_column];
                if matches!(pk_value, Value::Null) {
                    return Err(StorageError::Page(format!(
                        "NULL value not allowed for primary key column '{}' in table '{}'",
                        self.columns[self.primary_key_column].name, self.name
                    )));
                }
            }
            // Check PK uniqueness
            if self.primary_key_column < num_cols {
                let pk_key = pk_value_to_string(&row[self.primary_key_column]);
                if self.hash_index.lookup(&pk_key).is_some() {
                    return Err(StorageError::Index(format!(
                        "Duplicate primary key value: '{pk_key}' in table '{}'",
                        self.name
                    )));
                }
            }
        }

        let start_offset = self.num_rows;
        let total_new = rows.len();

        // Coerce literal values to the declared column types (P48.12), mirroring
        // `insert_row_with_txn`. This must happen before appending and index
        // updates so PK hash/ART keys use the coerced (UInt64) encoding.
        let mut coerced_rows = rows.to_vec();
        for row in &mut coerced_rows {
            coerce_values_to_columns(row, &self.columns)?;
        }
        let rows: &[Vec<Value>] = &coerced_rows;

        // Ensure we have a node group with enough capacity
        if self.node_groups.is_empty() || self.node_groups.last().unwrap().is_full() {
            let off = if self.node_groups.is_empty() {
                start_offset
            } else {
                self.num_rows
            };
            let mut new_group = NodeGroup::new(num_cols, off);
            if txn_id.is_some() {
                new_group.enable_version_info();
            }
            self.node_groups.push(new_group);
        }

        // Append to the last group (spilling into new groups if needed)
        let mut inserted = 0usize;
        while inserted < total_new {
            let current = self.node_groups.last_mut().unwrap();
            if txn_id.is_some() {
                current.enable_version_info();
            }
            let rem = current.remaining();
            let take = (total_new - inserted).min(rem);
            for row in &rows[inserted..inserted + take] {
                current.append_row_with_txn(row.clone(), txn_id)?;
            }
            self.num_rows += take as u64;
            inserted += take;
            if inserted < total_new {
                let off = self.num_rows;
                let mut new_group = NodeGroup::new(num_cols, off);
                if txn_id.is_some() {
                    new_group.enable_version_info();
                }
                self.node_groups.push(new_group);
            }
        }

        // Batch update indexes
        for (i, row) in rows.iter().enumerate() {
            if self.primary_key_column < num_cols {
                let pk_key = pk_value_to_string(&row[self.primary_key_column]);
                self.hash_index.insert(pk_key, start_offset + i as u64);
                if let Some(ref mut art_idx) = self.art_index
                    && let Some(art_key) = ArtKey::from_value(&row[self.primary_key_column])
                {
                    art_idx.insert(&art_key, start_offset + i as u64);
                }
            }
        }

        Ok(rows.len() as u64)
    }

    /// Look up a row offset by its primary key value.
    ///
    /// Returns `Some(row_offset)` if the PK exists, or `None` if not found.
    /// Uses the in-memory hash index for O(1) lookup.
    pub fn lookup_by_pk(&self, pk_value: &Value) -> Option<u64> {
        let pk_key = pk_value_to_string(pk_value);
        self.hash_index.lookup(&pk_key)
    }

    /// Batch look up row offsets for multiple primary key values.
    ///
    /// Returns a `Vec<Option<u64>>` parallel to the input, where each element
    /// is `Some(row_offset)` if the PK exists, or `None` if not found.
    /// Uses the in-memory hash index for O(1) per-key lookup, avoiding
    /// per-row method-call overhead by inlining the lookup logic.
    pub fn lookup_by_pk_batch(&self, pk_values: &[Value]) -> Vec<Option<u64>> {
        pk_values
            .iter()
            .map(|pk_value| {
                let pk_key = pk_value_to_string(pk_value);
                self.hash_index.lookup(&pk_key)
            })
            .collect()
    }

    /// Perform a range scan on the primary key column using the ART index.
    ///
    /// Returns up to `max_results` row offsets for keys within `[lower, upper]`
    /// (respecting inclusivity flags). Returns an empty vec if no ART index
    /// exists or no keys match.
    ///
    /// This is the bridge function called by `PhysicalArtIndexRangeScan`.
    pub fn lookup_by_pk_range(
        &self,
        lower: Option<&Value>,
        lower_inclusive: bool,
        upper: Option<&Value>,
        upper_inclusive: bool,
        max_results: u64,
    ) -> Vec<u64> {
        match &self.art_index {
            Some(idx) => {
                let lower_key = lower.and_then(ArtKey::from_value);
                let upper_key = upper.and_then(ArtKey::from_value);
                idx.range_scan(
                    lower_key.as_ref(),
                    lower_inclusive,
                    upper_key.as_ref(),
                    upper_inclusive,
                    max_results,
                )
            }
            None => Vec::new(),
        }
    }

    /// Scan all values for a given column across all node groups.
    ///
    /// Returns a flat `Vec<Value>` containing values from `start` to
    /// `start + count` (or fewer if the end of the table is reached).
    ///
    /// If `snapshot_ts` is `Some(...)`, performs MVCC snapshot isolation:
    /// rows inserted/deleted by transactions committed after `snapshot_ts`
    /// are excluded, and versioned updates are resolved.
    pub fn scan_column(
        &self,
        col_idx: usize,
        start: u64,
        count: u64,
        snapshot_ts: Option<u64>,
        commit_history: &[(u64, u64)],
    ) -> Vec<Value> {
        if col_idx >= self.columns.len() || start >= self.num_rows {
            return Vec::new();
        }
        let end = (start + count).min(self.num_rows);
        let mut result = Vec::with_capacity((end - start) as usize);

        // Find the first node group containing `start`.
        let group_start = self.find_group(start);
        let mut remaining = end - start;

        for g_idx in group_start..self.node_groups.len() {
            if remaining == 0 {
                break;
            }
            let group = &self.node_groups[g_idx];
            let local_start = if g_idx == group_start {
                (start - group.start_offset) as usize
            } else {
                0
            };
            let available = (group.num_nodes as usize).saturating_sub(local_start);
            let take = available.min(remaining as usize);

            for row in local_start..local_start + take {
                let val = group.get_value_with_snapshot(row, col_idx, snapshot_ts, commit_history);
                match val {
                    Some(v) => result.push(v.clone()),
                    None => result.push(Value::Null),
                }
            }
            remaining -= take as u64;
        }

        result
    }

    /// Rebuild the table's in-memory state from rows loaded off the durable
    /// column mirror (see `persistence.rs`).
    ///
    /// Populates `node_groups`, `num_rows`, the PK hash index, and the ART
    /// index directly, bypassing PK uniqueness checks so that soft-deleted
    /// rows (whose PK is `Null`) can be restored at their original row
    /// offsets.
    pub fn load_persisted_rows(&mut self, rows: Vec<Vec<Value>>) -> Result<(), StorageError> {
        let num_cols = self.columns.len();
        self.node_groups.clear();
        self.hash_index.clear();

        let mut offset = 0u64;
        let mut group = NodeGroup::new(num_cols, offset);
        for row in &rows {
            if row.len() != num_cols {
                return Err(StorageError::Page(format!(
                    "load_persisted_rows: expected {num_cols} values, got {}",
                    row.len()
                )));
            }
            group.append_row_with_txn(row.clone(), None)?;
            offset += 1;
            if group.is_full() {
                self.node_groups.push(group);
                group = NodeGroup::new(num_cols, offset);
            }
        }
        if group.num_nodes > 0 {
            self.node_groups.push(group);
        }
        self.num_rows = offset;

        // Rebuild the PK hash index (skip soft-deleted rows with a Null PK).
        if self.primary_key_column < num_cols {
            for (row_idx, values) in rows.iter().enumerate() {
                let pk = &values[self.primary_key_column];
                if matches!(pk, Value::Null) {
                    continue;
                }
                let key = pk_value_to_string(pk);
                self.hash_index.insert(key, row_idx as u64);
            }
        }

        // Rebuild the ART index if present.
        if self.art_index.is_some() && self.primary_key_column < num_cols {
            if let Some(art) = &mut self.art_index {
                art.clear();
                for (row_idx, values) in rows.iter().enumerate() {
                    let pk = &values[self.primary_key_column];
                    if matches!(pk, Value::Null) {
                        continue;
                    }
                    if let Some(key) = ArtKey::from_value(pk) {
                        art.insert(&key, row_idx as u64);
                    }
                }
            }
        }
        Ok(())
    }

    /// Update a single cell (row, column) with a new value.
    pub fn update_cell(&mut self, row_idx: u64, col_idx: usize, value: Value) -> Result<(), StorageError> {
        if col_idx >= self.columns.len() {
            return Err(StorageError::Page(format!("Column index {col_idx} out of range")));
        }
        if row_idx >= self.num_rows {
            return Err(StorageError::Page(format!(
                "Row index {row_idx} out of range (num_rows={})",
                self.num_rows
            )));
        }
        self.persistence_dirty = true;

        let mut offset = 0u64;
        for group in &mut self.node_groups {
            if row_idx < offset + group.num_nodes {
                let local_row = (row_idx - offset) as usize;
                if let Some(col_chunk) = group.columns.get_mut(col_idx) {
                    col_chunk.set_value(local_row, value)?;
                }
                return Ok(());
            }
            offset += group.num_nodes;
        }
        Err(StorageError::Page(format!(
            "Row index {row_idx} not found in any node group"
        )))
    }

    /// Delete a row by its index. Marks the row as null by setting all its column
    /// values to `Value::Null`. This is a soft delete — the row slot remains.
    pub fn delete_row(&mut self, row_idx: u64) -> Result<(), StorageError> {
        self.delete_row_with_txn(row_idx, None)
    }

    /// Delete a row with optional MVCC tracking.
    ///
    /// When `txn_id` is `Some(...)`, the delete is recorded in VersionInfo
    /// for MVCC snapshot isolation.
    pub fn delete_row_with_txn(&mut self, row_idx: u64, txn_id: Option<u64>) -> Result<(), StorageError> {
        if row_idx >= self.num_rows {
            return Err(StorageError::Page(format!(
                "Row index {row_idx} out of range (num_rows={})",
                self.num_rows
            )));
        }
        self.persistence_dirty = true;

        // Locate the node group containing this row
        let mut offset = 0u64;
        for group in &mut self.node_groups {
            if row_idx < offset + group.num_nodes {
                let local_row = (row_idx - offset) as usize;
                // Record delete in VersionInfo if MVCC tracking is active
                if let Some(txn) = txn_id {
                    if let Some(ref vi) = group.version_info {
                        vi.delete(txn, local_row as u32);
                    }
                }
                // Capture the PK value before it is nulled so the in-memory
                // PK indexes can be kept in sync (P52.16).
                let pk_value = (self.primary_key_column < self.columns.len())
                    .then(|| group.columns.get(self.primary_key_column))
                    .flatten()
                    .and_then(|chunk| chunk.get(local_row))
                    .cloned();
                // Set all columns to Null for this row
                for col_chunk in &mut group.columns {
                    let _ = col_chunk.set_value(local_row, Value::Null);
                }
                // Soft-deleted rows must no longer resolve via PK lookup, and
                // the same PK must be re-insertable. Drop it from the hash and
                // ART indexes (P52.16).
                if let Some(pk) = pk_value
                    && !matches!(pk, Value::Null)
                {
                    let pk_key = pk_value_to_string(&pk);
                    self.hash_index.delete(&pk_key);
                    if let Some(ref mut art_idx) = self.art_index
                        && let Some(art_key) = ArtKey::from_value(&pk)
                    {
                        art_idx.delete(&art_key, row_idx);
                    }
                }
                return Ok(());
            }
            offset += group.num_nodes;
        }
        Err(StorageError::Page(format!(
            "Row index {row_idx} not found in any node group"
        )))
    }

    /// Capture the full row (all columns) as serialized undo bytes.
    /// Used by the write path to record `UndoType::Delete` records so a
    /// rollback can restore a soft-deleted row (P52.18).
    pub fn row_undo_bytes(&self, row_idx: u64) -> Vec<u8> {
        let mut out = Vec::new();
        for col in 0..self.columns.len() {
            let val = self.get_value(row_idx as usize, col).cloned().unwrap_or(Value::Null);
            out.extend_from_slice(&Column::serialize_value(&val));
        }
        out
    }

    /// Capture a single cell as serialized undo bytes.
    /// Used to record `UndoType::Update` records for `SET` rollback (P52.18).
    pub fn cell_undo_bytes(&self, row_idx: u64, col_idx: usize) -> Vec<u8> {
        let val = self
            .get_value(row_idx as usize, col_idx)
            .cloned()
            .unwrap_or(Value::Null);
        Column::serialize_value(&val)
    }

    /// Get a single value at (row, col) by locating the correct `NodeGroup`
    /// and `ColumnChunk`.
    pub fn get_value(&self, row: usize, col: usize) -> Option<&Value> {
        self.get_value_with_snapshot(row, col, None, &[])
    }

    /// Get a single value with MVCC snapshot isolation.
    ///
    /// Checks `VersionInfo` for insert/delete visibility and `UpdateInfo`
    /// version chains when `snapshot_ts` is provided.
    pub fn get_value_with_snapshot(
        &self,
        row: usize,
        col: usize,
        snapshot_ts: Option<u64>,
        commit_history: &[(u64, u64)],
    ) -> Option<&Value> {
        if col >= self.columns.len() || row as u64 >= self.num_rows {
            return None;
        }
        let group_idx = self.find_group(row as u64);
        let group = self.node_groups.get(group_idx)?;
        let local_row = row as u64 - group.start_offset;
        group.get_value_with_snapshot(local_row as usize, col, snapshot_ts, commit_history)
    }

    /// Reconstruct column-major data (`Vec<Vec<Value>>`) from all node groups.
    ///
    /// Used by the processor (`resolve_scan_data`) for backward compatibility.
    pub fn to_column_major_data(&self) -> Vec<Vec<Value>> {
        self.to_column_major_data_with_predicate(None)
    }

    /// Like `to_column_major_data`, but applies an optional zone map predicate
    /// `(col_idx, op_string, val)` to skip entire node groups.
    pub fn to_column_major_data_with_predicate(&self, predicate: Option<(usize, &str, &Value)>) -> Vec<Vec<Value>> {
        let num_cols = self.columns.len();
        let mut result = vec![Vec::new(); num_cols]; // Avoid allocating self.num_rows if we skip chunks

        for group in &self.node_groups {
            if let Some((col_idx, op, val)) = predicate
                && let Some(col_chunk) = group.columns.get(col_idx)
            {
                use crate::predicate::{ZoneMapCheckResult, check_zone_map};
                if check_zone_map(&col_chunk.stats, op, val) == ZoneMapCheckResult::SkipScan {
                    continue; // Skip this entire node group
                }
            }

            for row in 0..group.num_nodes as usize {
                for (col, res_col) in result.iter_mut().enumerate().take(num_cols) {
                    match group.get_value(row, col) {
                        Some(v) => res_col.push(v.clone()),
                        None => res_col.push(Value::Null),
                    }
                }
            }
        }

        result
    }

    /// Like `to_column_major_data`, but with MVCC snapshot isolation.
    ///
    /// When `snapshot_ts` is `Some(...)`, rows inserted/deleted by transactions
    /// committed after `snapshot_ts` are excluded, and versioned updates are
    /// resolved to the value visible at that snapshot.
    pub fn to_column_major_data_with_snapshot(
        &self,
        snapshot_ts: Option<u64>,
        commit_history: &[(u64, u64)],
    ) -> Vec<Vec<Value>> {
        let num_cols = self.columns.len();
        let mut result = vec![Vec::new(); num_cols];

        for group in &self.node_groups {
            for row in 0..group.num_nodes as usize {
                for (col, res_col) in result.iter_mut().enumerate().take(num_cols) {
                    match group.get_value_owned_with_snapshot(row, col, snapshot_ts, commit_history) {
                        Some(v) => res_col.push(v),
                        None => res_col.push(Value::Null),
                    }
                }
            }
        }

        result
    }

    /// Like `to_column_major_data_with_snapshot`, but applies an optional zone
    /// map predicate to skip entire node groups.
    pub fn to_column_major_data_with_snapshot_and_predicate(
        &self,
        predicate: Option<(usize, &str, &Value)>,
        snapshot_ts: Option<u64>,
        commit_history: &[(u64, u64)],
    ) -> Vec<Vec<Value>> {
        let num_cols = self.columns.len();
        let mut result = vec![Vec::new(); num_cols];

        for group in &self.node_groups {
            if let Some((col_idx, op, val)) = predicate
                && let Some(col_chunk) = group.columns.get(col_idx)
            {
                use crate::predicate::{ZoneMapCheckResult, check_zone_map};
                if check_zone_map(&col_chunk.stats, op, val) == ZoneMapCheckResult::SkipScan {
                    continue;
                }
            }

            for row in 0..group.num_nodes as usize {
                for (col, res_col) in result.iter_mut().enumerate().take(num_cols) {
                    match group.get_value_owned_with_snapshot(row, col, snapshot_ts, commit_history) {
                        Some(v) => res_col.push(v),
                        None => res_col.push(Value::Null),
                    }
                }
            }
        }

        result
    }

    /// Like `to_column_major_data_with_predicate`, but additionally returns the
    /// internal node id (global row offset) for every emitted row, in the same
    /// order as the returned column data. This is the node id space used by the
    /// processor's extend/insert/join operators (`<var>._id`).
    pub fn to_column_major_data_with_predicate_and_ids(
        &self,
        predicate: Option<(usize, &str, &Value)>,
    ) -> (Vec<Vec<Value>>, Vec<u64>) {
        let num_cols = self.columns.len();
        let mut result = vec![Vec::new(); num_cols];
        let mut ids = Vec::new();

        for group in &self.node_groups {
            if let Some((col_idx, op, val)) = predicate
                && let Some(col_chunk) = group.columns.get(col_idx)
            {
                use crate::predicate::{ZoneMapCheckResult, check_zone_map};
                if check_zone_map(&col_chunk.stats, op, val) == ZoneMapCheckResult::SkipScan {
                    continue;
                }
            }

            for row in 0..group.num_nodes as usize {
                ids.push(group.start_offset + row as u64);
                for (col, res_col) in result.iter_mut().enumerate().take(num_cols) {
                    match group.get_value(row, col) {
                        Some(v) => res_col.push(v.clone()),
                        None => res_col.push(Value::Null),
                    }
                }
            }
        }

        (result, ids)
    }

    /// Like `to_column_major_data_with_snapshot_and_predicate`, but additionally
    /// returns the internal node id (global row offset) for every emitted row,
    /// in the same order as the returned column data.
    pub fn to_column_major_data_with_snapshot_and_predicate_and_ids(
        &self,
        predicate: Option<(usize, &str, &Value)>,
        snapshot_ts: Option<u64>,
        commit_history: &[(u64, u64)],
    ) -> (Vec<Vec<Value>>, Vec<u64>) {
        let num_cols = self.columns.len();
        let mut result = vec![Vec::new(); num_cols];
        let mut ids = Vec::new();

        for group in &self.node_groups {
            if let Some((col_idx, op, val)) = predicate
                && let Some(col_chunk) = group.columns.get(col_idx)
            {
                use crate::predicate::{ZoneMapCheckResult, check_zone_map};
                if check_zone_map(&col_chunk.stats, op, val) == ZoneMapCheckResult::SkipScan {
                    continue;
                }
            }

            for row in 0..group.num_nodes as usize {
                ids.push(group.start_offset + row as u64);
                for (col, res_col) in result.iter_mut().enumerate().take(num_cols) {
                    match group.get_value_owned_with_snapshot(row, col, snapshot_ts, commit_history) {
                        Some(v) => res_col.push(v),
                        None => res_col.push(Value::Null),
                    }
                }
            }
        }

        (result, ids)
    }

    /// Binary-search for the node group that contains `row`.
    fn find_group(&self, row: u64) -> usize {
        match self.node_groups.binary_search_by_key(&row, |g| g.start_offset) {
            Ok(i) => i,
            Err(i) => {
                if i == 0 {
                    0
                } else {
                    i - 1
                }
            }
        }
    }
}

/// Convert a Value to its string representation for use as a hash index key.
fn pk_value_to_string(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int64(i) => i.to_string(),
        Value::Int32(i) => i.to_string(),
        Value::Int16(i) => i.to_string(),
        Value::Int8(i) => i.to_string(),
        Value::UInt64(u) => u.to_string(),
        Value::UInt32(u) => u.to_string(),
        Value::UInt16(u) => u.to_string(),
        Value::UInt8(u) => u.to_string(),
        Value::Double(f) => f.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => s.clone(),
        Value::Date(d) => format!("Date({})", d.0),
        Value::Timestamp(ts) => format!("Timestamp({})", ts.0),
        other => format!("{other:?}"),
    }
}

/// Coerce a row's values to the table's declared column logical types.
///
/// Constant evaluation (e.g. CREATE `{id: 41}`) produces `Value::Int64`
/// literals regardless of the target column type. A UINT64 column must store
/// `Value::UInt64`: the scan builds its Arrow type from the physical type of
/// the column, and the ART primary-key index encodes signed and unsigned keys
/// differently. Mixed signed/unsigned keys would corrupt range scans.
fn coerce_values_to_columns(values: &mut [Value], columns: &[ColumnDefinition]) -> Result<(), StorageError> {
    for (i, col) in columns.iter().enumerate() {
        let coerced = match (col.logical_type, &values[i]) {
            (LogicalTypeID::UInt64, Value::Int64(x)) if *x >= 0 => Some(Value::UInt64(*x as u64)),
            (LogicalTypeID::UInt64, Value::Int32(x)) if *x >= 0 => Some(Value::UInt64(*x as u64)),
            (LogicalTypeID::UInt64, Value::Int16(x)) if *x >= 0 => Some(Value::UInt64(*x as u64)),
            (LogicalTypeID::UInt64, Value::Int8(x)) if *x >= 0 => Some(Value::UInt64(*x as u64)),
            (LogicalTypeID::UInt64, Value::UInt32(x)) => Some(Value::UInt64(*x as u64)),
            (LogicalTypeID::UInt64, Value::UInt16(x)) => Some(Value::UInt64(*x as u64)),
            (LogicalTypeID::UInt64, Value::UInt8(x)) => Some(Value::UInt64(*x as u64)),
            (LogicalTypeID::UInt64, Value::Int64(_))
            | (LogicalTypeID::UInt64, Value::Int32(_))
            | (LogicalTypeID::UInt64, Value::Int16(_))
            | (LogicalTypeID::UInt64, Value::Int8(_)) => {
                return Err(StorageError::Page(format!(
                    "Cannot store negative value in UINT64 column '{}'",
                    col.name
                )));
            }
            _ => None,
        };
        if let Some(c) = coerced {
            values[i] = c;
        }
    }
    Ok(())
}

/// A relationship (edge) table with CSR (Compressed Sparse Row) adjacency storage.
///
/// Each edge connects a source node to a destination node and may carry
/// a set of property values (one per column in `columns`).
///
/// # Storage layout
///
/// - `edges` — flat edge list: `edge_idx → (src_offset, dst_offset)`
/// - `fwd_adj` — forward index: `src_offset → Vec<(dst_offset, edge_idx)>`
/// - `rev_adj` — reverse index: `dst_offset → Vec<(src_offset, edge_idx)>`
/// - `properties` — column-major property storage: `properties[col_idx][edge_idx]`
#[derive(Debug, Clone)]
pub struct RelTable {
    pub table_id: u64,
    pub name: String,
    pub src_table_id: u64,
    pub dst_table_id: u64,
    pub columns: Vec<ColumnDefinition>,
    pub num_rows: u64,
    /// Flat edge list: edge_idx → (src_offset, dst_offset).
    pub edges: Vec<(u64, u64)>,
    /// Forward CSR adjacency: src_offset → [(dst_offset, edge_idx), ...].
    pub fwd_adj: HashMap<u64, Vec<(u64, usize)>>,
    /// Reverse CSR adjacency: dst_offset → [(src_offset, edge_idx), ...].
    pub rev_adj: HashMap<u64, Vec<(u64, usize)>>,
    /// Specialized CSR Index for fast graph traversals.
    pub csr_index: Option<CsrIndex>,
    /// Column-major property storage: properties[col_idx][edge_idx].
    pub properties: Vec<Vec<Value>>,
    /// Set when an UPDATE/DELETE touches the table. The durable column mirror
    /// (see `persistence.rs`) performs a full rewrite when this flag is set.
    pub persistence_dirty: bool,
}

impl RelTable {
    pub fn new(
        table_id: u64,
        name: String,
        src_table_id: u64,
        dst_table_id: u64,
        columns: Vec<ColumnDefinition>,
    ) -> Self {
        let num_cols = columns.len();
        Self {
            table_id,
            name,
            src_table_id,
            dst_table_id,
            columns,
            num_rows: 0,
            edges: Vec::new(),
            fwd_adj: HashMap::new(),
            rev_adj: HashMap::new(),
            csr_index: None,
            properties: vec![Vec::new(); num_cols],
            persistence_dirty: false,
        }
    }

    /// Insert a relationship (edge) between two nodes with property values.
    ///
    /// `from` and `to` are the node offsets of the source and destination
    /// nodes within their respective tables.
    ///
    /// Returns an error if the number of values doesn't match the number
    /// of property columns.
    pub fn insert_rel(&mut self, from: u64, to: u64, values: Vec<Value>) -> Result<(), StorageError> {
        if values.len() != self.columns.len() {
            return Err(StorageError::Page(format!(
                "Column count mismatch: expected {} values, got {}",
                self.columns.len(),
                values.len()
            )));
        }

        let edge_idx = self.edges.len();
        self.edges.push((from, to));

        // Update forward adjacency.
        self.fwd_adj.entry(from).or_default().push((to, edge_idx));

        // Update reverse adjacency.
        self.rev_adj.entry(to).or_default().push((from, edge_idx));
        // Store property values.
        for (col_idx, val) in values.into_iter().enumerate() {
            self.properties[col_idx].push(val);
        }
        self.num_rows += 1;
        Ok(())
    }

    /// Batch insert multiple relations efficiently.
    /// Each tuple is (from_offset, to_offset, property_values).
    pub fn insert_rels_batch(&mut self, rels: &[(u64, u64, Vec<Value>)]) -> Result<u64, StorageError> {
        if rels.is_empty() {
            return Ok(0);
        }
        let num_cols = self.columns.len();
        let total = rels.len();

        // Validate all rows
        for (i, (_, _, vals)) in rels.iter().enumerate() {
            if vals.len() != num_cols {
                return Err(StorageError::Page(format!(
                    "Rel {} column count mismatch: expected {} values, got {}",
                    i,
                    num_cols,
                    vals.len()
                )));
            }
        }

        // Pre-allocate
        self.edges.reserve(total);
        for col in &mut self.properties {
            col.reserve(total);
        }

        let _start_edge_idx = self.edges.len();

        // Batch append
        for (from, to, vals) in rels {
            let edge_idx = self.edges.len();
            self.edges.push((*from, *to));
            self.fwd_adj.entry(*from).or_default().push((*to, edge_idx));
            self.rev_adj.entry(*to).or_default().push((*from, edge_idx));
            for (col_idx, val) in vals.iter().enumerate() {
                self.properties[col_idx].push(val.clone());
            }
        }

        self.num_rows += total as u64;
        Ok(total as u64)
    }

    /// Delete an edge by its index. Marks the edge as deleted by removing it from adjacency lists
    /// and setting its properties to Null.
    pub fn delete_edge(&mut self, edge_idx: usize) -> Result<(), StorageError> {
        if edge_idx >= self.edges.len() {
            return Err(StorageError::Page(format!("Edge index {edge_idx} out of range")));
        }

        let (src, dst) = self.edges[edge_idx];
        if src == u64::MAX {
            // Already deleted
            return Ok(());
        }

        // Remove from fwd_adj
        if let Some(adj) = self.fwd_adj.get_mut(&src) {
            adj.retain(|&(_, idx)| idx != edge_idx);
        }

        // Remove from rev_adj
        if let Some(adj) = self.rev_adj.get_mut(&dst) {
            adj.retain(|&(_, idx)| idx != edge_idx);
        }

        // Tombstone the edge
        self.edges[edge_idx] = (u64::MAX, u64::MAX);
        self.persistence_dirty = true;

        // Nullify properties
        for col in &mut self.properties {
            if edge_idx < col.len() {
                col[edge_idx] = Value::Null;
            }
        }

        Ok(())
    }

    /// Update a single cell (edge property) with a new value.
    pub fn update_cell(&mut self, edge_idx: usize, col_idx: usize, value: Value) -> Result<(), StorageError> {
        if col_idx >= self.columns.len() {
            return Err(StorageError::Page(format!("Column index {col_idx} out of range")));
        }
        if edge_idx >= self.properties[col_idx].len() {
            return Err(StorageError::Page(format!("Edge index {edge_idx} out of range")));
        }

        self.properties[col_idx][edge_idx] = value;
        self.persistence_dirty = true;
        Ok(())
    }

    /// Capture an edge (src, dst) plus all property values as serialized undo
    /// bytes: `[src, dst, prop0..propN]`. Used to record `UndoType::Delete`
    /// records so a rollback can restore a deleted edge (P52.18).
    pub fn edge_undo_bytes(&self, edge_idx: usize) -> Vec<u8> {
        let (src, dst) = self.edges.get(edge_idx).copied().unwrap_or((u64::MAX, u64::MAX));
        let mut out = Vec::new();
        out.extend_from_slice(&Column::serialize_value(&Value::UInt64(src)));
        out.extend_from_slice(&Column::serialize_value(&Value::UInt64(dst)));
        for p in self.get_edge_properties(edge_idx) {
            out.extend_from_slice(&Column::serialize_value(&p));
        }
        out
    }

    /// Capture a single edge property as serialized undo bytes.
    /// Used to record `UndoType::Update` records for `SET` rollback (P52.18).
    pub fn edge_cell_undo_bytes(&self, edge_idx: usize, col_idx: usize) -> Vec<u8> {
        let v = self
            .get_edge_properties(edge_idx)
            .get(col_idx)
            .cloned()
            .unwrap_or(Value::Null);
        Column::serialize_value(&v)
    }

    /// Restore a tombstoned edge (rollback of a `DELETE` edge). Re-adds the
    /// edge to the forward/reverse adjacency lists and restores its properties
    /// (P52.18).
    pub fn restore_deleted_edge(
        &mut self,
        edge_idx: usize,
        src: u64,
        dst: u64,
        props: Vec<Value>,
    ) -> Result<(), StorageError> {
        if edge_idx >= self.edges.len() {
            return Err(StorageError::Page(format!("Edge index {edge_idx} out of range")));
        }
        self.edges[edge_idx] = (src, dst);
        self.fwd_adj.entry(src).or_default().push((dst, edge_idx));
        self.rev_adj.entry(dst).or_default().push((src, edge_idx));
        for (col_idx, val) in props.into_iter().enumerate() {
            if col_idx < self.properties.len() {
                if edge_idx < self.properties[col_idx].len() {
                    self.properties[col_idx][edge_idx] = val;
                } else {
                    self.properties[col_idx].push(val);
                }
            }
        }
        self.persistence_dirty = true;
        Ok(())
    }

    /// Insert a row of values (legacy alias that treats all columns as properties).
    /// Only the first two values are treated as (from, to) if the table has
    /// at least 2 columns; otherwise they are stored as pure properties.
    pub fn insert_row(&mut self, values: Vec<Value>) -> Result<u64, StorageError> {
        // If there are at least 2 "structural" columns (src_id, dst_id) plus
        // property columns, we assume the first two values are the node offsets.
        // This preserves backward compatibility with the old flat API.
        let num_prop_cols = self.columns.len();
        if values.len() != num_prop_cols {
            return Err(StorageError::Page(format!(
                "Column count mismatch: expected {} values, got {}",
                num_prop_cols,
                values.len()
            )));
        }

        // We treat the values as plain properties and use sequential edge IDs
        // as (from, to) placeholders. Real callers should use `insert_rel`.
        let from = self.num_rows;
        let to = self.num_rows;
        self.insert_rel(from, to, values)?;
        Ok(0) // insert_row on RelTable doesn't have a meaningful row offset right now
    }

    /// Scan the forward adjacency list for a given source node.
    ///
    /// Returns a list of `(dst_offset, edge_idx)` pairs, or an empty vec
    /// if the node has no outgoing edges.
    pub fn scan_adj_list(&self, src_offset: u64) -> &[(u64, usize)] {
        self.fwd_adj.get(&src_offset).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Scan the reverse adjacency list for a given destination node.
    ///
    /// Returns a list of `(src_offset, edge_idx)` pairs, or an empty vec
    /// if the node has no incoming edges.
    pub fn scan_rev_adj_list(&self, dst_offset: u64) -> &[(u64, usize)] {
        self.rev_adj.get(&dst_offset).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Get all outgoing edges from a source node as `(dst_offset, property_values)`.
    pub fn get_outgoing_edges(&self, src_offset: u64) -> Vec<(u64, Vec<Value>)> {
        self.scan_adj_list(src_offset)
            .iter()
            .map(|&(dst, edge_idx)| {
                let props = self.get_edge_properties(edge_idx);
                (dst, props)
            })
            .collect()
    }

    /// Get all incoming edges to a destination node as `(src_offset, property_values)`.
    pub fn get_incoming_edges(&self, dst_offset: u64) -> Vec<(u64, Vec<Value>)> {
        self.scan_rev_adj_list(dst_offset)
            .iter()
            .map(|&(src, edge_idx)| {
                let props = self.get_edge_properties(edge_idx);
                (src, props)
            })
            .collect()
    }

    /// Get the property values for a specific edge by index.
    pub fn get_edge_properties(&self, edge_idx: usize) -> Vec<Value> {
        let mut props = Vec::with_capacity(self.columns.len());
        for col in &self.properties {
            match col.get(edge_idx) {
                Some(v) => props.push(v.clone()),
                None => props.push(Value::Null),
            }
        }
        props
    }

    /// Get all values for a given property column (by index) as a slice.
    pub fn get_column(&self, col_idx: usize) -> Option<&[Value]> {
        self.properties.get(col_idx).map(|v| v.as_slice())
    }

    /// Reconstruct column-major data from properties for backward compatibility.
    pub fn to_column_major_data(&self) -> Vec<Vec<Value>> {
        self.properties.clone()
    }
}

/// A collection of tables managed by the storage engine.
///
/// Uses `DashMap` internally for lock-free concurrent reads.
/// Write operations synchronize on individual entries rather than
/// the entire catalog, allowing concurrent writers to different
/// tables to proceed in parallel.
#[derive(Debug, Default)]
pub struct TableCatalog {
    node_tables: DashMap<u64, NodeTable>,
    rel_tables: DashMap<u64, RelTable>,
    vector_indexes: DashMap<u64, VectorIndexTable>,
    /// Map from table name to table ID for node tables.
    node_name_to_id: DashMap<String, u64>,
    /// Map from table name to table ID for rel tables.
    rel_name_to_id: DashMap<String, u64>,
    /// Map from index name to index ID for vector indexes.
    vector_index_name_to_id: DashMap<String, u64>,
    next_table_id: std::sync::atomic::AtomicU64,
}

impl TableCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_node_table(&self, name: String, columns: Vec<ColumnDefinition>) -> NodeTable {
        let table_id = self.next_table_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let table = NodeTable::new(table_id, name.clone(), columns);
        self.node_name_to_id.insert(name, table_id);
        self.node_tables.insert(table_id, table.clone());
        table
    }

    /// Recreate a node table at a specific table ID (used when restoring a
    /// persisted catalog during recovery). Advances `next_table_id` so that
    /// subsequent auto-assigned IDs never collide with restored ones.
    pub fn create_node_table_with_id(&self, table_id: u64, name: String, columns: Vec<ColumnDefinition>) -> NodeTable {
        self.bump_next_table_id(table_id);
        let table = NodeTable::new(table_id, name.clone(), columns);
        self.node_name_to_id.insert(name, table_id);
        self.node_tables.insert(table_id, table.clone());
        table
    }

    pub fn create_rel_table(
        &self,
        name: String,
        src_table_id: u64,
        dst_table_id: u64,
        columns: Vec<ColumnDefinition>,
    ) -> RelTable {
        let table_id = self.next_table_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let table = RelTable::new(table_id, name.clone(), src_table_id, dst_table_id, columns);
        self.rel_name_to_id.insert(name, table_id);
        self.rel_tables.insert(table_id, table.clone());
        table
    }

    /// Recreate a rel table at a specific table ID (used when restoring a
    /// persisted catalog during recovery). Advances `next_table_id` so that
    /// subsequent auto-assigned IDs never collide with restored ones.
    pub fn create_rel_table_with_id(
        &self,
        table_id: u64,
        name: String,
        src_table_id: u64,
        dst_table_id: u64,
        columns: Vec<ColumnDefinition>,
    ) -> RelTable {
        self.bump_next_table_id(table_id);
        let table = RelTable::new(table_id, name.clone(), src_table_id, dst_table_id, columns);
        self.rel_name_to_id.insert(name, table_id);
        self.rel_tables.insert(table_id, table.clone());
        table
    }

    /// Advance `next_table_id` to be strictly greater than `table_id` so
    /// restored IDs are never re-issued by `create_node_table`/`create_rel_table`.
    fn bump_next_table_id(&self, table_id: u64) {
        let mut next = self.next_table_id.load(std::sync::atomic::Ordering::SeqCst);
        while next <= table_id {
            match self.next_table_id.compare_exchange(
                next,
                table_id + 1,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(current) => next = current,
            }
        }
    }

    pub fn get_node_table(&self, table_id: u64) -> Option<dashmap::mapref::one::Ref<'_, u64, NodeTable>> {
        self.node_tables.get(&table_id)
    }

    pub fn get_node_table_mut(&self, table_id: u64) -> Option<dashmap::mapref::one::RefMut<'_, u64, NodeTable>> {
        self.node_tables.get_mut(&table_id)
    }

    pub fn get_node_table_by_name(&self, name: &str) -> Option<dashmap::mapref::one::Ref<'_, u64, NodeTable>> {
        let id = self.node_name_to_id.get(name)?;
        self.node_tables.get(&*id)
    }

    pub fn get_node_table_by_name_mut(&self, name: &str) -> Option<dashmap::mapref::one::RefMut<'_, u64, NodeTable>> {
        let id = self.node_name_to_id.get(name)?;
        self.node_tables.get_mut(&*id)
    }

    pub fn get_rel_table(&self, table_id: u64) -> Option<dashmap::mapref::one::Ref<'_, u64, RelTable>> {
        self.rel_tables.get(&table_id)
    }

    pub fn get_rel_table_mut(&self, table_id: u64) -> Option<dashmap::mapref::one::RefMut<'_, u64, RelTable>> {
        self.rel_tables.get_mut(&table_id)
    }

    pub fn get_rel_table_by_name(&self, name: &str) -> Option<dashmap::mapref::one::Ref<'_, u64, RelTable>> {
        let id = self.rel_name_to_id.get(name)?;
        self.rel_tables.get(&*id)
    }

    pub fn get_rel_table_by_name_mut(&self, name: &str) -> Option<dashmap::mapref::one::RefMut<'_, u64, RelTable>> {
        let id = self.rel_name_to_id.get(name)?;
        self.rel_tables.get_mut(&*id)
    }

    /// Check if a node has any incident edges.
    pub fn has_incident_edges(&self, table_id: u64, node_idx: u64) -> bool {
        for rel_table in self.rel_tables.iter() {
            if rel_table.src_table_id == table_id {
                if let Some(edges) = rel_table.fwd_adj.get(&node_idx) {
                    if !edges.is_empty() {
                        return true;
                    }
                }
            }
            if rel_table.dst_table_id == table_id {
                if let Some(edges) = rel_table.rev_adj.get(&node_idx) {
                    if !edges.is_empty() {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Delete all incident edges for a given node.
    pub fn detach_node(&self, table_id: u64, node_idx: u64) {
        for mut rel_table in self.rel_tables.iter_mut() {
            let mut edges_to_delete = Vec::new();

            if rel_table.src_table_id == table_id {
                if let Some(edges) = rel_table.fwd_adj.get(&node_idx) {
                    for &(_, edge_idx) in edges {
                        edges_to_delete.push(edge_idx);
                    }
                }
            }

            if rel_table.dst_table_id == table_id {
                if let Some(edges) = rel_table.rev_adj.get(&node_idx) {
                    for &(_, edge_idx) in edges {
                        edges_to_delete.push(edge_idx);
                    }
                }
            }

            for edge_idx in edges_to_delete {
                let _ = rel_table.delete_edge(edge_idx);
            }
        }
    }

    pub fn all_node_tables(&self) -> Vec<dashmap::mapref::multiple::RefMulti<'_, u64, NodeTable>> {
        self.node_tables.iter().collect()
    }

    pub fn all_rel_tables(&self) -> Vec<dashmap::mapref::multiple::RefMulti<'_, u64, RelTable>> {
        self.rel_tables.iter().collect()
    }

    /// Get the number of rows in a node table by name.
    pub fn node_table_num_rows(&self, name: &str) -> u64 {
        self.get_node_table_by_name(name).map(|t| t.num_rows).unwrap_or(0)
    }

    /// Remove a node table by name. Returns true if the table existed.
    pub fn drop_node_table(&self, name: &str) -> bool {
        if let Some(id) = self.node_name_to_id.get(name) {
            let table_id = *id;
            drop(id);
            self.node_name_to_id.remove(name);
            self.node_tables.remove(&table_id).is_some()
        } else {
            false
        }
    }

    /// Create a new vector index in the catalog.
    pub fn create_vector_index(
        &self,
        name: String,
        table_name: String,
        column_name: String,
        metric: DistanceMetric,
        dimensions: u32,
    ) -> VectorIndexTable {
        let index_id = self.next_table_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let table = VectorIndexTable::new(index_id, name.clone(), table_name, column_name, metric, dimensions);
        self.vector_index_name_to_id.insert(name, index_id);
        self.vector_indexes.insert(index_id, table.clone());
        table
    }

    /// Get a vector index by its ID.
    pub fn get_vector_index(&self, index_id: u64) -> Option<dashmap::mapref::one::Ref<'_, u64, VectorIndexTable>> {
        self.vector_indexes.get(&index_id)
    }

    /// Get a vector index by name.
    pub fn get_vector_index_by_name(&self, name: &str) -> Option<dashmap::mapref::one::Ref<'_, u64, VectorIndexTable>> {
        let id = self.vector_index_name_to_id.get(name)?;
        self.vector_indexes.get(&*id)
    }

    /// Get a mutable vector index by name.
    pub fn get_vector_index_by_name_mut(
        &self,
        name: &str,
    ) -> Option<dashmap::mapref::one::RefMut<'_, u64, VectorIndexTable>> {
        let id = self.vector_index_name_to_id.get(name)?;
        self.vector_indexes.get_mut(&*id)
    }

    /// Get a mutable vector index by ID.
    pub fn get_vector_index_mut(
        &self,
        index_id: u64,
    ) -> Option<dashmap::mapref::one::RefMut<'_, u64, VectorIndexTable>> {
        self.vector_indexes.get_mut(&index_id)
    }

    /// Remove a vector index by name. Returns true if the index existed.
    pub fn drop_vector_index(&self, name: &str) -> bool {
        if let Some(id) = self.vector_index_name_to_id.get(name) {
            let index_id = *id;
            drop(id);
            self.vector_index_name_to_id.remove(name);
            self.vector_indexes.remove(&index_id).is_some()
        } else {
            false
        }
    }

    /// Get all vector indexes.
    pub fn all_vector_indexes(&self) -> Vec<dashmap::mapref::multiple::RefMulti<'_, u64, VectorIndexTable>> {
        self.vector_indexes.iter().collect()
    }

    /// Rebuild a vector index from the current contents of its node table.
    ///
    /// The HNSW graph is re-populated with the live row ids and vectors, so
    /// INSERT/DELETE after `CREATE VECTOR INDEX` are always reflected (P52.38).
    pub fn refresh_vector_index(&self, index_id: u64) {
        let (table_name, column_name) = match self.vector_indexes.get(&index_id) {
            Some(vi) => (vi.table_name.clone(), vi.column_name.clone()),
            None => return,
        };

        let col_idx = match self.get_node_table_by_name(&table_name) {
            Some(t) => match t.columns.iter().position(|c| c.name == column_name) {
                Some(idx) => idx,
                None => return,
            },
            None => return,
        };

        // Collect (row_id, vector) pairs while holding the read reference,
        // then re-insert under a single mutable borrow of the index.
        let mut data: Vec<(usize, Vec<f64>)> = Vec::new();
        if let Some(table) = self.get_node_table_by_name(&table_name) {
            for row_id in 0..table.num_rows as usize {
                if let Some(val) = table.get_value(row_id, col_idx) {
                    if let Ok(vec) = crate::extract_f64_list_from_value(val) {
                        data.push((row_id, vec));
                    }
                }
            }
        }
        if data.is_empty() {
            if let Some(mut vi) = self.vector_indexes.get_mut(&index_id) {
                vi.hnsw_mut().clear();
            }
            return;
        }

        let mut vi = self.vector_indexes.get_mut(&index_id);
        if let Some(vi) = vi.as_mut() {
            vi.hnsw_mut().clear();
            for (row_id, vec) in data {
                vi.hnsw_mut().insert(vec, row_id);
            }
        }
    }

    /// Rebuild the vector indexes of all node tables written by a statement.
    ///
    /// Called after successful writes so the on-disk/in-memory HNSW graph never
    /// serves stale or wrongly-positioned rows (P52.38).
    pub fn refresh_vector_indexes_for_tables(&self, table_ids: &[u64]) {
        for table_id in table_ids {
            let table_name = match self.get_node_table(*table_id) {
                Some(t) => t.name.clone(),
                None => continue,
            };
            let index_ids: Vec<u64> = self
                .vector_indexes
                .iter()
                .filter(|vi| vi.table_name == table_name)
                .map(|vi| *vi.key())
                .collect();
            for index_id in index_ids {
                self.refresh_vector_index(index_id);
            }
        }
    }

    /// Create an ART (Adaptive Radix Tree) index on a node table's PK column.
    ///
    /// Creates a new `ArtPrimaryKeyIndex`, backfills it with all existing rows,
    /// and attaches it to the `NodeTable`.
    ///
    /// The `index_name` is used as the BufferManager file name for persistence.
    pub fn create_art_index(&self, table_name: &str, index_name: &str) -> Result<(), StorageError> {
        let mut table = self
            .get_node_table_by_name_mut(table_name)
            .ok_or_else(|| StorageError::TableNotFound(format!("Node table '{table_name}' not found")))?;

        if table.art_index.is_some() {
            return Err(StorageError::Index(format!(
                "Table '{table_name}' already has an ART index"
            )));
        }

        let mut art_idx = ArtPrimaryKeyIndex::new(index_name);

        // Backfill existing rows
        let pk_col = table.primary_key_column;
        // Scan all rows via to_column_major_data for backfill
        let col_major = table.to_column_major_data();
        if pk_col < col_major.len() {
            for (row_offset, pk_val) in col_major[pk_col].iter().enumerate() {
                if !matches!(pk_val, Value::Null)
                    && let Some(art_key) = ArtKey::from_value(pk_val)
                {
                    art_idx.insert(&art_key, row_offset as u64);
                }
            }
        }

        table.art_index = Some(art_idx);
        Ok(())
    }

    /// Drop the ART index from a node table.
    pub fn drop_art_index(&self, table_name: &str) -> Result<(), StorageError> {
        let mut table = self
            .get_node_table_by_name_mut(table_name)
            .ok_or_else(|| StorageError::TableNotFound(format!("Node table '{table_name}' not found")))?;

        table.art_index = None;
        Ok(())
    }

    /// Get a reference to the ART index for a node table (via table name).
    /// Returns `None` if the table has no ART index or doesn't exist.
    pub fn get_art_index(&self, table_name: &str) -> Option<ArtPrimaryKeyIndex> {
        let table = self.get_node_table_by_name(table_name)?;
        table.art_index.clone()
    }

    /// Check if a node table has an ART index.
    pub fn has_art_index(&self, table_name: &str) -> bool {
        self.get_node_table_by_name(table_name)
            .map(|t| t.art_index.is_some())
            .unwrap_or(false)
    }

    /// Remove a rel table by name. Returns true if the table existed.
    pub fn drop_rel_table(&self, name: &str) -> bool {
        if let Some(id) = self.rel_name_to_id.get(name) {
            let table_id = *id;
            drop(id);
            self.rel_name_to_id.remove(name);
            self.rel_tables.remove(&table_id).is_some()
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column_chunk::NODE_GROUP_SIZE;

    // ==================== NodeTable tests ====================

    #[test]
    fn test_node_table_empty() {
        let table = NodeTable::new(
            1,
            "Person".into(),
            vec![
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "name".into(),
                    logical_type: LogicalTypeID::String,
                    is_primary_key: true,
                },
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "age".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: false,
                },
            ],
        );
        assert_eq!(table.num_rows, 0);
        assert!(table.node_groups.is_empty());
    }

    #[test]
    fn test_node_table_insert_and_get() {
        let mut table = NodeTable::new(
            1,
            "Person".into(),
            vec![
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "name".into(),
                    logical_type: LogicalTypeID::String,
                    is_primary_key: true,
                },
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "age".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: false,
                },
            ],
        );
        table
            .insert_row(vec![Value::String("Alice".into()), Value::Int64(30)])
            .unwrap();
        table
            .insert_row(vec![Value::String("Bob".into()), Value::Int64(25)])
            .unwrap();

        assert_eq!(table.num_rows, 2);
        assert_eq!(table.get_value(0, 0), Some(&Value::String("Alice".into())));
        assert_eq!(table.get_value(1, 1), Some(&Value::Int64(25)));
    }

    #[test]
    fn test_delete_row_removes_pk_from_hash_and_art_index() {
        let mut table = NodeTable::new(
            1,
            "Person".into(),
            vec![
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "name".into(),
                    logical_type: LogicalTypeID::String,
                    is_primary_key: true,
                },
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "age".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: false,
                },
            ],
        );
        table.art_index = Some(ArtPrimaryKeyIndex::new("test_art"));
        table
            .insert_row(vec![Value::String("Alice".into()), Value::Int64(30)])
            .unwrap();
        table
            .insert_row(vec![Value::String("Bob".into()), Value::Int64(25)])
            .unwrap();
        let art = table.art_index.as_ref().unwrap();
        assert_eq!(
            art.lookup(&ArtKey::from_value(&Value::String("Alice".into())).unwrap()),
            Some(0)
        );

        table.delete_row(0).unwrap();

        // Soft-deleted row must no longer resolve via PK lookup (P52.16).
        assert!(table.lookup_by_pk(&Value::String("Alice".into())).is_none());
        let art = table.art_index.as_ref().unwrap();
        assert_eq!(art.len(), 1, "ART must drop the deleted entry");
        assert!(
            art.lookup(&ArtKey::from_value(&Value::String("Alice".into())).unwrap())
                .is_none()
        );
        assert!(
            art.lookup(&ArtKey::from_value(&Value::String("Bob".into())).unwrap())
                .is_some()
        );
        // Range scan over the ART must not surface the deleted PK.
        let hits = table.lookup_by_pk_range(
            Some(&Value::String("A".into())),
            true,
            Some(&Value::String("C".into())),
            true,
            100,
        );
        assert_eq!(hits, vec![1], "only 'Bob' (row 1) should be in range");

        // Re-inserting the same PK must now succeed (was: duplicate PK error).
        table
            .insert_row(vec![Value::String("Alice".into()), Value::Int64(31)])
            .unwrap();
        assert_eq!(table.lookup_by_pk(&Value::String("Alice".into())), Some(2));
    }

    #[test]
    fn test_node_table_scan_column() {
        let mut table = NodeTable::new(
            1,
            "T".into(),
            vec![ColumnDefinition {
                compression: akar_common::enums::CompressionType::Uncompressed,
                name: "val".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: false,
            }],
        );
        for i in 0..100 {
            table.insert_row(vec![Value::Int64(i)]).unwrap();
        }
        let scanned = table.scan_column(0, 10, 5, None, &[]);
        assert_eq!(scanned.len(), 5);
        assert_eq!(scanned[0], Value::Int64(10));
        assert_eq!(scanned[4], Value::Int64(14));
    }

    #[test]
    fn test_node_table_to_column_major() {
        let mut table = NodeTable::new(
            1,
            "T".into(),
            vec![
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "x".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: false,
                },
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "y".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: false,
                },
            ],
        );
        table.insert_row(vec![Value::Int64(1), Value::Int64(10)]).unwrap();
        table.insert_row(vec![Value::Int64(2), Value::Int64(20)]).unwrap();

        let data = table.to_column_major_data();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0], vec![Value::Int64(1), Value::Int64(2)]);
        assert_eq!(data[1], vec![Value::Int64(10), Value::Int64(20)]);
    }

    #[test]
    fn test_node_table_auto_node_group() {
        let mut table = NodeTable::new(
            1,
            "T".into(),
            vec![ColumnDefinition {
                compression: akar_common::enums::CompressionType::Uncompressed,
                name: "v".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: false,
            }],
        );
        // Insert NODE_GROUP_SIZE + 1 rows to force a second node group
        for i in 0..NODE_GROUP_SIZE as u64 + 1 {
            table.insert_row(vec![Value::Int64(i as i64)]).unwrap();
        }
        assert_eq!(table.num_rows, NODE_GROUP_SIZE as u64 + 1);
        assert_eq!(table.node_groups.len(), 2);
        assert_eq!(table.node_groups[0].num_nodes, NODE_GROUP_SIZE as u64);
        assert_eq!(table.node_groups[1].num_nodes, 1);
        // Scan should still return all values
        assert_eq!(table.get_value(0, 0), Some(&Value::Int64(0)));
        assert_eq!(
            table.get_value(NODE_GROUP_SIZE, 0),
            Some(&Value::Int64(NODE_GROUP_SIZE as i64))
        );
    }

    // ==================== RelTable (CSR) tests ====================

    fn make_rel_table() -> RelTable {
        RelTable::new(
            1,
            "Knows".into(),
            0,
            1,
            vec![
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "since".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: false,
                },
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "weight".into(),
                    logical_type: LogicalTypeID::Double,
                    is_primary_key: false,
                },
            ],
        )
    }

    #[test]
    fn test_rel_table_empty() {
        let rel = make_rel_table();
        assert_eq!(rel.num_rows, 0);
        assert!(rel.edges.is_empty());
        assert!(rel.fwd_adj.is_empty());
        assert!(rel.rev_adj.is_empty());
    }

    #[test]
    fn test_rel_insert_basic() {
        let mut rel = make_rel_table();
        rel.insert_rel(0, 1, vec![Value::Int64(2020), Value::Double(0.5)])
            .unwrap();
        rel.insert_rel(0, 2, vec![Value::Int64(2021), Value::Double(0.8)])
            .unwrap();
        rel.insert_rel(1, 0, vec![Value::Int64(2020), Value::Double(0.3)])
            .unwrap();

        assert_eq!(rel.num_rows, 3);
        assert_eq!(rel.edges.len(), 3);

        // Forward adjacency from node 0
        let fwd = rel.scan_adj_list(0);
        assert_eq!(fwd.len(), 2);
        assert_eq!(fwd[0], (1, 0)); // (dst=1, edge_idx=0)
        assert_eq!(fwd[1], (2, 1)); // (dst=2, edge_idx=1)

        // Forward from node 1
        let fwd1 = rel.scan_adj_list(1);
        assert_eq!(fwd1.len(), 1);
        assert_eq!(fwd1[0], (0, 2));
    }

    #[test]
    fn test_rel_reverse_adjacency() {
        let mut rel = make_rel_table();
        rel.insert_rel(0, 5, vec![Value::Int64(2022), Value::Double(1.0)])
            .unwrap();
        rel.insert_rel(3, 5, vec![Value::Int64(2023), Value::Double(1.5)])
            .unwrap();

        // Node 5 has two incoming edges
        let rev = rel.scan_rev_adj_list(5);
        assert_eq!(rev.len(), 2);
        assert_eq!(rev[0], (0, 0));
        assert_eq!(rev[1], (3, 1));
    }

    #[test]
    fn test_rel_get_edge_properties() {
        let mut rel = make_rel_table();
        rel.insert_rel(0, 1, vec![Value::Int64(2020), Value::Double(0.5)])
            .unwrap();
        rel.insert_rel(2, 3, vec![Value::Int64(2021), Value::Double(0.9)])
            .unwrap();

        let props0 = rel.get_edge_properties(0);
        assert_eq!(props0, vec![Value::Int64(2020), Value::Double(0.5)]);

        let props1 = rel.get_edge_properties(1);
        assert_eq!(props1, vec![Value::Int64(2021), Value::Double(0.9)]);
    }

    #[test]
    fn test_rel_get_outgoing_edges() {
        let mut rel = make_rel_table();
        rel.insert_rel(0, 10, vec![Value::Int64(2020), Value::Double(1.0)])
            .unwrap();
        rel.insert_rel(0, 20, vec![Value::Int64(2021), Value::Double(2.0)])
            .unwrap();

        let outgoing = rel.get_outgoing_edges(0);
        assert_eq!(outgoing.len(), 2);
        assert_eq!(outgoing[0].0, 10);
        assert_eq!(outgoing[0].1, vec![Value::Int64(2020), Value::Double(1.0)]);
        assert_eq!(outgoing[1].0, 20);
    }

    #[test]
    fn test_rel_get_incoming_edges() {
        let mut rel = make_rel_table();
        rel.insert_rel(10, 5, vec![Value::Int64(2020), Value::Double(1.0)])
            .unwrap();
        rel.insert_rel(20, 5, vec![Value::Int64(2021), Value::Double(2.0)])
            .unwrap();

        let incoming = rel.get_incoming_edges(5);
        assert_eq!(incoming.len(), 2);
        assert_eq!(incoming[0].0, 10);
        assert_eq!(incoming[1].0, 20);
    }

    #[test]
    fn test_rel_no_edges() {
        let rel = make_rel_table();
        assert!(rel.scan_adj_list(0).is_empty());
        assert!(rel.scan_rev_adj_list(0).is_empty());
        assert!(rel.get_outgoing_edges(0).is_empty());
        assert!(rel.get_incoming_edges(0).is_empty());
    }

    #[test]
    fn test_rel_insert_row_legacy() {
        let mut rel = make_rel_table();
        // insert_row treats values as properties with sequential edge IDs
        rel.insert_row(vec![Value::Int64(2022), Value::Double(3.0)]).unwrap();
        assert_eq!(rel.num_rows, 1);
        assert_eq!(rel.edges[0], (0, 0)); // sequential from=0, to=0
        assert_eq!(rel.get_edge_properties(0), vec![Value::Int64(2022), Value::Double(3.0)]);
    }

    #[test]
    fn test_rel_wrong_column_count() {
        let mut rel = make_rel_table();
        let result = rel.insert_rel(0, 1, vec![Value::Int64(42)]); // 1 value, expected 2
        assert!(result.is_err());
    }

    #[test]
    fn test_rel_get_column() {
        let mut rel = make_rel_table();
        rel.insert_rel(0, 1, vec![Value::Int64(2020), Value::Double(1.5)])
            .unwrap();
        rel.insert_rel(1, 2, vec![Value::Int64(2021), Value::Double(2.5)])
            .unwrap();

        let since_col = rel.get_column(0).unwrap();
        assert_eq!(since_col, &[Value::Int64(2020), Value::Int64(2021)]);

        let weight_col = rel.get_column(1).unwrap();
        assert_eq!(weight_col, &[Value::Double(1.5), Value::Double(2.5)]);
    }

    #[test]
    fn test_rel_to_column_major() {
        let mut rel = make_rel_table();
        rel.insert_rel(0, 1, vec![Value::Int64(2020), Value::Double(0.5)])
            .unwrap();
        rel.insert_rel(2, 3, vec![Value::Int64(2021), Value::Double(0.9)])
            .unwrap();

        let data = rel.to_column_major_data();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0], vec![Value::Int64(2020), Value::Int64(2021)]);
        assert_eq!(data[1], vec![Value::Double(0.5), Value::Double(0.9)]);
    }

    // ==================== TableCatalog tests ====================

    #[test]
    fn test_catalog_create_and_lookup() {
        let cat = TableCatalog::new();
        let node_table = cat.create_node_table(
            "Person".into(),
            vec![ColumnDefinition {
                compression: akar_common::enums::CompressionType::Uncompressed,
                name: "id".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: true,
            }],
        );
        assert_eq!(node_table.table_id, 0);

        let rel_table = cat.create_rel_table(
            "Knows".into(),
            0,
            1,
            vec![ColumnDefinition {
                compression: akar_common::enums::CompressionType::Uncompressed,
                name: "since".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: false,
            }],
        );
        assert_eq!(rel_table.table_id, 1);

        assert!(cat.get_node_table(0).is_some());
        assert!(cat.get_rel_table(1).is_some());
        assert_eq!(cat.node_table_num_rows("Person"), 0);
    }

    #[test]
    fn test_refresh_vector_index_after_dml() {
        // P52.38: the HNSW graph used to be populated only during CREATE VECTOR
        // INDEX; rows inserted later went un-indexed. refresh_vector_indexes_for_tables
        // must rebuild the graph from the live table so post-index DML is visible.
        let cat = TableCatalog::new();
        cat.create_node_table(
            "Item".into(),
            vec![
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "id".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: true,
                },
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "embedding".into(),
                    logical_type: LogicalTypeID::List,
                    is_primary_key: false,
                },
            ],
        );

        let vec3 = |x: f64, y: f64, z: f64| Value::List(vec![Value::Double(x), Value::Double(y), Value::Double(z)]);
        {
            let mut t = cat.get_node_table_by_name_mut("Item").unwrap();
            t.insert_row(vec![Value::Int64(1), vec3(1.0, 0.0, 0.0)]).unwrap();
            t.insert_row(vec![Value::Int64(2), vec3(0.0, 1.0, 0.0)]).unwrap();
            t.insert_row(vec![Value::Int64(3), vec3(0.0, 0.0, 1.0)]).unwrap();
        }

        cat.create_vector_index(
            "item_vec".into(),
            "Item".into(),
            "embedding".into(),
            DistanceMetric::Cosine,
            3,
        );

        // Seed the index from the 3 pre-existing rows; the vector for row 2 must exist.
        cat.refresh_vector_indexes_for_tables(&[0]);
        {
            let vi = cat.get_vector_index(1).unwrap();
            assert_eq!(vi.hnsw().len(), 3);
            assert!(vi.hnsw().get_vector(2).is_some());
        }

        // Rows inserted AFTER the index was created must become searchable.
        {
            let mut t = cat.get_node_table_by_name_mut("Item").unwrap();
            t.insert_row(vec![Value::Int64(4), vec3(1.0, 1.0, 0.0)]).unwrap();
            t.insert_row(vec![Value::Int64(5), vec3(0.0, 1.0, 1.0)]).unwrap();
        }

        cat.refresh_vector_indexes_for_tables(&[0]);
        let vi = cat.get_vector_index(1).unwrap();
        assert_eq!(vi.hnsw().len(), 5);
        assert!(vi.hnsw().get_vector(4).is_some());
    }
}
