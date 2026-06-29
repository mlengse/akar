//! VersionInfo — per-node-group tracking of insert/delete visibility.
//!
//! Each `NodeGroup` can have an optional `VersionInfo` that tracks which
//! transactions have inserted or deleted rows within it. When reading at
//! a given snapshot timestamp, the `VersionInfo` determines whether a
//! specific row is visible.
//!
//! This is the Rust port of Vela C++'s `VersionInfo` / `VectorVersionInfo`.

use std::collections::HashMap;
use std::sync::Mutex;

/// Tracks insert/delete visibility for a single vector (1024 rows).
///
/// Uses a `Mutex`-protected map from transaction ID to a bitmap of
/// affected row indices within this vector.
#[derive(Debug)]
pub struct VectorVersionInfo {
    /// Map: transaction_id → set of inserted row indices (relative to vector).
    inserted: Mutex<HashMap<u64, Vec<u32>>>,
    /// Map: transaction_id → set of deleted row indices.
    deleted: Mutex<HashMap<u64, Vec<u32>>>,
}

impl Clone for VectorVersionInfo {
    fn clone(&self) -> Self {
        Self {
            inserted: Mutex::new(self.inserted.lock().unwrap().clone()),
            deleted: Mutex::new(self.deleted.lock().unwrap().clone()),
        }
    }
}

impl Default for VectorVersionInfo {
    fn default() -> Self {
        Self {
            inserted: Mutex::new(HashMap::new()),
            deleted: Mutex::new(HashMap::new()),
        }
    }
}

impl VectorVersionInfo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `txn_id` inserted a row at `row_in_vector`.
    pub fn insert(&self, txn_id: u64, row_in_vector: u32) {
        let mut ins = self.inserted.lock().unwrap();
        ins.entry(txn_id).or_default().push(row_in_vector);
    }

    /// Record that `txn_id` deleted a row at `row_in_vector`.
    pub fn delete(&self, txn_id: u64, row_in_vector: u32) {
        let mut del = self.deleted.lock().unwrap();
        del.entry(txn_id).or_default().push(row_in_vector);
    }

    /// Check whether a specific row is visible at the given snapshot.
    ///
    /// A row is visible if it was inserted by a committed transaction
    /// whose commit_ts ≤ snapshot_ts, and not deleted by any committed
    /// transaction whose commit_ts ≤ snapshot_ts.
    ///
    /// `commit_history` is used to look up commit timestamps for
    /// transaction IDs.
    pub fn is_visible(
        &self,
        row_in_vector: u32,
        snapshot_ts: u64,
        commit_history: &[(u64, u64)],
    ) -> bool {
        // Check deletions: if any committed txn with commit_ts ≤ snapshot_ts
        // deleted this row, it's not visible.
        if let Ok(del) = self.deleted.lock() {
            for (&txn_id, rows) in del.iter() {
                if rows.contains(&row_in_vector) {
                    if is_txn_committed_before(txn_id, snapshot_ts, commit_history) {
                        return false;
                    }
                }
            }
        }

        // Check insertions: if any committed txn with commit_ts ≤ snapshot_ts
        // inserted this row, it's visible. If no insert record, the row is
        // visible by default (pre-existing data).
        if let Ok(ins) = self.inserted.lock() {
            for (&txn_id, rows) in ins.iter() {
                if rows.contains(&row_in_vector) {
                    return is_txn_committed_before(txn_id, snapshot_ts, commit_history);
                }
            }
        }

        // No insert record — row existed before any tracked transaction.
        true
    }

    /// Number of unique transactions that inserted rows.
    pub fn num_inserters(&self) -> usize {
        self.inserted.lock().map(|m| m.len()).unwrap_or(0)
    }

    /// Number of unique transactions that deleted rows.
    pub fn num_deleters(&self) -> usize {
        self.deleted.lock().map(|m| m.len()).unwrap_or(0)
    }
}

/// Helper: check if a transaction's commit timestamp ≤ snapshot_ts.
fn is_txn_committed_before(txn_id: u64, snapshot_ts: u64, commit_history: &[(u64, u64)]) -> bool {
    for &(id, commit_ts) in commit_history {
        if id == txn_id {
            return commit_ts <= snapshot_ts;
        }
    }
    false // Not yet committed at this snapshot
}

/// Per-node-group version tracking.
///
/// Contains one `VectorVersionInfo` per vector-sized chunk of the node group.
/// A "vector" is `NODE_GROUP_SIZE / vectors` rows — typically 1024 rows each.
///
/// Note: `Clone` is implemented manually because `Mutex` is not `Clone`.
#[derive(Debug)]
pub struct VersionInfo {
    /// One VectorVersionInfo per vector in the node group.
    vectors: Vec<VectorVersionInfo>,
    /// Number of rows per vector.
    vector_size: u32,
}

impl Clone for VersionInfo {
    fn clone(&self) -> Self {
        Self {
            vectors: self.vectors.clone(),
            vector_size: self.vector_size,
        }
    }
}

impl VersionInfo {
    /// Create a new VersionInfo for a node group of `total_rows` capacity.
    pub fn new(total_rows: usize) -> Self {
        // Default vector size: 1024 (matching Vela C++ convention)
        let vector_size = 1024u32;
        let num_vectors = total_rows.div_ceil(vector_size as usize);
        let vectors = (0..num_vectors).map(|_| VectorVersionInfo::new()).collect();
        Self {
            vectors,
            vector_size,
        }
    }

    fn vector_idx(&self, row: u32) -> usize {
        (row / self.vector_size) as usize
    }

    fn row_in_vector(&self, row: u32) -> u32 {
        row % self.vector_size
    }

    /// Record an insert by `txn_id` at global `row` index.
    pub fn insert(&self, txn_id: u64, row: u32) {
        let v_idx = self.vector_idx(row);
        if v_idx < self.vectors.len() {
            self.vectors[v_idx].insert(txn_id, self.row_in_vector(row));
        }
    }

    /// Record a delete by `txn_id` at global `row` index.
    pub fn delete(&self, txn_id: u64, row: u32) {
        let v_idx = self.vector_idx(row);
        if v_idx < self.vectors.len() {
            self.vectors[v_idx].delete(txn_id, self.row_in_vector(row));
        }
    }

    /// Check whether global `row` is visible at `snapshot_ts`.
    pub fn is_visible(&self, row: u32, snapshot_ts: u64, commit_history: &[(u64, u64)]) -> bool {
        let v_idx = self.vector_idx(row);
        if v_idx < self.vectors.len() {
            self.vectors[v_idx].is_visible(self.row_in_vector(row), snapshot_ts, commit_history)
        } else {
            true // Row beyond tracked range is visible (pre-existing)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_info_basic() {
        let vi = VersionInfo::new(NODE_GROUP_SIZE);
        let history = vec![(1u64, 10u64), (2u64, 20u64)];

        vi.insert(1, 5);
        vi.delete(2, 10);

        // Row 5 was inserted by txn#1 (commit_ts=10) — visible at ts=10
        assert!(vi.is_visible(5, 10, &history));
        // Row 10 was deleted by txn#2 (commit_ts=20) — not visible at ts=20
        assert!(!vi.is_visible(10, 20, &history));
        // Row 10 visible at ts=15 (before txn#2 committed)
        assert!(vi.is_visible(10, 15, &history));
        // Row 0 has no records — visible by default
        assert!(vi.is_visible(0, 10, &history));
    }

    #[test]
    fn test_vector_version_info_insert() {
        let vvi = VectorVersionInfo::new();
        vvi.insert(1, 42);
        let history = vec![(1u64, 5u64)];

        assert!(vvi.is_visible(42, 5, &history));
        assert!(!vvi.is_visible(42, 3, &history)); // Before commit
    }

    #[test]
    fn test_vector_version_info_delete() {
        let vvi = VectorVersionInfo::new();
        vvi.delete(1, 7);
        let history = vec![(1u64, 10u64)];

        assert!(!vvi.is_visible(7, 10, &history)); // Deleted
        assert!(vvi.is_visible(7, 5, &history));   // Before delete committed
    }

    // Need NODE_GROUP_SIZE for the VersionInfo::new() test
    use crate::column_chunk::NODE_GROUP_SIZE;
}
