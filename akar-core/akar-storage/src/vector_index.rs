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
/// - bytes 33..47: reserved
const HEADER_SIZE: usize = 48;

fn serialize_header(
    num_vectors: u64,
    entry_point: Option<usize>,
    max_level: usize,
    dimensions: u32,
    metric: &DistanceMetric,
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
    buf
}

fn deserialize_header(buf: &[u8]) -> Option<(u64, Option<usize>, usize, u32, DistanceMetric)> {
    if buf.len() < HEADER_SIZE {
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
    Some((num_vectors, entry_point, max_level, dimensions, metric))
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
    /// Writes the header page (page 0) and all data pages.
    pub fn save(&mut self, bm: &mut BufferManager) -> Result<(), StorageError> {
        if !bm.is_file_registered(&self.file_name) {
            return Err(StorageError::Index(format!(
                "Vector index file '{}' not registered with BufferManager",
                self.file_name
            )));
        }

        let num_vectors = self.hnsw.len() as u64;
        let entry_point = self.hnsw.entry_point();
        let max_level = self.hnsw.max_level();

        // Serialize header
        let header = serialize_header(
            num_vectors,
            entry_point,
            max_level,
            self.dimensions,
            &self.hnsw.metric(),
        );

        // Write header page
        let frame = bm
            .pin_mut(&self.file_name, 0)
            .map_err(|e| StorageError::Index(format!("Failed to pin header page: {e}")))?;
        let data = &mut frame.data;
        let write_len = header.len().min(data.len());
        data[..write_len].copy_from_slice(&header[..write_len]);
        frame.is_dirty = true;
        bm.unpin(&self.file_name, 0);

        // Serialize and write vector data pages
        let vectors = self.hnsw.vectors();
        let data_page_start = 1;
        let mut page_idx = data_page_start;
        let mut offset = 0u64;

        while offset < num_vectors {
            let frame = bm
                .pin_mut(&self.file_name, page_idx)
                .map_err(|e| StorageError::Index(format!("Failed to pin data page {page_idx}: {e}")))?;
            let page_data = &mut frame.data;
            let capacity = page_data.len();
            page_data.fill(0u8);
            let mut written = 0usize;
            let mut pos = 0usize;

            while offset < num_vectors && pos + 8 < capacity {
                // Write vector ID
                page_data[pos..pos + 8].copy_from_slice(&offset.to_le_bytes());
                pos += 8;

                // Write vector data
                if let Some(vec_data) = vectors.get(offset as usize) {
                    let vec_bytes: Vec<u8> = vec_data.iter().flat_map(|f| f.to_le_bytes()).collect();
                    let vec_len = vec_bytes.len() as u32;
                    if pos + 4 + vec_len as usize > capacity {
                        break; // Not enough room for this vector — continue on next page
                    }
                    page_data[pos..pos + 4].copy_from_slice(&vec_len.to_le_bytes());
                    pos += 4;
                    page_data[pos..pos + vec_len as usize].copy_from_slice(&vec_bytes);
                    pos += vec_len as usize;
                }

                offset += 1;
                written += 1;
            }

            frame.is_dirty = true;
            bm.unpin(&self.file_name, page_idx);

            if written == 0 {
                break;
            }
            page_idx += 1;
        }

        self.page_count = page_idx;
        self.dirty = false;
        Ok(())
    }

    /// Load the HNSW index from BufferManager-backed pages.
    ///
    /// Reads the header page and all data pages, rebuilding the in-memory index.
    /// After loading, the index is fully searchable.
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
        let (_num_vectors, _entry_point, _max_level, dimensions, metric) =
            deserialize_header(header_data).ok_or(StorageError::Index("Invalid vector index header".into()))?;
        self.dimensions = dimensions;
        bm.unpin(&self.file_name, 0);

        // Rebuild the HNSW index
        let mut new_hnsw = HnswIndex::new(metric);

        // Read data pages from page 1 onward
        let data_page_start = 1u64;
        let mut page_idx = data_page_start;
        let mut _vector_id = 0usize;

        loop {
            if !bm.is_file_registered(&self.file_name) {
                break;
            }
            let frame_result = bm.pin(&self.file_name, page_idx);
            if frame_result.is_err() {
                break; // No more pages
            }
            let frame = frame_result.unwrap();
            let page_data = &frame.data;
            let mut pos = 0usize;
            let capacity = page_data.len();

            while pos + 8 <= capacity {
                let id = u64::from_le_bytes(page_data[pos..pos + 8].try_into().unwrap()) as usize;
                pos += 8;

                if pos + 4 > capacity {
                    break;
                }
                let vec_len = u32::from_le_bytes(page_data[pos..pos + 4].try_into().unwrap()) as usize;
                pos += 4;

                if pos + vec_len > capacity {
                    break;
                }

                let dims = vec_len / 8;
                let mut vec_data = Vec::with_capacity(dims);
                for i in 0..dims {
                    let f = f64::from_le_bytes(page_data[pos + i * 8..pos + (i + 1) * 8].try_into().unwrap());
                    vec_data.push(f);
                }
                pos += vec_len;

                new_hnsw.insert(vec_data, id);
                _vector_id += 1;
            }

            bm.unpin(&self.file_name, page_idx);
            page_idx += 1;

            // Safety: prevent infinite loop if pages keep reading
            if page_idx > 1024 * 1024 {
                break;
            }
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
        bm.flush_all().map_err(|e| StorageError::Index(format!("Failed to flush vector index: {e}")))
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
    match val {
        Value::List(items) => {
            let mut result = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Value::Double(d) => result.push(*d),
                    Value::Int64(i) => result.push(*i as f64),
                    Value::Int32(i) => result.push(*i as f64),
                    Value::Float(f) => result.push(*f as f64),
                    other => {
                        return Err(StorageError::TypeMismatch {
                            expected: "numeric value".into(),
                            actual: format!("{:?}", other),
                        });
                    }
                }
            }
            Ok(result)
        }
        other => Err(StorageError::TypeMismatch {
            expected: "List value for vector".into(),
            actual: format!("{:?}", other),
        }),
    }
}
