//! Vector index table — wraps `HnswIndex` with BufferManager-backed persistence.
//!
//! Follows the `OnDiskHashIndex` persistence pattern:
//! - Header page (metric, dims, num_vectors, entry_point, max_level)
//! - Data pages (serialized HNSW nodes + connections)
//!
//! Page layout:
//! - Page 0: Header
//! - Pages 1..=data_page_count: Node data (serialized vectors + connections)

use crate::buffer_manager::BufferManager;
use akar_common::error::StorageError;
use akar_common::types::Value;
use akar_vector::hnsw::{DistanceMetric, HnswIndex};

/// Default page size for vector index storage.
/// Header page layout (page 0):
/// - bytes 0..7:   magic number (0x484E5357 = "HNSW")
/// - bytes 8..15:  num_vectors (u64 LE)
/// - bytes 16..23: entry_point (i64 LE, -1 for none)
/// - bytes 24..27: max_level (u32 LE)
/// - bytes 28..31: dimensions (u32 LE)
/// - byte  32:     metric (0=Cosine, 1=Euclidean, 2=L1, 3=L2Squared, 4=DotProduct)
/// - byte  33:     precision (0=F32, 1=F64)
/// - bytes 34..41: conn_page_start (u64 LE)
/// - bytes 42..49: conn_page_count (u64 LE)
/// - bytes 50..51: format_version (u16 LE: 1=Legacy flat, 2=Graph-backed)
/// - bytes 52..63: reserved
pub const FORMAT_VERSION_LEGACY: u16 = 1;
pub const FORMAT_VERSION_GRAPH_V2: u16 = 2;
const HEADER_SIZE: usize = 64;
const LEGACY_HEADER_SIZE: usize = 48;

/// Vector element precision used when serializing the index to disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VectorPrecision {
    /// Native 32-bit float (current format, ~half the memory of F64).
    #[default]
    F32,
    /// Legacy 64-bit double (previous format; must be rebuilt or fail loudly).
    F64,
}

impl VectorPrecision {
    pub fn byte(self) -> u8 {
        match self {
            VectorPrecision::F32 => 0,
            VectorPrecision::F64 => 1,
        }
    }

    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(VectorPrecision::F32),
            1 => Some(VectorPrecision::F64),
            _ => None,
        }
    }
}

fn serialize_header(
    num_vectors: u64,
    entry_point: Option<usize>,
    max_level: usize,
    dimensions: u32,
    metric: &DistanceMetric,
    precision: VectorPrecision,
    conn_page_start: u64,
    conn_page_count: u64,
    format_version: u16,
) -> Vec<u8> {
    let mut buf = vec![0u8; HEADER_SIZE];
    buf[0..8].copy_from_slice(&0x484E5357u64.to_le_bytes()); // magic
    buf[8..16].copy_from_slice(&num_vectors.to_le_bytes());
    let ep = entry_point.map(|v| v as i64).unwrap_or(-1);
    buf[16..24].copy_from_slice(&ep.to_le_bytes());
    buf[24..28].copy_from_slice(&(max_level as u32).to_le_bytes());
    buf[28..32].copy_from_slice(&dimensions.to_le_bytes());
    let metric_byte = match metric {
        DistanceMetric::Cosine => 0u8,
        DistanceMetric::Euclidean => 1,
        DistanceMetric::L1 => 2,
        DistanceMetric::L2Squared => 3,
        DistanceMetric::DotProduct => 4,
    };
    buf[32] = metric_byte;
    buf[33] = precision.byte();
    buf[34..42].copy_from_slice(&conn_page_start.to_le_bytes());
    buf[42..50].copy_from_slice(&conn_page_count.to_le_bytes());
    buf[50..52].copy_from_slice(&format_version.to_le_bytes());
    buf
}

fn deserialize_header(
    buf: &[u8],
) -> Option<(
    u64,
    Option<usize>,
    usize,
    u32,
    DistanceMetric,
    VectorPrecision,
    u64,
    u64,
    u16,
)> {
    if buf.len() < LEGACY_HEADER_SIZE {
        return None;
    }
    let magic = u64::from_le_bytes(buf[0..8].try_into().ok()?);
    if magic != 0x484E5357 {
        return None;
    }
    let num_vectors = u64::from_le_bytes(buf[8..16].try_into().ok()?);
    let ep_raw = i64::from_le_bytes(buf[16..24].try_into().ok()?);
    let entry_point = if ep_raw < 0 { None } else { Some(ep_raw as usize) };
    let max_level = u32::from_le_bytes(buf[24..28].try_into().ok()?) as usize;
    let dimensions = u32::from_le_bytes(buf[28..32].try_into().ok()?);
    let metric = match buf[32] {
        0 => DistanceMetric::Cosine,
        1 => DistanceMetric::Euclidean,
        2 => DistanceMetric::L1,
        3 => DistanceMetric::L2Squared,
        4 => DistanceMetric::DotProduct,
        _ => return None,
    };
    let precision = VectorPrecision::from_byte(buf[33]).unwrap_or(VectorPrecision::F32);
    let (conn_page_start, conn_page_count, format_version) = if buf.len() >= HEADER_SIZE {
        let cp_start = u64::from_le_bytes(buf[34..42].try_into().unwrap_or([0; 8]));
        let cp_count = u64::from_le_bytes(buf[42..50].try_into().unwrap_or([0; 8]));
        let f_ver = u16::from_le_bytes(buf[50..52].try_into().unwrap_or([1, 0]));
        (cp_start, cp_count, f_ver)
    } else {
        (0, 0, FORMAT_VERSION_LEGACY)
    };
    Some((
        num_vectors,
        entry_point,
        max_level,
        dimensions,
        metric,
        precision,
        conn_page_start,
        conn_page_count,
        format_version,
    ))
}

/// A persisted vector index that wraps `HnswIndex` with BufferManager-backed storage.
///
/// # Persistence
///
/// - `save()` serializes the in-memory HNSW index to disk pages
/// - `load()` reads disk pages back into memory
/// - `flush()` writes dirty pages back to the BufferManager
#[derive(Debug, Clone)]
pub struct VectorIndexTable {
    pub index_id: u64,
    pub name: String,
    pub table_name: String,
    pub column_name: String,
    pub dimensions: u32,
    pub hnsw: HnswIndex,
    /// Precision used when serializing vectors to disk. F64 indicates a
    /// legacy index persisted by a previous version that must be rebuilt
    /// or fail loudly on load (FR-011).
    pub precision: VectorPrecision,
    /// Number of pages allocated for this index.
    page_count: u64,
    /// BufferManager file name (used for register_file / pin / unpin).
    file_name: String,
    /// Whether in-memory state has changed since last save.
    dirty: bool,
}

impl VectorIndexTable {
    /// Create a new vector index table with the given parameters.
    pub fn new(
        index_id: u64,
        name: String,
        table_name: String,
        column_name: String,
        metric: DistanceMetric,
        dimensions: u32,
    ) -> Self {
        Self {
            index_id,
            name,
            table_name,
            column_name,
            dimensions,
            hnsw: HnswIndex::new(metric),
            precision: VectorPrecision::F32,
            page_count: 1, // header page always exists
            file_name: format!("vi_{index_id}"),
            dirty: false,
        }
    }

    /// Get a reference to the underlying HNSW index.
    pub fn hnsw(&self) -> &HnswIndex {
        &self.hnsw
    }

    /// Get a mutable reference to the underlying HNSW index.
    pub fn hnsw_mut(&mut self) -> &mut HnswIndex {
        self.dirty = true;
        &mut self.hnsw
    }

    /// Get the distance metric.
    pub fn metric(&self) -> DistanceMetric {
        self.hnsw.metric()
    }

    /// Save the in-memory HNSW index to BufferManager-backed pages.
    ///
    /// Writes the header page (page 0) and all data pages. Every vector is
    /// stored whole on a single page; a vector that does not fit in one page
    /// is an error rather than a silent drop (P52.17).
    pub fn save(&mut self, bm: &mut BufferManager) -> Result<(), StorageError> {
        if !bm.is_file_registered(&self.file_name) {
            return Err(StorageError::Index(format!(
                "Vector index file '{}' not registered with BufferManager",
                self.file_name
            )));
        }

        let nodes = self.hnsw.nodes();
        let num_vectors = nodes.len() as u64;
        let entry_point = self.hnsw.entry_point();
        let max_level = self.hnsw.max_level();
        let graph_topology = self.hnsw.export_graph_topology();

        // Serialize and write vector data pages starting at page 1
        let data_page_start = 1;
        let mut page_idx = data_page_start;
        let mut offset = 0usize;

        while offset < num_vectors as usize {
            let frame = bm
                .pin_mut(&self.file_name, page_idx)
                .map_err(|e| StorageError::Index(format!("Failed to pin data page {page_idx}: {e}")))?;
            let page_data = &mut frame.data;
            let capacity = page_data.len();
            page_data.fill(0u8);
            let mut pos = 0usize;
            let mut written_this_page = 0usize;

            while offset < num_vectors as usize {
                let (id, vec_data) = nodes[offset];
                let vec_len = vec_data.len().saturating_mul(4);
                let entry_len = 8 + 4 + vec_len;
                if pos + entry_len > capacity {
                    // Doesn't fit on this page. If the page is still empty the
                    // vector is too large to persist at all — fail loudly
                    // instead of silently dropping it (P52.17).
                    if written_this_page == 0 {
                        bm.unpin(&self.file_name, page_idx);
                        return Err(StorageError::Index(format!(
                            "Vector at id {id} ({vec_len} bytes) does not fit in a {} byte page",
                            capacity
                        )));
                    }
                    break; // Continue on the next page
                }
                // Write vector ID
                page_data[pos..pos + 8].copy_from_slice(&(id as u64).to_le_bytes());
                pos += 8;
                // Write vector data length
                page_data[pos..pos + 4].copy_from_slice(&(vec_len as u32).to_le_bytes());
                pos += 4;
                // Write vector data
                let mut vec_bytes = Vec::with_capacity(vec_len);
                for &f in vec_data {
                    vec_bytes.extend_from_slice(&f.to_le_bytes());
                }
                page_data[pos..pos + vec_len].copy_from_slice(&vec_bytes);
                pos += vec_len;
                written_this_page += 1;
                offset += 1;
            }

            frame.is_dirty = true;
            bm.unpin(&self.file_name, page_idx);
            page_idx += 1;
        }

        // Serialize and write connection pages if graph topology is built
        let (conn_page_start, conn_page_count, format_version) = if let Some((_ep, _max_level, ref conns)) = graph_topology {
            let start_p = page_idx;
            let mut conn_offset = 0usize;
            let total_nodes = conns.len();

            while conn_offset < total_nodes {
                let frame = bm
                    .pin_mut(&self.file_name, page_idx)
                    .map_err(|e| StorageError::Index(format!("Failed to pin connection page {page_idx}: {e}")))?;
                let page_data = &mut frame.data;
                let capacity = page_data.len();
                page_data.fill(0u8);
                let mut pos = 0usize;
                let mut written_this_page = 0usize;

                while conn_offset < total_nodes {
                    let (id, ref levels) = conns[conn_offset];
                    let num_levels = levels.len();
                    let mut entry_len = 8 + 2;
                    for l in levels {
                        entry_len += 2 + l.len() * 8;
                    }

                    if pos + entry_len > capacity {
                        if written_this_page == 0 {
                            bm.unpin(&self.file_name, page_idx);
                            return Err(StorageError::Index(format!(
                                "Connections for node {id} ({entry_len} bytes) do not fit in a {capacity} byte page"
                            )));
                        }
                        break;
                    }

                    // Write node id
                    page_data[pos..pos + 8].copy_from_slice(&(id as u64).to_le_bytes());
                    pos += 8;
                    // Write num_levels
                    page_data[pos..pos + 2].copy_from_slice(&(num_levels as u16).to_le_bytes());
                    pos += 2;
                    // Write levels
                    for l in levels {
                        let conn_count = l.len() as u16;
                        page_data[pos..pos + 2].copy_from_slice(&conn_count.to_le_bytes());
                        pos += 2;
                        for &neighbor_id in l {
                            page_data[pos..pos + 8].copy_from_slice(&(neighbor_id as u64).to_le_bytes());
                            pos += 8;
                        }
                    }

                    written_this_page += 1;
                    conn_offset += 1;
                }

                frame.is_dirty = true;
                bm.unpin(&self.file_name, page_idx);
                page_idx += 1;
            }

            (start_p, page_idx - start_p, FORMAT_VERSION_GRAPH_V2)
        } else {
            (0, 0, FORMAT_VERSION_LEGACY)
        };

        // Write header page with complete page counts and format version
        let header = serialize_header(
            num_vectors,
            entry_point,
            max_level,
            self.dimensions,
            &self.hnsw.metric(),
            self.precision,
            conn_page_start,
            conn_page_count,
            format_version,
        );

        let frame = bm
            .pin_mut(&self.file_name, 0)
            .map_err(|e| StorageError::Index(format!("Failed to pin header page: {e}")))?;
        let data = &mut frame.data;
        let write_len = header.len().min(data.len());
        data[..write_len].copy_from_slice(&header[..write_len]);
        frame.is_dirty = true;
        bm.unpin(&self.file_name, 0);

        self.page_count = page_idx;
        self.dirty = false;
        Ok(())
    }

    /// Load the HNSW index from BufferManager-backed pages.
    ///
    /// Reads the header page and exactly `num_vectors` entries from the data
    /// pages, rebuilding or restoring the in-memory index. If the index was
    /// saved with graph persistence (`FORMAT_VERSION_GRAPH_V2`), connection
    /// pages are read and applied in O(1) time without rebuilding the graph.
    pub fn load(&mut self, bm: &mut BufferManager) -> Result<(), StorageError> {
        if !bm.is_file_registered(&self.file_name) {
            return Err(StorageError::Index(format!(
                "Vector index file '{}' not registered with BufferManager",
                self.file_name
            )));
        }

        // Read header page
        let frame = bm
            .pin(&self.file_name, 0)
            .map_err(|e| StorageError::Index(format!("Failed to pin header page: {e}")))?;
        let header_data = &frame.data;
        let (
            num_vectors,
            entry_point,
            max_level,
            dimensions,
            metric,
            precision,
            conn_page_start,
            conn_page_count,
            format_version,
        ) = deserialize_header(header_data).ok_or(StorageError::Index("Invalid vector index header".into()))?;
        self.dimensions = dimensions;
        self.precision = precision;

        // FR-011: detect a legacy 64-bit index and fail loudly with rebuild guidance.
        // The current reader only understands f32 layout; f64 data would be
        // misread as garbage, so we refuse silently degraded results.
        if precision == VectorPrecision::F64 {
            bm.unpin(&self.file_name, 0);
            return Err(StorageError::Index(format!(
                "Vector index '{}' was persisted with 64-bit (f64) vectors by a previous version. \
                 The current format stores 32-bit (f32) vectors. Drop and recreate the index \
                 (DROP VECTOR INDEX {name}; then CREATE VECTOR INDEX {name} IF NOT EXISTS ...) \
                 so it is rebuilt at 32-bit precision, or re-run the ingestion path that builds \
                 the index from stored embeddings.",
                self.name,
                name = self.name
            )));
        }
        bm.unpin(&self.file_name, 0);

        // Rebuild or restore the HNSW index
        let mut new_hnsw = HnswIndex::new(metric);
        let has_graph_persistence =
            format_version == FORMAT_VERSION_GRAPH_V2 && conn_page_count > 0 && entry_point.is_some();

        // Read data pages from page 1 onward until all vectors are loaded.
        let data_page_start = 1u64;
        let mut page_idx = data_page_start;
        let mut loaded = 0usize;

        while (loaded as u64) < num_vectors {
            let frame_result = bm.pin(&self.file_name, page_idx);
            let frame = match frame_result {
                Ok(f) => f,
                Err(_) => break, // No more pages
            };
            let page_data = &frame.data;
            let capacity = page_data.len();
            let mut pos = 0usize;
            let remaining = num_vectors - loaded as u64;
            let mut loaded_this_page = 0u64;

            while loaded_this_page < remaining && pos + 8 <= capacity {
                let id = match page_data.get(pos..pos + 8).and_then(|s| s.try_into().ok()) {
                    Some(b) => u64::from_le_bytes(b) as usize,
                    None => break,
                };
                pos += 8;

                if pos + 4 > capacity {
                    break;
                }
                let vec_len = match page_data.get(pos..pos + 4).and_then(|s| s.try_into().ok()) {
                    Some(b) => u32::from_le_bytes(b) as usize,
                    None => break,
                };
                pos += 4;

                if pos + vec_len > capacity {
                    break;
                }
                if vec_len == 0 {
                    // Zeroed page tail (padding from save) — no more real data.
                    break;
                }

                let dims = vec_len / 4;
                let mut vec_data = Vec::with_capacity(dims);
                for i in 0..dims {
                    let f = match page_data
                        .get(pos + i * 4..pos + (i + 1) * 4)
                        .and_then(|s| s.try_into().ok())
                    {
                        Some(b) => f32::from_le_bytes(b),
                        None => break,
                    };
                    vec_data.push(f);
                }
                pos += vec_len;

                if has_graph_persistence {
                    new_hnsw.insert_flat(vec_data, id);
                } else {
                    new_hnsw.insert(vec_data, id);
                }
                loaded_this_page += 1;
            }

            bm.unpin(&self.file_name, page_idx);
            page_idx += 1;
            loaded += loaded_this_page as usize;

            if loaded_this_page == 0 {
                // Empty/zeroed page — nothing more to read.
                break;
            }
            // Safety: prevent infinite loop if pages keep reading
            if page_idx > 1024 * 1024 {
                break;
            }
        }

        // Restore persisted graph connections if present
        if has_graph_persistence {
            let mut all_connections: Vec<(usize, Vec<Vec<usize>>)> = Vec::with_capacity(num_vectors as usize);
            let mut conn_p = conn_page_start;
            let end_p = conn_page_start + conn_page_count;

            while conn_p < end_p {
                let frame_result = bm.pin(&self.file_name, conn_p);
                let frame = match frame_result {
                    Ok(f) => f,
                    Err(e) => {
                        return Err(StorageError::Index(format!(
                            "Failed to pin connection page {conn_p}: {e}"
                        )));
                    }
                };
                let page_data = &frame.data;
                let capacity = page_data.len();
                let mut pos = 0usize;

                while pos + 10 <= capacity {
                    let id = match page_data.get(pos..pos + 8).and_then(|s| s.try_into().ok()) {
                        Some(b) => u64::from_le_bytes(b) as usize,
                        None => break,
                    };
                    pos += 8;

                    let num_levels = match page_data.get(pos..pos + 2).and_then(|s| s.try_into().ok()) {
                        Some(b) => u16::from_le_bytes(b) as usize,
                        None => break,
                    };
                    pos += 2;

                    // A valid node always has at least level 0 (num_levels >= 1).
                    // num_levels == 0 indicates zeroed padding tail of the page.
                    if num_levels == 0 {
                        break;
                    }

                    let mut levels = Vec::with_capacity(num_levels);
                    let mut corrupt = false;
                    for _ in 0..num_levels {
                        if pos + 2 > capacity {
                            corrupt = true;
                            break;
                        }
                        let conn_count = match page_data.get(pos..pos + 2).and_then(|s| s.try_into().ok()) {
                            Some(b) => u16::from_le_bytes(b) as usize,
                            None => {
                                corrupt = true;
                                break;
                            }
                        };
                        pos += 2;

                        if pos + conn_count * 8 > capacity {
                            corrupt = true;
                            break;
                        }

                        let mut nids = Vec::with_capacity(conn_count);
                        for _ in 0..conn_count {
                            let nid = match page_data.get(pos..pos + 8).and_then(|s| s.try_into().ok()) {
                                Some(b) => u64::from_le_bytes(b) as usize,
                                None => {
                                    corrupt = true;
                                    break;
                                }
                            };
                            pos += 8;
                            nids.push(nid);
                        }
                        levels.push(nids);
                    }

                    if corrupt {
                        break;
                    }

                    all_connections.push((id, levels));
                    if all_connections.len() >= num_vectors as usize {
                        break;
                    }
                }

                bm.unpin(&self.file_name, conn_p);
                conn_p += 1;
            }

            let ep = entry_point.unwrap_or(0);
            new_hnsw
                .import_graph_topology(ep, max_level, all_connections)
                .map_err(|e| StorageError::Index(format!("Failed to import graph topology: {e}")))?;
        }

        self.hnsw = new_hnsw;
        self.dirty = false;
        Ok(())
    }

    /// Flush dirty pages to disk via the BufferManager.
    pub fn flush(&mut self, bm: &mut BufferManager) -> Result<(), StorageError> {
        if self.dirty {
            self.save(bm)?;
        }
        bm.flush_all()
            .map_err(|e| StorageError::Index(format!("Failed to flush vector index: {e}")))
    }

    /// Register the vector index file with the BufferManager.
    pub fn register_file(&self, bm: &mut BufferManager, db_path: &std::path::Path) {
        let file_path = db_path.join(format!("{}.idx", self.file_name));
        bm.register_file(&self.file_name, file_path);
    }

    /// Check whether the in-memory state differs from disk.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
}

/// Helper: extract a `Vec<f64>` from a `Value` (expects `Value::List` of numbers).
pub fn extract_f64_list_from_value(val: &Value) -> Result<Vec<f64>, StorageError> {
    akar_common::types::extract_f64_list(val).map_err(|e| StorageError::TypeMismatch {
        expected: "numeric vector".into(),
        actual: e,
    })
}

/// Helper: extract a `Vec<f32>` from a `Value` (expects `Value::List` of numbers).
pub fn extract_f32_list_from_value(val: &Value) -> Result<Vec<f32>, StorageError> {
    akar_common::types::extract_f32_list(val).map_err(|e| StorageError::TypeMismatch {
        expected: "numeric vector".into(),
        actual: e,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer_manager::BufferManagerConfig;
    use crate::page::DEFAULT_PAGE_SIZE;
    use akar_common::memory::MemoryManager;
    use std::sync::Arc;

    fn setup_bm(db_path: &std::path::Path) -> BufferManager {
        let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
        BufferManager::new(db_path.to_path_buf(), mm, BufferManagerConfig::default())
    }

    #[test]
    fn test_vector_index_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut bm = setup_bm(dir.path());

        let mut idx = VectorIndexTable::new(
            1,
            "vec_idx".into(),
            "items".into(),
            "embedding".into(),
            DistanceMetric::Cosine,
            3,
        );
        idx.register_file(&mut bm, dir.path());
        idx.hnsw_mut().insert(vec![1.0, 2.0, 3.0], 0);
        idx.hnsw_mut().insert(vec![4.0, 5.0, 6.0], 1);
        idx.hnsw_mut().insert(vec![7.0, 8.0, 9.0], 2);
        assert_eq!(idx.hnsw.len(), 3);
        idx.save(&mut bm).unwrap();

        // Reload into a fresh instance.
        let mut loaded = VectorIndexTable::new(
            1,
            "vec_idx".into(),
            "items".into(),
            "embedding".into(),
            DistanceMetric::Cosine,
            0,
        );
        loaded.register_file(&mut bm, dir.path());
        loaded.load(&mut bm).unwrap();

        // Exactly the persisted vectors are rebuilt — no phantom empties (P52.17).
        assert_eq!(loaded.hnsw.len(), 3);
        assert_eq!(loaded.dimensions, 3);
        let hits = loaded.hnsw.search(&[1.0, 2.0, 3.0], 3);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].1, 0, "nearest vector to [1,2,3] must be the first inserted");
        // All persisted vectors must be present in the result set.
        let mut ids: Vec<usize> = hits.iter().map(|&(_, id)| id).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![0, 1, 2]);
    }

    #[test]
    fn test_vector_index_roundtrip_preserves_ids() {
        // P51.17: node ids (row offsets) must survive persist/load, even when
        // sparse (rows with a NULL vector are skipped during populate).
        let dir = tempfile::tempdir().unwrap();
        let mut bm = setup_bm(dir.path());

        let mut idx = VectorIndexTable::new(
            2,
            "sparse_idx".into(),
            "items".into(),
            "embedding".into(),
            DistanceMetric::Euclidean,
            2,
        );
        idx.register_file(&mut bm, dir.path());
        idx.hnsw_mut().insert(vec![0.0, 0.0], 0);
        idx.hnsw_mut().insert(vec![1.0, 1.0], 2); // row 1 skipped
        idx.hnsw_mut().insert(vec![2.0, 2.0], 5);
        idx.save(&mut bm).unwrap();

        let mut loaded = VectorIndexTable::new(
            2,
            "sparse_idx".into(),
            "items".into(),
            "embedding".into(),
            DistanceMetric::Euclidean,
            0,
        );
        loaded.register_file(&mut bm, dir.path());
        loaded.load(&mut bm).unwrap();

        assert_eq!(loaded.hnsw.len(), 3);
        assert!(loaded.hnsw().get_vector(5).is_some(), "id 5 must be restored");
        assert!(loaded.hnsw().get_vector(1).is_none());
        let hits = loaded.hnsw().search(&[2.0, 2.0], 3);
        assert_eq!(hits[0].1, 5, "nearest vector must still address row 5");
    }

    #[test]
    fn test_legacy_f64_vector_index_fails_loudly_on_load() {
        // FR-011: an index persisted at 64-bit precision must not be read as if
        // it were f32. Loading it has to fail with rebuild guidance instead of
        // returning silently degraded or empty results.
        let dir = tempfile::tempdir().unwrap();
        let mut bm = setup_bm(dir.path());

        let mut idx = VectorIndexTable::new(
            1,
            "legacy_idx".into(),
            "items".into(),
            "embedding".into(),
            DistanceMetric::Cosine,
            3,
        );
        idx.register_file(&mut bm, dir.path());

        let header = serialize_header(
            0,
            None,
            0,
            3,
            &DistanceMetric::Cosine,
            VectorPrecision::F64,
            0,
            0,
            FORMAT_VERSION_LEGACY,
        );
        {
            let frame = bm.pin_mut(&idx.file_name, 0).unwrap();
            frame.data[..header.len()].copy_from_slice(&header);
            frame.is_dirty = true;
            bm.unpin(&idx.file_name, 0);
        }

        let error = idx
            .load(&mut bm)
            .expect_err("a legacy f64 index must not load silently");
        let message = error.to_string();
        assert!(message.contains("64-bit"), "unexpected error: {message}");
        assert!(
            message.contains("DROP VECTOR INDEX"),
            "rebuild guidance missing: {message}"
        );
    }

    #[test]
    fn test_vector_index_save_errors_when_vector_too_large() {
        let dir = tempfile::tempdir().unwrap();
        let mut bm = setup_bm(dir.path());

        let mut idx = VectorIndexTable::new(
            1,
            "vec_idx".into(),
            "items".into(),
            "embedding".into(),
            DistanceMetric::Cosine,
            1,
        );
        idx.register_file(&mut bm, dir.path());
        // A vector that cannot fit in one page must fail loudly, not silently
        // drop data (P52.17).
        let huge = vec![1.0f32; DEFAULT_PAGE_SIZE];
        idx.hnsw_mut().insert(huge, 0);
        assert!(idx.save(&mut bm).is_err());
    }
}
