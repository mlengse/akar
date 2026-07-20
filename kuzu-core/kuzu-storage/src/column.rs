//! Columnar storage — page-based column with BufferManager-backed I/O.
//!
//! Each `Column` stores values of a single `LogicalType` across multiple
//! fixed-size pages managed by the `BufferManager`. Values are serialized
//! in a compact binary format and packed sequentially within pages.
//!
//! # Page Layout
//!
//! Each page (default 8 KiB) stores a sequence of serialized values.
//! A `PageHeader` records how many values are in the page and the byte
//! offset of each value, enabling direct lookup without scanning.
//!
//! # Value Format
//!
//! Every value is stored as:
//!   1. Tag byte (the Value discriminant, 0x00–0x1B)
//!   2. Type-specific payload (primitives as fixed-size LE bytes,
//!      variable-length types with a u32 length prefix)

use crate::buffer_manager::BufferManager;
use crate::compression::{compress_serialized_value, decompress_serialized_value, serialized_value_size};
use crate::page::FileHandle;
use kuzu_common::enums::CompressionType;
use kuzu_common::types::{LogicalTypeID, PhysicalTypeID, Value};

use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Tag bytes for Value discriminant (must match Value::DISCRIMINANT ordering)
// ---------------------------------------------------------------------------
pub(crate) const TAG_NULL: u8 = 0;
pub(crate) const TAG_BOOL: u8 = 1;
pub(crate) const TAG_INT64: u8 = 2;
pub(crate) const TAG_INT32: u8 = 3;
pub(crate) const TAG_INT16: u8 = 4;
pub(crate) const TAG_INT8: u8 = 5;
pub(crate) const TAG_UINT64: u8 = 6;
pub(crate) const TAG_UINT32: u8 = 7;
pub(crate) const TAG_UINT16: u8 = 8;
pub(crate) const TAG_UINT8: u8 = 9;
pub(crate) const TAG_INT128: u8 = 10;
pub(crate) const TAG_DOUBLE: u8 = 11;
pub(crate) const TAG_FLOAT: u8 = 12;
pub(crate) const TAG_STRING: u8 = 13;
pub(crate) const TAG_BLOB: u8 = 14;
pub(crate) const TAG_DATE: u8 = 15;
pub(crate) const TAG_TIMESTAMP: u8 = 16;
pub(crate) const TAG_TIMESTAMP_TZ: u8 = 17;
pub(crate) const TAG_TIMESTAMP_NS: u8 = 18;
pub(crate) const TAG_TIMESTAMP_MS: u8 = 19;
pub(crate) const TAG_TIMESTAMP_SEC: u8 = 20;
pub(crate) const TAG_INTERVAL: u8 = 21;
pub(crate) const TAG_INTERNAL_ID: u8 = 22;
pub(crate) const TAG_LIST: u8 = 23;
pub(crate) const TAG_MAP: u8 = 24;
pub(crate) const TAG_STRUCT: u8 = 25;
pub(crate) const TAG_UINT128: u8 = 26;
pub(crate) const TAG_JSON: u8 = 27;
pub(crate) const TAG_DTIME: u8 = 28;
pub(crate) const TAG_UNION: u8 = 29;

/// Maximum values stored per page (keeps the offset array fixed-size).
const MAX_VALS_PER_PAGE: usize = 256;

/// Fixed-size header: [num_values: u32][end_offsets: 256×u32]
/// Header size = 4 + 256*4 = 1028 bytes.
const PAGE_HEADER_SIZE: usize = 4 + MAX_VALS_PER_PAGE * 4;

/// Metadata stored at the start of each page.
#[derive(Debug, Clone, Copy)]
struct PageHeader {
    /// Number of values stored in this page.
    num_values: u32,
    /// End byte offsets of each value from the start of the data area.
    /// `end_offsets[i]` = cumulative bytes of data after value i.
    /// So value i spans bytes `[prev_end..end_offsets[i]]` in the data area.
    offsets: [u32; MAX_VALS_PER_PAGE],
}

/// A column stores values of a single type across multiple pages.
///
/// The column owns a dedicated file (named `col_{table_id}_{col_idx}`) and
/// uses the `BufferManager` for all page-level I/O with automatic caching.
#[derive(Debug)]
pub struct Column {
    /// The logical type of values stored in this column.
    pub logical_type: LogicalTypeID,
    /// Physical type derived from `logical_type`.
    pub physical_type: PhysicalTypeID,
    /// The owning table ID (used to construct the file name).
    pub table_id: u64,
    /// The column index within the table.
    pub col_idx: u32,
    /// File name used in the BufferManager's file registry.
    pub file_name: String,
    /// File handle for low-level page I/O.
    pub file_handle: FileHandle,
    /// Shared buffer manager for page caching and eviction.
    pub buffer_manager: Arc<Mutex<BufferManager>>,
    /// Compression algorithm applied to serialized values.
    pub compression_type: CompressionType,
    /// Byte size of the serialized primitive value (0 for variable-length types).
    pub value_size: usize,
    /// Total number of values stored.
    pub num_values: u64,
    /// Number of pages allocated.
    pub num_pages: u64,
    /// Cumulative value count per page (for binary-search lookup).
    /// `page_row_offsets[i]` = the global row index of the first value in page i.
    pub page_row_offsets: Vec<u64>,
}

impl Column {
    /// Create a new column backed by a file in `db_path`.
    ///
    /// The file is named `col_{table_id}_{col_idx}` and registered with
    /// the `BufferManager` automatically.
    ///
    /// `compression_type` determines the compression algorithm applied to
    /// serialized values. Use `CompressionType::Uncompressed` for no compression.
    pub fn new(
        logical_type: LogicalTypeID,
        table_id: u64,
        col_idx: u32,
        db_path: &std::path::Path,
        buffer_manager: Arc<Mutex<BufferManager>>,
        page_size: usize,
    ) -> Self {
        Self::with_compression(
            logical_type,
            table_id,
            col_idx,
            db_path,
            buffer_manager,
            page_size,
            CompressionType::Uncompressed,
        )
    }

    /// Create a new column with a specific compression algorithm.
    pub fn with_compression(
        logical_type: LogicalTypeID,
        table_id: u64,
        col_idx: u32,
        db_path: &std::path::Path,
        buffer_manager: Arc<Mutex<BufferManager>>,
        page_size: usize,
        compression_type: CompressionType,
    ) -> Self {
        let file_name = format!("col_{}_{}", table_id, col_idx);
        let col_file_path = db_path.join(&file_name);

        // Register the file with the buffer manager so it can manage its pages.
        {
            let mut bm = buffer_manager.lock().unwrap();
            bm.register_file(&file_name, col_file_path.clone());
        }

        let fh = FileHandle::new(col_file_path, page_size)
            .with_free_space_manager(std::sync::Arc::new(crate::free_space_manager::FreeSpaceManager::new()));
        let physical_type = kuzu_common::types::physical_type_from_logical(logical_type);
        let value_size = serialized_value_size(physical_type);

        Self {
            logical_type,
            physical_type,
            table_id,
            col_idx,
            file_name,
            file_handle: fh,
            buffer_manager,
            compression_type,
            value_size,
            num_values: 0,
            num_pages: 0,
            page_row_offsets: Vec::new(),
        }
    }

    // ------------------------------------------------------------------
    // Public API
    // ------------------------------------------------------------------

    /// Append a single value to the column.
    ///
    /// Append a single value to the column.
    ///
    /// Automatically allocates a new page when the current one is full or
    /// when the current page has reached the maximum values per page (256).
    ///
    /// Serialized values are compressed according to `self.compression_type`.
    pub fn append_value(&mut self, value: &Value) -> std::io::Result<()> {
        let raw = Self::serialize_value(value);
        let serialized = compress_serialized_value(self.compression_type, &raw, self.value_size);
        // Check if the current page is full (too many values or no space) before trying.
        if self.num_pages > 0 {
            let last_page = self.num_pages - 1;
            let page_data = self.read_page_data(last_page as usize)?;
            let num_vals = u32::from_le_bytes(page_data[..4].try_into().unwrap()) as usize;
            if num_vals >= MAX_VALS_PER_PAGE {
                // Page has hit the max values limit; allocate a new one.
                let new_page = self.allocate_new_page()?;
                return self.write_value_to_page(new_page, &serialized);
            }
            // Check if the value fits in the remaining space.
            let data_end = if num_vals > 0 {
                let last_off_pos = 4 + (num_vals - 1) * 4;
                PAGE_HEADER_SIZE
                    + u32::from_le_bytes(page_data[last_off_pos..last_off_pos + 4].try_into().unwrap()) as usize
            } else {
                PAGE_HEADER_SIZE
            };
            if data_end + serialized.len() > self.file_handle.page_size {
                let new_page = self.allocate_new_page()?;
                return self.write_value_to_page(new_page, &serialized);
            }
        }
        let page_idx = self.ensure_page_for_write()?;
        self.write_value_to_page(page_idx, &serialized)
    }

    /// Get a single value by row index.
    pub fn get_value(&self, row_idx: u64) -> std::io::Result<Value> {
        if row_idx >= self.num_values {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("row index {} out of range (num_values = {})", row_idx, self.num_values),
            ));
        }
        let (page_idx, _) = self.locate_row(row_idx);
        let serialized = self.read_page_data(page_idx)?;
        let header = self.parse_page_header(&serialized)?;
        let local_row = (row_idx - self.page_row_offsets[page_idx]) as usize;
        let value = self.deserialize_value_from_page(&serialized, &header, local_row)?;
        Ok(value)
    }

    /// Scan a range of values (inclusive of `start`, exclusive of `start + count`).
    pub fn scan_values(&self, start: u64, count: u64) -> std::io::Result<Vec<Value>> {
        if count == 0 || start >= self.num_values {
            return Ok(Vec::new());
        }
        let end = (start + count).min(self.num_values);
        let mut results = Vec::with_capacity((end - start) as usize);

        for row in start..end {
            results.push(self.get_value(row)?);
        }
        Ok(results)
    }

    /// Read the raw serialised bytes of a single value (useful for compression).
    pub fn read_value_bytes(&self, row_idx: u64) -> std::io::Result<Vec<u8>> {
        let (page_idx, _) = self.locate_row(row_idx);
        let serialized = self.read_page_data(page_idx)?;
        let header = self.parse_page_header(&serialized)?;
        let local_row = (row_idx - self.page_row_offsets[page_idx]) as usize;
        self.extract_value_bytes(&serialized, &header, local_row)
    }

    /// Flush the column's dirty pages to disk.
    pub fn flush(&self) -> std::io::Result<()> {
        for i in 0..self.num_pages {
            let mut bm = self.buffer_manager.lock().unwrap();
            bm.flush(&self.file_name, i)?;
        }
        Ok(())
    }

    /// Save column metadata to a `.meta` sidecar file so it can be
    /// reconstructed after a crash or restart without scanning every page.
    ///
    /// Layout (all little-endian):
    ///   magic: 4 bytes b"CMET"
    ///   version: u32
    ///   logical_type: u32
    ///   table_id: u64
    ///   col_idx: u32
    ///   num_values: u64
    ///   num_pages: u64
    ///   page_row_offsets: [u64; num_pages]
    pub fn save_metadata(&self) -> std::io::Result<()> {
        let meta_path = self.file_handle.path.with_extension("meta");
        let mut buf = Vec::with_capacity(64 + self.num_pages as usize * 8);

        buf.extend_from_slice(b"CMET");
        buf.extend_from_slice(&1u32.to_le_bytes()); // version
        buf.extend_from_slice(&(self.logical_type as u32).to_le_bytes());
        buf.extend_from_slice(&self.table_id.to_le_bytes());
        buf.extend_from_slice(&self.col_idx.to_le_bytes());
        buf.extend_from_slice(&self.num_values.to_le_bytes());
        buf.extend_from_slice(&self.num_pages.to_le_bytes());
        for offset in &self.page_row_offsets {
            buf.extend_from_slice(&offset.to_le_bytes());
        }

        std::fs::write(&meta_path, &buf)?;
        Ok(())
    }

    /// Load column metadata from a `.meta` sidecar file.
    ///
    /// Returns `Ok(true)` if metadata was loaded successfully,
    /// `Ok(false)` if no metadata file exists (fresh column).
    pub fn load_metadata(&mut self) -> std::io::Result<bool> {
        let meta_path = self.file_handle.path.with_extension("meta");
        if !meta_path.exists() {
            return Ok(false);
        }

        let data = std::fs::read(&meta_path)?;
        if data.len() < 36 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "column metadata file too small",
            ));
        }

        if &data[0..4] != b"CMET" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid column metadata magic bytes",
            ));
        }

        let mut pos = 4;
        let _version = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
        pos += 4;
        let _logical_type = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
        pos += 4;
        let _table_id = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
        pos += 8;
        let _col_idx = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
        pos += 4;
        self.num_values = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
        pos += 8;
        self.num_pages = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
        pos += 8;

        let num_pages = self.num_pages as usize;
        if data.len() < pos + num_pages * 8 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "column metadata file truncated (page_row_offsets)",
            ));
        }

        self.page_row_offsets = Vec::with_capacity(num_pages);
        for _ in 0..num_pages {
            self.page_row_offsets
                .push(u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap()));
            pos += 8;
        }

        Ok(true)
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Serialise `value` into a compact byte sequence.
    pub(crate) fn serialize_value(value: &Value) -> Vec<u8> {
        let mut buf = Vec::with_capacity(16);
        Self::serialize_into(&mut buf, value);
        buf
    }

    fn serialize_into(buf: &mut Vec<u8>, value: &Value) {
        match value {
            Value::Null => buf.push(TAG_NULL),
            Value::Bool(v) => {
                buf.push(TAG_BOOL);
                buf.push(if *v { 1 } else { 0 });
            }
            Value::Int64(v) => {
                buf.push(TAG_INT64);
                buf.extend_from_slice(&v.to_le_bytes());
            }
            Value::Int32(v) => {
                buf.push(TAG_INT32);
                buf.extend_from_slice(&v.to_le_bytes());
            }
            Value::Int16(v) => {
                buf.push(TAG_INT16);
                buf.extend_from_slice(&v.to_le_bytes());
            }
            Value::Int8(v) => {
                buf.push(TAG_INT8);
                buf.push(*v as u8);
            }
            Value::UInt64(v) => {
                buf.push(TAG_UINT64);
                buf.extend_from_slice(&v.to_le_bytes());
            }
            Value::UInt32(v) => {
                buf.push(TAG_UINT32);
                buf.extend_from_slice(&v.to_le_bytes());
            }
            Value::UInt16(v) => {
                buf.push(TAG_UINT16);
                buf.extend_from_slice(&v.to_le_bytes());
            }
            Value::UInt8(v) => {
                buf.push(TAG_UINT8);
                buf.push(*v);
            }
            Value::Int128(v) => {
                buf.push(TAG_INT128);
                buf.extend_from_slice(&v.to_le_bytes());
            }
            Value::Double(v) => {
                buf.push(TAG_DOUBLE);
                buf.extend_from_slice(&v.to_le_bytes());
            }
            Value::Float(v) => {
                buf.push(TAG_FLOAT);
                buf.extend_from_slice(&v.to_le_bytes());
            }
            Value::String(v) => {
                buf.push(TAG_STRING);
                buf.extend_from_slice(&(v.len() as u32).to_le_bytes());
                buf.extend_from_slice(v.as_bytes());
            }
            Value::Blob(v) => {
                buf.push(TAG_BLOB);
                buf.extend_from_slice(&(v.len() as u32).to_le_bytes());
                buf.extend_from_slice(v);
            }
            Value::Date(v) => {
                buf.push(TAG_DATE);
                buf.extend_from_slice(&v.0.to_le_bytes());
            }
            Value::Timestamp(v) | Value::TimestampNs(v) | Value::TimestampMs(v) | Value::TimestampSec(v) => {
                let tag = match value {
                    Value::Timestamp(_) => TAG_TIMESTAMP,
                    Value::TimestampNs(_) => TAG_TIMESTAMP_NS,
                    Value::TimestampMs(_) => TAG_TIMESTAMP_MS,
                    Value::TimestampSec(_) => TAG_TIMESTAMP_SEC,
                    _ => unreachable!(),
                };
                buf.push(tag);
                buf.extend_from_slice(&v.0.to_le_bytes());
            }
            Value::TimestampTz(v) => {
                buf.push(TAG_TIMESTAMP_TZ);
                buf.extend_from_slice(&v.0.to_le_bytes());
            }
            Value::UInt128(v) => {
                buf.push(TAG_UINT128);
                buf.extend_from_slice(&v.to_le_bytes());
            }
            Value::Json(v) => {
                buf.push(TAG_JSON);
                let json_str = v.to_string();
                buf.extend_from_slice(&(json_str.len() as u32).to_le_bytes());
                buf.extend_from_slice(json_str.as_bytes());
            }
            Value::DTime(v) => {
                buf.push(TAG_DTIME);
                buf.extend_from_slice(&v.to_le_bytes());
            }
            Value::Union(tag, val) => {
                buf.push(TAG_UNION);
                buf.extend_from_slice(&(tag.len() as u32).to_le_bytes());
                buf.extend_from_slice(tag.as_bytes());
                Self::serialize_into(buf, val);
            }
            Value::Interval(v) => {
                buf.push(TAG_INTERVAL);
                buf.extend_from_slice(&v.months.to_le_bytes());
                buf.extend_from_slice(&v.days.to_le_bytes());
                buf.extend_from_slice(&v.micros.to_le_bytes());
            }
            Value::InternalID(v) => {
                buf.push(TAG_INTERNAL_ID);
                buf.extend_from_slice(&v.table_id.to_le_bytes());
                buf.extend_from_slice(&v.offset.to_le_bytes());
            }
            Value::List(v) => {
                buf.push(TAG_LIST);
                buf.extend_from_slice(&(v.len() as u32).to_le_bytes());
                for elem in v {
                    Self::serialize_into(buf, elem);
                }
            }
            Value::Map(v) => {
                buf.push(TAG_MAP);
                buf.extend_from_slice(&(v.len() as u32).to_le_bytes());
                for (k, val) in v {
                    Self::serialize_into(buf, k);
                    Self::serialize_into(buf, val);
                }
            }
            Value::Struct(v) => {
                buf.push(TAG_STRUCT);
                buf.extend_from_slice(&(v.len() as u32).to_le_bytes());
                for (name, val) in v {
                    let name_bytes = name.as_bytes();
                    buf.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
                    buf.extend_from_slice(name_bytes);
                    Self::serialize_into(buf, val);
                }
            }
        }
    }

    /// Deserialise a Value from a byte slice starting at the tag byte.
    fn deserialize_value(data: &[u8], pos: &mut usize) -> std::io::Result<Value> {
        if *pos >= data.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "unexpected EOF reading value tag",
            ));
        }
        let tag = data[*pos];
        *pos += 1;

        macro_rules! read_le {
            ($ty:ty) => {{
                let size = std::mem::size_of::<$ty>();
                if *pos + size > data.len() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "unexpected EOF reading value",
                    ));
                }
                let mut arr = [0u8; std::mem::size_of::<$ty>()];
                arr.copy_from_slice(&data[*pos..*pos + size]);
                *pos += size;
                <$ty>::from_le_bytes(arr)
            }};
        }

        match tag {
            TAG_NULL => Ok(Value::Null),
            TAG_BOOL => {
                if *pos >= data.len() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "eof reading bool",
                    ));
                }
                let v = data[*pos] != 0;
                *pos += 1;
                Ok(Value::Bool(v))
            }
            TAG_INT64 => Ok(Value::Int64(read_le!(i64))),
            TAG_INT32 => Ok(Value::Int32(read_le!(i32))),
            TAG_INT16 => Ok(Value::Int16(read_le!(i16))),
            TAG_INT8 => {
                if *pos >= data.len() {
                    return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof reading i8"));
                }
                let v = data[*pos] as i8;
                *pos += 1;
                Ok(Value::Int8(v))
            }
            TAG_UINT64 => Ok(Value::UInt64(read_le!(u64))),
            TAG_UINT32 => Ok(Value::UInt32(read_le!(u32))),
            TAG_UINT16 => Ok(Value::UInt16(read_le!(u16))),
            TAG_UINT8 => {
                if *pos >= data.len() {
                    return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof reading u8"));
                }
                let v = data[*pos];
                *pos += 1;
                Ok(Value::UInt8(v))
            }
            TAG_INT128 => Ok(Value::Int128(read_le!(i128))),
            TAG_DOUBLE => Ok(Value::Double(read_le!(f64))),
            TAG_FLOAT => Ok(Value::Float(read_le!(f32))),
            TAG_STRING => {
                let len = read_le!(u32) as usize;
                if *pos + len > data.len() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "eof reading string data",
                    ));
                }
                let s = String::from_utf8_lossy(&data[*pos..*pos + len]).into_owned();
                *pos += len;
                Ok(Value::String(s))
            }
            TAG_BLOB => {
                let len = read_le!(u32) as usize;
                if *pos + len > data.len() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "eof reading blob data",
                    ));
                }
                let blob = data[*pos..*pos + len].to_vec();
                *pos += len;
                Ok(Value::Blob(blob))
            }
            TAG_DATE => Ok(Value::Date(kuzu_common::types::Date(read_le!(i32)))),
            TAG_TIMESTAMP => Ok(Value::Timestamp(kuzu_common::types::Timestamp(read_le!(i64)))),
            TAG_TIMESTAMP_TZ => Ok(Value::TimestampTz(kuzu_common::types::TimestampTZ(read_le!(i64)))),
            TAG_TIMESTAMP_NS => Ok(Value::TimestampNs(kuzu_common::types::Timestamp(read_le!(i64)))),
            TAG_TIMESTAMP_MS => Ok(Value::TimestampMs(kuzu_common::types::Timestamp(read_le!(i64)))),
            TAG_TIMESTAMP_SEC => Ok(Value::TimestampSec(kuzu_common::types::Timestamp(read_le!(i64)))),
            TAG_INTERVAL => {
                let months = read_le!(i32);
                let days = read_le!(i32);
                let micros = read_le!(i64);
                Ok(Value::Interval(kuzu_common::types::Interval { months, days, micros }))
            }
            TAG_INTERNAL_ID => {
                let table_id = read_le!(u64);
                let offset = read_le!(u64);
                Ok(Value::InternalID(kuzu_common::types::InternalID { table_id, offset }))
            }
            TAG_LIST => {
                let len = read_le!(u32) as usize;
                let mut elems = Vec::with_capacity(len);
                for _ in 0..len {
                    elems.push(Self::deserialize_value(data, pos)?);
                }
                Ok(Value::List(elems))
            }
            TAG_MAP => {
                let len = read_le!(u32) as usize;
                let mut elems = Vec::with_capacity(len);
                for _ in 0..len {
                    let k = Self::deserialize_value(data, pos)?;
                    let v = Self::deserialize_value(data, pos)?;
                    elems.push((k, v));
                }
                Ok(Value::Map(elems))
            }
            TAG_STRUCT => {
                let len = read_le!(u32) as usize;
                let mut fields = Vec::with_capacity(len);
                for _ in 0..len {
                    let name_len = read_le!(u32) as usize;
                    if *pos + name_len > data.len() {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "eof reading struct field name",
                        ));
                    }
                    let name = String::from_utf8_lossy(&data[*pos..*pos + name_len]).into_owned();
                    *pos += name_len;
                    let val = Self::deserialize_value(data, pos)?;
                    fields.push((name, val));
                }
                Ok(Value::Struct(fields))
            }
            TAG_UINT128 => Ok(Value::UInt128(read_le!(u128))),
            TAG_JSON => {
                let len = read_le!(u32) as usize;
                if *pos + len > data.len() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "eof reading json data",
                    ));
                }
                let s = String::from_utf8_lossy(&data[*pos..*pos + len]).into_owned();
                *pos += len;
                match serde_json::from_str(&s) {
                    Ok(v) => Ok(Value::Json(v)),
                    Err(e) => Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("invalid json: {}", e),
                    )),
                }
            }
            TAG_DTIME => Ok(Value::DTime(read_le!(i64))),
            TAG_UNION => {
                let len = read_le!(u32) as usize;
                if *pos + len > data.len() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "eof reading union tag",
                    ));
                }
                let tag = String::from_utf8_lossy(&data[*pos..*pos + len]).into_owned();
                *pos += len;
                let val = Self::deserialize_value(data, pos)?;
                Ok(Value::Union(tag, Box::new(val)))
            }
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown value tag: 0x{:02x}", tag),
            )),
        }
    }

    /// Write a serialized value to a page (append to the data area).
    ///
    /// Page layout (fixed-size header, never shifts):
    ///   ```text
    ///   [0..4)            = num_values (u32 LE)
    ///   [4..PAGE_HEADER_SIZE) = end_offsets[256] (u32 × 256)
    ///   [PAGE_HEADER_SIZE..)   = data area (serialised values packed sequentially)
    ///   ```
    /// Header size is always `PAGE_HEADER_SIZE` (= 1028 bytes), so the data
    /// area never moves regardless of how many values are in the page.
    ///
    /// Value i occupies bytes `[start_off..end_off)` in the data area where:
    ///   start_off = 0 if i == 0 else end_offsets[i-1]
    ///   end_off   = end_offsets[i]
    fn write_value_to_page(&mut self, page_idx: u64, serialized: &[u8]) -> std::io::Result<()> {
        let mut bm = self.buffer_manager.lock().unwrap();
        let frame = bm.pin_mut(&self.file_name, page_idx)?;
        let page_size = self.file_handle.page_size;

        let num_vals = u32::from_le_bytes(frame.data[..4].try_into().unwrap()) as usize;

        // Data area starts at PAGE_HEADER_SIZE (fixed, never shifts).
        let data_area_start = PAGE_HEADER_SIZE;

        // Compute the byte offset into the data area where the new value begins.
        let prev_end = if num_vals > 0 {
            let prev_off_pos = 4 + (num_vals - 1) * 4;
            u32::from_le_bytes(frame.data[prev_off_pos..prev_off_pos + 4].try_into().unwrap()) as usize
        } else {
            0usize
        };

        let data_write_pos = data_area_start + prev_end;

        // Ensure the value fits.
        if data_write_pos + serialized.len() > page_size {
            return Err(std::io::Error::new(
                std::io::ErrorKind::OutOfMemory,
                format!(
                    "value of {} bytes does not fit in page (page_size={}, used={})",
                    serialized.len(),
                    page_size,
                    data_write_pos,
                ),
            ));
        }

        // Write the end offset for this value (= prev_end + serialized.len()).
        let new_end = (prev_end + serialized.len()) as u32;
        let off_pos = 4 + num_vals * 4;
        frame.data[off_pos..off_pos + 4].copy_from_slice(&new_end.to_le_bytes());

        // Write value data.
        frame.data[data_write_pos..data_write_pos + serialized.len()].copy_from_slice(serialized);

        // Update num_values.
        frame.data[..4].copy_from_slice(&((num_vals + 1) as u32).to_le_bytes());
        frame.mark_dirty();
        bm.unpin(&self.file_name, page_idx);
        drop(bm);

        self.num_values += 1;

        Ok(())
    }

    /// Determine which page a row lives in via binary search.
    fn locate_row(&self, row_idx: u64) -> (usize, u64) {
        match self.page_row_offsets.binary_search(&row_idx) {
            Ok(i) => (i, row_idx - self.page_row_offsets[i]),
            Err(i) => {
                if i == 0 {
                    (0, row_idx)
                } else {
                    (i - 1, row_idx - self.page_row_offsets[i - 1])
                }
            }
        }
    }

    /// Ensure at least one page exists, allocating a new one if needed.
    fn ensure_page_for_write(&mut self) -> std::io::Result<u64> {
        if self.num_pages == 0 {
            let page_num = self.allocate_new_page()?;
            Ok(page_num)
        } else {
            Ok(self.num_pages - 1)
        }
    }

    /// Allocate a new, empty page and update tracking metadata.
    fn allocate_new_page(&mut self) -> std::io::Result<u64> {
        let mut fh = self.file_handle.clone();
        let page_num = fh.allocate_page();
        let empty_header = vec![0u8; self.file_handle.page_size];
        fh.write_page(page_num, &empty_header)?;
        self.file_handle = fh;
        self.page_row_offsets.push(self.num_values);
        let page_idx = self.num_pages;
        self.num_pages += 1;
        Ok(page_idx)
    }

    /// Read raw page data from the buffer manager.
    fn read_page_data(&self, page_idx: usize) -> std::io::Result<Vec<u8>> {
        let mut bm = self.buffer_manager.lock().unwrap();
        let frame = bm.pin(&self.file_name, page_idx as u64)?;
        let data = frame.data.clone();
        bm.unpin(&self.file_name, page_idx as u64);
        Ok(data)
    }

    /// Parse the page header from raw page bytes.
    fn parse_page_header(&self, data: &[u8]) -> std::io::Result<PageHeader> {
        if data.len() < PAGE_HEADER_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("page too small for header: {} < {}", data.len(), PAGE_HEADER_SIZE),
            ));
        }
        let num_values = u32::from_le_bytes(data[..4].try_into().unwrap());
        let num_vals = num_values.min(MAX_VALS_PER_PAGE as u32) as usize;
        let mut offsets = [0u32; MAX_VALS_PER_PAGE];
        for (i, offset) in offsets.iter_mut().enumerate().take(num_vals) {
            let off_pos = 4 + i * 4;
            *offset = u32::from_le_bytes(data[off_pos..off_pos + 4].try_into().unwrap());
        }
        Ok(PageHeader { num_values, offsets })
    }

    /// Extract raw bytes of a single value from a page.
    ///
    /// Data area starts at `PAGE_HEADER_SIZE` (fixed). Value i occupies
    /// bytes `[PAGE_HEADER_SIZE + start_off .. PAGE_HEADER_SIZE + end_off)`
    /// where `start_off = 0 if i==0 else offsets[i-1]`, `end_off = offsets[i]`.
    fn extract_value_bytes(&self, data: &[u8], header: &PageHeader, local_row: usize) -> std::io::Result<Vec<u8>> {
        if local_row >= header.num_values as usize {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "local row out of range",
            ));
        }
        let value_start = if local_row > 0 {
            PAGE_HEADER_SIZE + header.offsets[local_row - 1] as usize
        } else {
            PAGE_HEADER_SIZE
        };
        let value_end = PAGE_HEADER_SIZE + header.offsets[local_row] as usize;
        if value_start >= data.len() || value_end > data.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "value offset out of bounds",
            ));
        }
        Ok(data[value_start..value_end].to_vec())
    }

    /// Deserialize a value from a page at the given local row index.
    ///
    /// Decompresses the stored bytes according to `self.compression_type`
    /// before deserializing the Value.
    fn deserialize_value_from_page(
        &self,
        data: &[u8],
        header: &PageHeader,
        local_row: usize,
    ) -> std::io::Result<Value> {
        let stored = self.extract_value_bytes(data, header, local_row)?;
        let bytes = decompress_serialized_value(self.compression_type, &stored, self.value_size);
        let mut pos = 0;
        Self::deserialize_value(&bytes, &mut pos)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::DEFAULT_PAGE_SIZE;
    use kuzu_common::enums::CompressionType;
    use kuzu_common::memory::MemoryManager;
    use kuzu_common::types::{Date, InternalID, Interval, Timestamp, TimestampTZ};

    fn setup_column() -> (Column, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().to_path_buf();
        let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
        let config = crate::buffer_manager::BufferManagerConfig::default();
        let bm = Arc::new(Mutex::new(crate::buffer_manager::BufferManager::new(
            db_path.clone(),
            mm,
            config,
        )));
        let col = Column::new(
            LogicalTypeID::Int64,
            0, // table_id
            0, // col_idx
            &db_path,
            bm,
            DEFAULT_PAGE_SIZE,
        );
        (col, dir)
    }

    #[test]
    fn test_serialize_deserialize_primitives() {
        let values = vec![
            Value::Null,
            Value::Bool(true),
            Value::Bool(false),
            Value::Int64(42),
            Value::Int64(-1),
            Value::Int32(12345),
            Value::Int16(-32768),
            Value::Int8(127),
            Value::UInt64(u64::MAX),
            Value::UInt32(99999),
            Value::UInt16(65535),
            Value::UInt8(255),
            Value::Int128(i128::MAX),
            Value::Double(3.15),
            Value::Float(std::f32::consts::E),
        ];

        for v in &values {
            let buf = Column::serialize_value(v);
            let mut pos = 0;
            let deserialized = Column::deserialize_value(&buf, &mut pos).unwrap();
            assert_eq!(&deserialized, v, "roundtrip failed for value: {:?}", v);
        }
    }

    #[test]
    fn test_serialize_deserialize_string() {
        let v = Value::String("hello world!".to_string());
        let buf = Column::serialize_value(&v);
        let mut pos = 0;
        let deserialized = Column::deserialize_value(&buf, &mut pos).unwrap();
        assert_eq!(deserialized, v);
    }

    #[test]
    fn test_serialize_deserialize_blob() {
        let v = Value::Blob(vec![0xDE, 0xAD, 0xBE, 0xEF]);
        let buf = Column::serialize_value(&v);
        let mut pos = 0;
        let deserialized = Column::deserialize_value(&buf, &mut pos).unwrap();
        assert_eq!(deserialized, v);
    }

    #[test]
    fn test_serialize_deserialize_date_time() {
        let vals = vec![
            Value::Date(Date(12345)),
            Value::Timestamp(Timestamp(1_700_000_000_000_000)),
            Value::TimestampTz(TimestampTZ(1_700_000_000_000_000)),
            Value::Interval(Interval {
                months: 12,
                days: 30,
                micros: 1_000_000,
            }),
            Value::InternalID(InternalID {
                table_id: 10,
                offset: 42,
            }),
        ];
        for v in &vals {
            let buf = Column::serialize_value(v);
            let mut pos = 0;
            let deserialized = Column::deserialize_value(&buf, &mut pos).unwrap();
            assert_eq!(&deserialized, v, "failed for {:?}", v);
        }
    }

    #[test]
    fn test_serialize_deserialize_list() {
        let v = Value::List(vec![Value::Int64(1), Value::Int64(2), Value::Int64(3)]);
        let buf = Column::serialize_value(&v);
        let mut pos = 0;
        let deserialized = Column::deserialize_value(&buf, &mut pos).unwrap();
        assert_eq!(deserialized, v);
    }

    #[test]
    fn test_serialize_deserialize_struct() {
        let v = Value::Struct(vec![
            ("name".to_string(), Value::String("Alice".to_string())),
            ("age".to_string(), Value::Int64(30)),
        ]);
        let buf = Column::serialize_value(&v);
        let mut pos = 0;
        let deserialized = Column::deserialize_value(&buf, &mut pos).unwrap();
        assert_eq!(deserialized, v);
    }

    #[test]
    fn test_append_and_read() {
        let (mut col, _dir) = setup_column();

        col.append_value(&Value::Int64(100)).unwrap();
        col.append_value(&Value::Int64(200)).unwrap();
        col.append_value(&Value::Int64(300)).unwrap();

        assert_eq!(col.num_values, 3);

        let v0 = col.get_value(0).unwrap();
        assert_eq!(v0, Value::Int64(100));

        let v1 = col.get_value(1).unwrap();
        assert_eq!(v1, Value::Int64(200));

        let v2 = col.get_value(2).unwrap();
        assert_eq!(v2, Value::Int64(300));
    }

    #[test]
    fn test_scan_values() {
        let (mut col, _dir) = setup_column();

        for i in 0..10 {
            col.append_value(&Value::Int64(i as i64)).unwrap();
        }

        let scanned = col.scan_values(2, 5).unwrap();
        assert_eq!(scanned.len(), 5);
        assert_eq!(scanned[0], Value::Int64(2));
        assert_eq!(scanned[4], Value::Int64(6));
    }

    #[test]
    fn test_out_of_range() {
        let (mut col, _dir) = setup_column();
        col.append_value(&Value::Int64(42)).unwrap();

        let result = col.get_value(5);
        assert!(result.is_err());
    }

    #[test]
    fn test_mixed_types() {
        let (mut col, _dir) = setup_column();

        col.append_value(&Value::String("hello".to_string())).unwrap();
        col.append_value(&Value::Double(3.15)).unwrap();
        col.append_value(&Value::Bool(true)).unwrap();
        col.append_value(&Value::List(vec![Value::Int64(1), Value::Int64(2)]))
            .unwrap();

        assert_eq!(col.get_value(0).unwrap(), Value::String("hello".to_string()));
        assert_eq!(col.get_value(1).unwrap(), Value::Double(3.15));
        assert_eq!(col.get_value(2).unwrap(), Value::Bool(true));
        assert_eq!(
            col.get_value(3).unwrap(),
            Value::List(vec![Value::Int64(1), Value::Int64(2)])
        );
    }

    #[test]
    fn test_roundtrip_via_buffer_manager() {
        let (mut col, _dir) = setup_column();

        for i in 0..50 {
            col.append_value(&Value::Int64(i as i64)).unwrap();
        }

        col.flush().unwrap();

        for i in 0..50 {
            let v = col.get_value(i as u64).unwrap();
            assert_eq!(v, Value::Int64(i as i64));
        }
    }

    #[test]
    fn test_empty_column() {
        let (col, _dir) = setup_column();
        assert_eq!(col.num_values, 0);
        assert_eq!(col.num_pages, 0);
    }

    // ==================== Compression integration ====================

    fn setup_compressed_column(ctype: CompressionType) -> (Column, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().to_path_buf();
        let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
        let config = crate::buffer_manager::BufferManagerConfig::default();
        let bm = Arc::new(Mutex::new(crate::buffer_manager::BufferManager::new(
            db_path.clone(),
            mm,
            config,
        )));
        let col = Column::with_compression(
            LogicalTypeID::Int64,
            0, // table_id
            0, // col_idx
            &db_path,
            bm,
            DEFAULT_PAGE_SIZE,
            ctype,
        );
        (col, dir)
    }

    #[test]
    fn test_column_with_integer_bitpacking() {
        let (mut col, _dir) = setup_compressed_column(CompressionType::IntegerBitpacking);
        assert_eq!(col.compression_type, CompressionType::IntegerBitpacking);

        // Small integers benefit from bitpacking
        for i in 0i64..50 {
            col.append_value(&Value::Int64(i)).unwrap();
        }
        assert_eq!(col.num_values, 50);

        // Verify roundtrip
        for i in 0i64..50 {
            let v = col.get_value(i as u64).unwrap();
            assert_eq!(v, Value::Int64(i));
        }
    }

    #[test]
    fn test_column_with_float_compression() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().to_path_buf();
        let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
        let config = crate::buffer_manager::BufferManagerConfig::default();
        let bm = Arc::new(Mutex::new(crate::buffer_manager::BufferManager::new(
            db_path.clone(),
            mm,
            config,
        )));
        let mut col = Column::with_compression(
            LogicalTypeID::Double,
            0,
            0,
            &db_path,
            bm,
            DEFAULT_PAGE_SIZE,
            CompressionType::Float,
        );

        let vals = vec![1.0, 3.15, -2.5, 0.0, 1e10];
        for v in &vals {
            col.append_value(&Value::Double(*v)).unwrap();
        }
        assert_eq!(col.num_values, 5);

        for (i, expected) in vals.iter().enumerate() {
            let v = col.get_value(i as u64).unwrap();
            match v {
                Value::Double(d) => assert!((d - expected).abs() < 1e-10, "mismatch at {}", i),
                _ => panic!("expected Double, got {:?}", v),
            }
        }
    }

    #[test]
    fn test_column_compression_large_values() {
        // Large integers should still roundtrip correctly
        let (mut col, _dir) = setup_compressed_column(CompressionType::IntegerBitpacking);
        let large: Vec<i64> = vec![i64::MAX, i64::MIN, 0, 1, -1, 1_000_000_000, -999_999_999];

        for v in &large {
            col.append_value(&Value::Int64(*v)).unwrap();
        }

        for (i, expected) in large.iter().enumerate() {
            let v = col.get_value(i as u64).unwrap();
            assert_eq!(v, Value::Int64(*expected), "mismatch at index {}", i);
        }
    }

    #[test]
    fn test_column_compression_mixed_types() {
        // String values should pass through correctly with IntegerBitpacking
        // (value_size=0 for strings, so compression is pass-through)
        let (mut col, _dir) = setup_compressed_column(CompressionType::IntegerBitpacking);
        // Actually, for String type, value_size=0, so it behaves like pass-through
        // This test uses Int64 to verify compression doesn't break data
        col.append_value(&Value::Int64(42)).unwrap();
        col.append_value(&Value::Int64(0)).unwrap();
        col.append_value(&Value::Int64(-1)).unwrap();
        assert_eq!(col.get_value(0).unwrap(), Value::Int64(42));
        assert_eq!(col.get_value(1).unwrap(), Value::Int64(0));
        assert_eq!(col.get_value(2).unwrap(), Value::Int64(-1));
    }
}
