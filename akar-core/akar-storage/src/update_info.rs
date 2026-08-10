//! UpdateInfo — MVCC version chain for column updates.
//!
//! Each `ColumnChunk` can have an optional `UpdateInfo` that tracks
//! versioned updates to individual rows. On a versioned write, the old
//! value is preserved in a version chain. Readers at a given snapshot
//! timestamp traverse the chain to find the value visible to them.
//!
//! This is the Rust port of Vela C++'s `UpdateInfo` / `VectorUpdateInfo`.

use std::sync::Mutex;

/// A single node in the update version chain for one vector (1024 rows).
///
/// Each node stores the version (transaction commit timestamp) at which an
/// update takes effect, the OLD value that update replaced, and a link to
/// the previous (older) version node.
#[derive(Debug, Clone)]
pub struct VectorUpdateInfo {
    /// The commit timestamp at which this update takes effect.
    pub version: u64,
    /// The serialized value that this update replaced (for undo / snapshot reads).
    pub data: Vec<u8>,
    /// Link to the previous (older) version in the chain.
    pub prev: Option<Box<VectorUpdateInfo>>,
}

impl VectorUpdateInfo {
    pub fn new(version: u64, data: Vec<u8>, prev: Option<Box<VectorUpdateInfo>>) -> Self {
        Self { version, data, prev }
    }
}

/// Manages update version chains for a column chunk.
///
/// The structure is a per-vector (1024 rows) linked list of `VectorUpdateInfo`
/// nodes. Each update prepends a new node to the chain for the affected
/// vector, so recent versions are found first.
///
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

    /// Get the value visible to a snapshot at `snapshot_ts` for a given row.
    ///
    /// Each chain node stores the OLD value its update replaced; the base
    /// (latest) value lives in the `ColumnChunk.values` array. The value
    /// visible at `snapshot_ts` is the data of the newest node whose version
    /// is still STRICTLY GREATER than the snapshot — that node holds the value
    /// that the next update hasn't replaced yet. So the chain is walked from
    /// newest to oldest and the node immediately before the first node with
    /// `version <= snapshot_ts` is the answer.
    ///
    /// Returns `None` when every update is visible (`version <= snapshot_ts`),
    /// meaning the caller should use the base (latest) value.
    pub fn get_version(&self, row: u32, snapshot_ts: u64) -> Option<Vec<u8>> {
        let v_idx = self.vector_idx(row);
        let vectors = self.vectors.lock().unwrap();
        if v_idx >= vectors.len() {
            return None;
        }
        let mut current = vectors[v_idx].as_ref()?;
        let mut prev: Option<&VectorUpdateInfo> = None;
        loop {
            if current.version <= snapshot_ts {
                break;
            }
            prev = Some(current);
            match &current.prev {
                Some(next) => current = next,
                None => break,
            }
        }
        prev.map(|p| p.data.clone())
    }

    /// Get the most recent update data for a row (regardless of visibility).
    /// Used internally during commit to resolve the latest version.
    pub fn latest(&self, row: u32) -> Option<Vec<u8>> {
        let v_idx = self.vector_idx(row);
        let vectors = self.vectors.lock().unwrap();
        vectors
            .get(v_idx)
            .as_ref()
            .and_then(|o| o.as_ref().map(|node| node.data.clone()))
    }

    /// Number of vectors with at least one update.
    pub fn num_dirty_vectors(&self) -> usize {
        self.vectors.lock().unwrap().iter().filter(|v| v.is_some()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_info_append_and_get() {
        let ui = UpdateInfo::new(4096);

        // Update for row 5 at version 10 replaces value 0xAA.
        ui.append_update(5, 10, vec![0xAA]);

        // Before v10 the replaced value (0xAA) is still visible.
        assert_eq!(ui.get_version(5, 5), Some(vec![0xAA]));
        // At/after v10 the update is visible → base (latest) value is visible.
        assert_eq!(ui.get_version(5, 10), None);
        assert_eq!(ui.get_version(5, 20), None);
    }

    #[test]
    fn test_update_info_version_chain() {
        let ui = UpdateInfo::new(4096);

        // Row 5: value 0xAA replaced at v10, then value 0xBB replaced at v20.
        ui.append_update(5, 10, vec![0xAA]);
        ui.append_update(5, 20, vec![0xBB]);

        // Before v10 the value replaced at v10 (0xAA) is visible.
        assert_eq!(ui.get_version(5, 5), Some(vec![0xAA]));
        // Between v10 and v20 the value replaced at v20 (0xBB) is visible.
        assert_eq!(ui.get_version(5, 15), Some(vec![0xBB]));
        // At/after v20 both updates are visible → base (latest) value visible.
        assert_eq!(ui.get_version(5, 25), None);
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
