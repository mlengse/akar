//! UpdateInfo — MVCC version chain for column updates.
//!
//! Each `ColumnChunk` can have an optional `UpdateInfo` that tracks
//! versioned updates to individual rows. On a `set_value()`, the old
//! value is preserved in a version chain. Readers at a given snapshot
//! timestamp traverse the chain to find the value visible to them.
//!
//! This is the Rust port of Vela C++'s `UpdateInfo` / `VectorUpdateInfo`.

use std::sync::Mutex;

/// A single node in the update version chain for one vector (1024 rows).
///
/// Each node stores the version (transaction commit timestamp) at which
/// this update becomes visible, the new data, and a link to the previous
/// (older) version node.
#[derive(Debug, Clone)]
pub struct VectorUpdateInfo {
    /// The commit timestamp at which this version becomes visible.
    pub version: u64,
    /// The updated value.
    pub data: Vec<u8>,
    /// Link to the previous (older) version in the chain.
    pub prev: Option<Box<VectorUpdateInfo>>,
}

impl VectorUpdateInfo {
    pub fn new(version: u64, data: Vec<u8>, prev: Option<Box<VectorUpdateInfo>>) -> Self {
        Self {
            version,
            data,
            prev,
        }
    }
}

/// Manages update version chains for a column chunk.
///
/// The structure is a per-vector (1024 rows) linked list of `VectorUpdateInfo`
/// nodes. Each update prepends a new node to the chain for the affected
/// vector, so recent versions are found first.
///
/// Note: `Clone` is implemented manually because `Mutex` is not `Clone`.
/// Note: `Clone` is implemented manually because `Mutex` is not `Clone`.
#[derive(Debug)]
pub struct UpdateInfo {
    /// Per-vector version chains. Indexed by `vector_idx`.
    vectors: Mutex<Vec<Option<Box<VectorUpdateInfo>>>>,
    /// Number of rows per vector.
    vector_size: u32,
}

impl Clone for UpdateInfo {
    fn clone(&self) -> Self {
        let vectors = self.vectors.lock().unwrap().clone();
        Self {
            vectors: Mutex::new(vectors),
            vector_size: self.vector_size,
        }
    }
}

impl UpdateInfo {
    /// Create a new UpdateInfo for a column chunk with `total_rows` capacity.
    pub fn new(total_rows: usize) -> Self {
        let vector_size = 1024u32;
        let num_vectors = total_rows.div_ceil(vector_size as usize);
        Self {
            vectors: Mutex::new(vec![None; num_vectors]),
            vector_size,
        }
    }

    fn vector_idx(&self, row: u32) -> usize {
        (row / self.vector_size) as usize
    }

    /// Append an update: create a new `VectorUpdateInfo` node at the head
    /// of the chain for the vector containing `row`.
    ///
    /// `version` is the commit timestamp that will make this update visible.
    /// `data` is the serialized old data being replaced (for undo).
    pub fn append_update(&self, row: u32, version: u64, data: Vec<u8>) {
        let v_idx = self.vector_idx(row);
        let mut vectors = self.vectors.lock().unwrap();
        if v_idx >= vectors.len() {
            vectors.resize(v_idx + 1, None);
        }
        let prev = vectors[v_idx].take();
        vectors[v_idx] = Some(Box::new(VectorUpdateInfo::new(version, data, prev)));
    }

    /// Get the data that was visible at `snapshot_ts` for a given row.
    ///
    /// Traverses the update chain from newest to oldest, returning the
    /// data of the first node whose `version ≤ snapshot_ts`.
    /// Returns `None` if no update is visible (caller should use the base value).
    pub fn get_version(&self, row: u32, snapshot_ts: u64) -> Option<Vec<u8>> {
        let v_idx = self.vector_idx(row);
        let vectors = self.vectors.lock().unwrap();
        if v_idx >= vectors.len() {
            return None;
        }
        let mut current = vectors[v_idx].as_ref()?;
        // Traverse chain from newest to oldest
        loop {
            if current.version <= snapshot_ts {
                return Some(current.data.clone());
            }
            match &current.prev {
                Some(prev) => current = prev,
                None => return None,
            }
        }
    }

    /// Get the most recent update data for a row (regardless of visibility).
    /// Used internally during commit to resolve the latest version.
    pub fn latest(&self, row: u32) -> Option<Vec<u8>> {
        let v_idx = self.vector_idx(row);
        let vectors = self.vectors.lock().unwrap();
        vectors.get(v_idx).as_ref().and_then(|o| {
            o.as_ref().map(|node| node.data.clone())
        })
    }

    /// Number of vectors with at least one update.
    pub fn num_dirty_vectors(&self) -> usize {
        self.vectors
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v.is_some())
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_info_append_and_get() {
        let ui = UpdateInfo::new(4096);

        // Append update for row 5 at version 10
        ui.append_update(5, 10, vec![0xAA]);

        // Visible at version >= 10
        assert_eq!(ui.get_version(5, 10), Some(vec![0xAA]));
        assert_eq!(ui.get_version(5, 20), Some(vec![0xAA]));

        // Not visible before version 10
        assert_eq!(ui.get_version(5, 5), None);
    }

    #[test]
    fn test_update_info_version_chain() {
        let ui = UpdateInfo::new(4096);

        // Row 5: updated at v10, then again at v20
        ui.append_update(5, 10, vec![0xAA]);
        ui.append_update(5, 20, vec![0xBB]);

        // At v15, should see the v10 update
        assert_eq!(ui.get_version(5, 15), Some(vec![0xAA]));
        // At v25, should see the v20 update
        assert_eq!(ui.get_version(5, 25), Some(vec![0xBB]));
        // At v5, no update visible
        assert_eq!(ui.get_version(5, 5), None);
    }

    #[test]
    fn test_update_info_no_update() {
        let ui = UpdateInfo::new(4096);
        assert_eq!(ui.get_version(0, 100), None);
        assert_eq!(ui.latest(0), None);
    }

    #[test]
    fn test_update_info_latest() {
        let ui = UpdateInfo::new(4096);
        ui.append_update(5, 10, vec![0xAA]);
        ui.append_update(5, 20, vec![0xBB]);
        assert_eq!(ui.latest(5), Some(vec![0xBB]));
    }
}
