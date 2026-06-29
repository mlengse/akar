//! ValueVector — typed columnar data array used throughout the query engine.

use crate::types::{PhysicalTypeID, Value};

/// A vector of values of the same physical type.
/// This is the fundamental columnar data unit in Kuzu's query execution.
#[derive(Debug, Clone)]
pub struct ValueVector {
    physical_type: PhysicalTypeID,
    /// The actual data buffer (type-erased byte buffer).
    data: Vec<u8>,
    /// Nullability mask (true = not null).
    null_mask: Vec<bool>,
    /// Number of elements currently in the vector.
    size: usize,
    /// Capacity of the vector (number of elements, not bytes).
    capacity: usize,
}

impl ValueVector {
    pub fn new(physical_type: PhysicalTypeID, capacity: usize) -> Self {
        let type_size = physical_type_size(physical_type);
        Self {
            physical_type,
            data: vec![0u8; capacity * type_size],
            null_mask: vec![true; capacity],
            size: 0,
            capacity,
        }
    }

    pub fn physical_type(&self) -> PhysicalTypeID {
        self.physical_type
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn is_null(&self, idx: usize) -> bool {
        !self.null_mask[idx]
    }

    pub fn set_null(&mut self, idx: usize, is_null: bool) {
        self.null_mask[idx] = !is_null;
    }

    pub fn resize(&mut self, new_size: usize) {
        assert!(new_size <= self.capacity);
        self.size = new_size;
    }

    /// Get a reference to the raw data buffer.
    pub fn data(&self) -> &[u8] {
        &self.data[..self.size * physical_type_size(self.physical_type)]
    }

    pub fn data_mut(&mut self) -> &mut [u8] {
        let type_size = physical_type_size(self.physical_type);
        &mut self.data[..self.size * type_size]
    }
}

/// Returns the byte size of a given physical type.
pub const fn physical_type_size(t: PhysicalTypeID) -> usize {
    match t {
        PhysicalTypeID::Bool => 1,
        PhysicalTypeID::Int8 | PhysicalTypeID::UInt8 => 1,
        PhysicalTypeID::Int16 | PhysicalTypeID::UInt16 => 2,
        PhysicalTypeID::Int32 | PhysicalTypeID::UInt32 | PhysicalTypeID::Float => 4,
        PhysicalTypeID::Int64 | PhysicalTypeID::UInt64 | PhysicalTypeID::Double | PhysicalTypeID::Interval => 8,
        PhysicalTypeID::Int128 => 16,
        PhysicalTypeID::String => 16,                       // string view
        PhysicalTypeID::Struct => 8,                        // pointer to struct data
        PhysicalTypeID::List | PhysicalTypeID::Array => 16, // list header
        PhysicalTypeID::Blob => 16,
        PhysicalTypeID::Any => 1,
    }
}

// --- Typed getters/setters ---

impl ValueVector {
    /// Get an i64 value at index.
    pub fn get_i64(&self, idx: usize) -> Option<i64> {
        if self.is_null(idx) {
            return None;
        }
        let type_size = physical_type_size(self.physical_type);
        let offset = idx * type_size;
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&self.data[offset..offset + 8]);
        Some(i64::from_le_bytes(buf))
    }

    /// Set an i64 value at index.
    pub fn set_i64(&mut self, idx: usize, val: i64) {
        let type_size = physical_type_size(self.physical_type);
        let offset = idx * type_size;
        self.data[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
        self.null_mask[idx] = true;
        if idx >= self.size {
            self.size = idx + 1;
        }
    }

    /// Get an i32 value at index.
    pub fn get_i32(&self, idx: usize) -> Option<i32> {
        if self.is_null(idx) {
            return None;
        }
        let type_size = physical_type_size(self.physical_type);
        let offset = idx * type_size;
        let mut buf = [0u8; 4];
        buf.copy_from_slice(&self.data[offset..offset + 4]);
        Some(i32::from_le_bytes(buf))
    }

    /// Set an i32 value at index.
    pub fn set_i32(&mut self, idx: usize, val: i32) {
        let type_size = physical_type_size(self.physical_type);
        let offset = idx * type_size;
        self.data[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
        self.null_mask[idx] = true;
        if idx >= self.size {
            self.size = idx + 1;
        }
    }

    /// Get an f64 (double) value at index.
    pub fn get_double(&self, idx: usize) -> Option<f64> {
        if self.is_null(idx) {
            return None;
        }
        let type_size = physical_type_size(self.physical_type);
        let offset = idx * type_size;
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&self.data[offset..offset + 8]);
        Some(f64::from_le_bytes(buf))
    }

    /// Set an f64 (double) value at index.
    pub fn set_double(&mut self, idx: usize, val: f64) {
        let type_size = physical_type_size(self.physical_type);
        let offset = idx * type_size;
        self.data[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
        self.null_mask[idx] = true;
        if idx >= self.size {
            self.size = idx + 1;
        }
    }
}

impl ValueVector {
    /// Get a Value enum from this vector at a given row index.
    /// This converts the raw byte buffer into the appropriate Value variant.
    pub fn get_value(&self, idx: usize) -> Option<Value> {
        if idx >= self.size || self.is_null(idx) {
            return None;
        }
        let type_size = physical_type_size(self.physical_type);
        let offset = idx * type_size;
        match self.physical_type {
            PhysicalTypeID::Bool => Some(Value::Bool(self.data[offset] != 0)),
            PhysicalTypeID::Int64 => {
                let mut buf = [0u8; 8];
                buf.copy_from_slice(&self.data[offset..offset + 8]);
                Some(Value::Int64(i64::from_le_bytes(buf)))
            }
            PhysicalTypeID::Int32 => {
                let mut buf = [0u8; 4];
                buf.copy_from_slice(&self.data[offset..offset + 4]);
                Some(Value::Int32(i32::from_le_bytes(buf)))
            }
            PhysicalTypeID::Int16 => {
                let mut buf = [0u8; 2];
                buf.copy_from_slice(&self.data[offset..offset + 2]);
                Some(Value::Int16(i16::from_le_bytes(buf)))
            }
            PhysicalTypeID::Int8 => Some(Value::Int8(self.data[offset] as i8)),
            PhysicalTypeID::UInt64 => {
                let mut buf = [0u8; 8];
                buf.copy_from_slice(&self.data[offset..offset + 8]);
                Some(Value::UInt64(u64::from_le_bytes(buf)))
            }
            PhysicalTypeID::UInt32 => {
                let mut buf = [0u8; 4];
                buf.copy_from_slice(&self.data[offset..offset + 4]);
                Some(Value::UInt32(u32::from_le_bytes(buf)))
            }
            PhysicalTypeID::UInt16 => {
                let mut buf = [0u8; 2];
                buf.copy_from_slice(&self.data[offset..offset + 2]);
                Some(Value::UInt16(u16::from_le_bytes(buf)))
            }
            PhysicalTypeID::UInt8 => Some(Value::UInt8(self.data[offset])),
            PhysicalTypeID::Double => {
                let mut buf = [0u8; 8];
                buf.copy_from_slice(&self.data[offset..offset + 8]);
                Some(Value::Double(f64::from_le_bytes(buf)))
            }
            PhysicalTypeID::Float => {
                let mut buf = [0u8; 4];
                buf.copy_from_slice(&self.data[offset..offset + 4]);
                Some(Value::Float(f32::from_le_bytes(buf)))
            }
            PhysicalTypeID::String => {
                let len = self.data[offset] as usize;
                let s = String::from_utf8_lossy(&self.data[offset + 1..offset + 1 + len.min(15)]).to_string();
                Some(Value::String(s))
            }
            // For struct/list types, return a simplified representation
            PhysicalTypeID::Struct => Some(Value::Struct(Vec::new())),
            PhysicalTypeID::List => Some(Value::List(Vec::new())),
            _ => None,
        }
    }

    /// Push a boolean value to the end of the vector.
    pub fn push_bool(&mut self, val: bool) {
        let idx = self.size;
        let byte: u8 = if val { 1 } else { 0 };
        self.data[idx] = byte;
        self.null_mask[idx] = true;
        self.size += 1;
    }

    /// Get a boolean value at index.
    pub fn get_bool(&self, idx: usize) -> Option<bool> {
        if self.is_null(idx) {
            return None;
        }
        Some(self.data[idx] != 0)
    }

    /// Push a string value (stores as inline bytes for now).
    pub fn push_string(&mut self, val: &str) {
        let idx = self.size;
        let bytes = val.as_bytes();
        // Store string length + data (simplified: just store bytes, max ~16 bytes)
        let len = bytes.len().min(15) as u8;
        self.data[idx * 16] = len;
        let copy_len = bytes.len().min(15);
        self.data[idx * 16 + 1..idx * 16 + 1 + copy_len].copy_from_slice(&bytes[..copy_len]);
        self.null_mask[idx] = true;
        self.size += 1;
    }

    /// Append a value from another vector (for DataChunk operations).
    pub fn append(&mut self, other: &ValueVector) {
        let start = self.size;
        let count = other.size;
        let type_size = physical_type_size(self.physical_type);
        let bytes_to_copy = count * type_size;
        if start * type_size + bytes_to_copy > self.data.len() {
            self.data.resize((start + count) * type_size, 0);
            self.null_mask.resize(start + count, true);
            self.capacity = start + count;
        }
        self.data[start * type_size..start * type_size + bytes_to_copy].copy_from_slice(&other.data[..bytes_to_copy]);
        for i in 0..count {
            self.null_mask[start + i] = other.null_mask[i];
        }
        self.size = start + count;
    }
}

/// A collection of ValueVectors that represent a subset of a table's columns.
#[derive(Debug, Clone)]
pub struct DataChunk {
    pub fields: Vec<ValueVector>,
    pub size: usize,
}

/// Resize a DataChunk to the given number of rows.
pub fn resize_chunk(chunk: &mut DataChunk, new_size: usize) {
    chunk.size = new_size;
    for field in &mut chunk.fields {
        field.resize(new_size);
    }
}

impl DataChunk {
    pub fn new(fields: Vec<ValueVector>) -> Self {
        let size = fields.first().map(|f| f.size()).unwrap_or(0);
        Self { fields, size }
    }

    pub fn field(&self, idx: usize) -> &ValueVector {
        &self.fields[idx]
    }

    pub fn field_mut(&mut self, idx: usize) -> &mut ValueVector {
        &mut self.fields[idx]
    }

    pub fn num_fields(&self) -> usize {
        self.fields.len()
    }

    pub fn resize(&mut self, new_size: usize) {
        self.size = new_size;
        for field in &mut self.fields {
            field.resize(new_size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_vector_bool() {
        let mut v = ValueVector::new(PhysicalTypeID::Bool, 5);
        v.push_bool(true);
        v.push_bool(false);
        assert_eq!(v.get_bool(0), Some(true));
        assert_eq!(v.get_bool(1), Some(false));
    }

    #[test]
    fn test_physical_type_size() {
        assert_eq!(physical_type_size(PhysicalTypeID::Bool), 1);
        assert_eq!(physical_type_size(PhysicalTypeID::Int64), 8);
        assert_eq!(physical_type_size(PhysicalTypeID::Int32), 4);
        assert_eq!(physical_type_size(PhysicalTypeID::Double), 8);
        assert_eq!(physical_type_size(PhysicalTypeID::String), 16);
    }

    #[test]
    fn test_data_chunk() {
        let v1 = ValueVector::new(PhysicalTypeID::Int64, 10);
        let v2 = ValueVector::new(PhysicalTypeID::Double, 10);
        let chunk = DataChunk::new(vec![v1, v2]);
        assert_eq!(chunk.num_fields(), 2);
    }

    #[test]
    fn test_vector_append() {
        let mut v1 = ValueVector::new(PhysicalTypeID::Int64, 10);
        let mut v2 = ValueVector::new(PhysicalTypeID::Int64, 10);
        v1.set_i64(0, 1);
        v1.set_i64(1, 2);
        v2.set_i64(0, 3);
        v2.set_i64(1, 4);
        v1.append(&v2);
        assert_eq!(v1.size(), 4);
        assert_eq!(v1.get_i64(2), Some(3));
        assert_eq!(v1.get_i64(3), Some(4));
    }
}
