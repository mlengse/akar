//! Column compression algorithms.
//!
//! Supports constant, one-value, boolean, integer bitpacking,
//! string dictionary, float, and list delta compression.

use crate::StringDictionary;
use kuzu_common::enums::CompressionType;

/// A compressed column chunk.
#[derive(Debug, Clone)]
pub struct CompressedChunk {
    pub compression: CompressionType,
    pub data: Vec<u8>,
    pub num_values: usize,
}

/// Compress a byte slice using the given algorithm.
pub fn compress(compression: CompressionType, data: &[u8], num_values: usize) -> CompressedChunk {
    match compression {
        CompressionType::Constant => compress_constant(data, num_values),
        CompressionType::Boolean => compress_boolean(data, num_values),
        CompressionType::IntegerBitpacking | CompressionType::ListDelta => {
            // Determine value size from the data length
            let value_size = data.len().checked_div(num_values).unwrap_or(8);
            compress_integer_bitpacking(data, num_values, value_size)
        }
        CompressionType::Float => {
            let value_size = data.len().checked_div(num_values).unwrap_or(4);
            compress_float(data, num_values, value_size)
        }
        CompressionType::OneValue | CompressionType::Uncompressed => CompressedChunk {
            compression,
            data: data.to_vec(),
            num_values,
        },
        CompressionType::StringDictionary => {
            compress_string_dictionary(data, num_values)
        }
    }
}

/// Decompress a chunk back to raw bytes.
pub fn decompress(chunk: &CompressedChunk, expected_size: usize) -> Vec<u8> {
    match chunk.compression {
        CompressionType::Constant => decompress_constant(&chunk.data, expected_size),
        CompressionType::Boolean => decompress_boolean(&chunk.data, expected_size),
        CompressionType::IntegerBitpacking => decompress_integer_bitpacking(&chunk.data, expected_size),
        CompressionType::Float => decompress_float(&chunk.data, expected_size),
        CompressionType::StringDictionary => decompress_string_dictionary(&chunk.data, expected_size),
        _ => chunk.data.clone(),
    }
}

// =====================================================================
// String Dictionary — deduplicate strings via ID encoding
// =====================================================================

fn compress_string_dictionary(data: &[u8], num_values: usize) -> CompressedChunk {
    let dict = StringDictionary::deserialize(data).unwrap_or_default();
    let dict_bytes = dict.serialize();
    CompressedChunk {
        compression: CompressionType::StringDictionary,
        data: dict_bytes,
        num_values,
    }
}

fn decompress_string_dictionary(data: &[u8], _expected_size: usize) -> Vec<u8> {
    let dict = match StringDictionary::deserialize(data) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    dict.serialize()
}

// =====================================================================
// Integer Bitpacking — encode integers in the minimum bytes needed
// =====================================================================
//
// Each integer is stored as: [num_bytes: u8][value_bytes...]
// where num_bytes is the minimum number of bytes needed to represent
// the value (1-8 for i64, 1-4 for i32, etc.).
//
// For batch compression of a page, we determine the common bit width
// across all values and pack them tightly.

/// Compress a single integer value using bitpacking.
///
/// Strips leading zero bytes from the LE representation.
/// Format: [used_bytes: u8][significant_bytes...]
fn compress_integer_impl(value_bytes: &[u8]) -> Vec<u8> {
    let n = value_bytes.len();
    // Find the last non-zero byte from the high end of the LE representation.
    let significant = (0..n).rev().find(|&i| value_bytes[i] != 0).map_or(0, |i| i + 1);
    let used = significant.max(1); // at least 1 byte
    let mut out = Vec::with_capacity(1 + used);
    out.push(used as u8);
    out.extend_from_slice(&value_bytes[..used]);
    out
}

/// Decompress a single integer value packed by `compress_integer_impl`.
///
/// Format: [used_bytes: u8][significant_bytes...]
/// Returns the value expanded to `original_size` bytes (zero-extended).
fn decompress_integer_impl(data: &[u8], original_size: usize) -> Vec<u8> {
    if data.is_empty() {
        return vec![0u8; original_size];
    }
    let used = data[0] as usize;
    let used = used.min(original_size);
    let mut out = vec![0u8; original_size];
    let avail = data.len().saturating_sub(1);
    let copy = used.min(avail);
    out[..copy].copy_from_slice(&data[1..1 + copy]);
    out
}

/// Integer bitpacking compression for a batch of values.
/// Format: [value_size: u8][num_values: u32][packed_values...]
/// Each packed value is [used_bytes: u8][significant_bytes...]
pub fn compress_integer_bitpacking(data: &[u8], num_values: usize, value_size: usize) -> CompressedChunk {
    let mut compressed = Vec::with_capacity(data.len());
    compressed.push(value_size as u8);
    compressed.extend_from_slice(&(num_values as u32).to_le_bytes());

    let mut offset = 0;
    for _ in 0..num_values {
        if offset + value_size > data.len() {
            break;
        }
        let val_bytes = &data[offset..offset + value_size];
        let packed = compress_integer_impl(val_bytes);
        compressed.extend_from_slice(&packed);
        offset += value_size;
    }

    CompressedChunk {
        compression: CompressionType::IntegerBitpacking,
        data: compressed,
        num_values,
    }
}

fn decompress_integer_bitpacking(data: &[u8], expected_size: usize) -> Vec<u8> {
    if data.len() < 5 {
        return Vec::new();
    }
    let value_size = data[0] as usize;
    let num_values = u32::from_le_bytes(data[1..5].try_into().unwrap()) as usize;
    let mut result = Vec::with_capacity(expected_size.max(num_values * value_size));
    let mut offset = 5;

    for _ in 0..num_values {
        if offset >= data.len() {
            break;
        }
        let used = data[offset] as usize;
        let total = 1 + used.min(value_size);
        if offset + total > data.len() {
            break;
        }
        let val_bytes = &data[offset..offset + total];
        let expanded = decompress_integer_impl(val_bytes, value_size);
        result.extend_from_slice(&expanded);
        offset += total;
    }

    result
}

// =====================================================================
// Float compression (Delta/Offset + Integer Bitpacking)
// =====================================================================

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatCompressionStrategy {
    Raw = 0,
    Delta = 1,
    Offset = 2,
}

/// Float compression: uses integer bitpacking on delta/offset representations.
/// Format: [value_size: u8][num_values: u32][strategy: u8][payload]
pub fn compress_float(data: &[u8], num_values: usize, value_size: usize) -> CompressedChunk {
    if num_values == 0 || (value_size != 4 && value_size != 8) {
        return compress_float_raw(data, num_values, value_size);
    }
    
    let mut ints_u32 = Vec::new();
    let mut ints_u64 = Vec::new();
    
    if value_size == 4 {
        ints_u32.reserve(num_values);
        let mut offset = 0;
        for _ in 0..num_values {
            if offset + 4 > data.len() { break; }
            ints_u32.push(u32::from_le_bytes(data[offset..offset+4].try_into().unwrap()));
            offset += 4;
        }
    } else {
        ints_u64.reserve(num_values);
        let mut offset = 0;
        for _ in 0..num_values {
            if offset + 8 > data.len() { break; }
            ints_u64.push(u64::from_le_bytes(data[offset..offset+8].try_into().unwrap()));
            offset += 8;
        }
    }
    
    let actual_values = if value_size == 4 { ints_u32.len() } else { ints_u64.len() };
    if actual_values == 0 {
        return compress_float_raw(data, num_values, value_size);
    }

    let mut offset_data = Vec::with_capacity(actual_values * value_size);
    let mut delta_data = Vec::with_capacity(actual_values * value_size);

    if value_size == 4 {
        let min_val = *ints_u32.iter().min().unwrap_or(&0);
        for &v in &ints_u32 {
            let diff = v.wrapping_sub(min_val);
            offset_data.extend_from_slice(&diff.to_le_bytes());
        }
        
        let mut prev = 0u32;
        for (i, &v) in ints_u32.iter().enumerate() {
            let diff = if i == 0 { v } else { v.wrapping_sub(prev) };
            delta_data.extend_from_slice(&diff.to_le_bytes());
            prev = v;
        }
    } else {
        let min_val = *ints_u64.iter().min().unwrap_or(&0);
        for &v in &ints_u64 {
            let diff = v.wrapping_sub(min_val);
            offset_data.extend_from_slice(&diff.to_le_bytes());
        }
        
        let mut prev = 0u64;
        for (i, &v) in ints_u64.iter().enumerate() {
            let diff = if i == 0 { v } else { v.wrapping_sub(prev) };
            delta_data.extend_from_slice(&diff.to_le_bytes());
            prev = v;
        }
    }

    let offset_chunk = compress_integer_bitpacking(&offset_data, actual_values, value_size);
    let delta_chunk = compress_integer_bitpacking(&delta_data, actual_values, value_size);
    
    let raw_len = actual_values * value_size;
    let offset_payload_len = value_size + offset_chunk.data.len().saturating_sub(5);
    let delta_payload_len = delta_chunk.data.len().saturating_sub(5);
    
    let min_len = raw_len.min(offset_payload_len).min(delta_payload_len);
    
    let mut compressed = Vec::new();
    compressed.push(value_size as u8);
    compressed.extend_from_slice(&(num_values as u32).to_le_bytes());
    
    if min_len == raw_len {
        compressed.push(FloatCompressionStrategy::Raw as u8);
        compressed.extend_from_slice(&data[..raw_len]);
    } else if min_len == offset_payload_len {
        compressed.push(FloatCompressionStrategy::Offset as u8);
        if value_size == 4 {
            let min_val = *ints_u32.iter().min().unwrap();
            compressed.extend_from_slice(&min_val.to_le_bytes());
        } else {
            let min_val = *ints_u64.iter().min().unwrap();
            compressed.extend_from_slice(&min_val.to_le_bytes());
        }
        if offset_chunk.data.len() >= 5 {
            compressed.extend_from_slice(&offset_chunk.data[5..]);
        }
    } else {
        compressed.push(FloatCompressionStrategy::Delta as u8);
        if delta_chunk.data.len() >= 5 {
            compressed.extend_from_slice(&delta_chunk.data[5..]);
        }
    }
    
    CompressedChunk {
        compression: CompressionType::Float,
        data: compressed,
        num_values,
    }
}

fn compress_float_raw(data: &[u8], num_values: usize, value_size: usize) -> CompressedChunk {
    let mut compressed = Vec::with_capacity(6 + data.len());
    compressed.push(value_size as u8);
    compressed.extend_from_slice(&(num_values as u32).to_le_bytes());
    compressed.push(FloatCompressionStrategy::Raw as u8);
    let byte_count = num_values * value_size;
    compressed.extend_from_slice(&data[..byte_count.min(data.len())]);

    CompressedChunk {
        compression: CompressionType::Float,
        data: compressed,
        num_values,
    }
}

fn decompress_float(data: &[u8], expected_size: usize) -> Vec<u8> {
    if data.len() < 6 {
        return Vec::new();
    }
    let value_size = data[0] as usize;
    let num_values = u32::from_le_bytes(data[1..5].try_into().unwrap()) as usize;
    let strategy = data[5];
    let expected = expected_size.max(num_values * value_size);
    
    if strategy == FloatCompressionStrategy::Raw as u8 {
        let mut result = vec![0u8; expected];
        let avail = data.len().saturating_sub(6);
        let byte_count = num_values * value_size;
        let copy = byte_count.min(avail);
        result[..copy].copy_from_slice(&data[6..6 + copy]);
        return result;
    }
    
    let mut int_pack_data = Vec::new();
    int_pack_data.push(value_size as u8);
    int_pack_data.extend_from_slice(&(num_values as u32).to_le_bytes());
    
    if strategy == FloatCompressionStrategy::Offset as u8 {
        let mut offset_idx = 6;
        if offset_idx + value_size > data.len() {
            return Vec::new();
        }
        let min_val_bytes = &data[offset_idx..offset_idx+value_size];
        offset_idx += value_size;
        int_pack_data.extend_from_slice(&data[offset_idx..]);
        
        let unpacked = decompress_integer_bitpacking(&int_pack_data, num_values * value_size);
        let mut result = vec![0u8; expected];
        if value_size == 4 {
            let min_val = u32::from_le_bytes(min_val_bytes.try_into().unwrap());
            for i in 0..num_values {
                if i * 4 + 4 > unpacked.len() { break; }
                let diff = u32::from_le_bytes(unpacked[i*4..i*4+4].try_into().unwrap());
                let val = diff.wrapping_add(min_val);
                result[i*4..i*4+4].copy_from_slice(&val.to_le_bytes());
            }
        } else {
            let min_val = u64::from_le_bytes(min_val_bytes.try_into().unwrap());
            for i in 0..num_values {
                if i * 8 + 8 > unpacked.len() { break; }
                let diff = u64::from_le_bytes(unpacked[i*8..i*8+8].try_into().unwrap());
                let val = diff.wrapping_add(min_val);
                result[i*8..i*8+8].copy_from_slice(&val.to_le_bytes());
            }
        }
        return result;
    }
    
    if strategy == FloatCompressionStrategy::Delta as u8 {
        int_pack_data.extend_from_slice(&data[6..]);
        let unpacked = decompress_integer_bitpacking(&int_pack_data, num_values * value_size);
        
        let mut result = vec![0u8; expected];
        if value_size == 4 {
            let mut prev = 0u32;
            for i in 0..num_values {
                if i * 4 + 4 > unpacked.len() { break; }
                let diff = u32::from_le_bytes(unpacked[i*4..i*4+4].try_into().unwrap());
                let val = if i == 0 { diff } else { prev.wrapping_add(diff) };
                result[i*4..i*4+4].copy_from_slice(&val.to_le_bytes());
                prev = val;
            }
        } else {
            let mut prev = 0u64;
            for i in 0..num_values {
                if i * 8 + 8 > unpacked.len() { break; }
                let diff = u64::from_le_bytes(unpacked[i*8..i*8+8].try_into().unwrap());
                let val = if i == 0 { diff } else { prev.wrapping_add(diff) };
                result[i*8..i*8+8].copy_from_slice(&val.to_le_bytes());
                prev = val;
            }
        }
        return result;
    }
    
    Vec::new()
}

// =====================================================================
// Individual value compression (for Column integration)
// =====================================================================

/// Compress a single serialized value.
///
/// The serialized format is: [tag: u8][payload...]
/// For IntegerBitpacking/Float, only the payload (after the tag) is compressed.
/// The tag byte is always preserved as the first byte of the output.
///
/// Returns: [tag: u8][compressed_payload...]
/// For pass-through types, the payload is unchanged.
pub fn compress_serialized_value(compression: CompressionType, raw: &[u8], value_size: usize) -> Vec<u8> {
    if raw.is_empty() {
        return Vec::new();
    }
    let tag = raw[0];
    let payload = &raw[1..];

    match compression {
        CompressionType::IntegerBitpacking if value_size > 0 && value_size <= 8 => {
            // Compress only the payload, keep the tag
            let packed = compress_integer_impl(payload);
            let mut out = Vec::with_capacity(1 + packed.len());
            out.push(tag);
            out.extend_from_slice(&packed);
            out
        }
        CompressionType::Float if value_size > 0 && value_size <= 8 => {
            // Float pass-through: keep tag + raw payload
            let mut out = Vec::with_capacity(1 + 1 + payload.len());
            out.push(tag);
            out.push(payload.len() as u8);
            out.extend_from_slice(payload);
            out
        }
        _ => {
            // Pass-through: keep tag + payload as-is
            let mut out = Vec::with_capacity(raw.len());
            out.extend_from_slice(raw);
            out
        }
    }
}

/// Decompress a single serialized value.
///
/// Input format: [tag: u8][compressed_payload...]
/// Output format: [tag: u8][decompressed_payload...]
pub fn decompress_serialized_value(compression: CompressionType, compressed: &[u8], value_size: usize) -> Vec<u8> {
    if compressed.is_empty() {
        return Vec::new();
    }
    let tag = compressed[0];
    let stored_payload = &compressed[1..];

    match compression {
        CompressionType::IntegerBitpacking if value_size > 0 && value_size <= 8 => {
            let expanded = decompress_integer_impl(stored_payload, value_size);
            let mut out = Vec::with_capacity(1 + expanded.len());
            out.push(tag);
            out.extend_from_slice(&expanded);
            out
        }
        CompressionType::Float if value_size > 0 && value_size <= 8 => {
            // Float stored as [tag][len_byte][payload...]
            if stored_payload.is_empty() {
                return compressed.to_vec();
            }
            let len = stored_payload[0] as usize;
            let len = len.min(stored_payload.len().saturating_sub(1));
            let mut out = Vec::with_capacity(1 + len);
            out.push(tag);
            out.extend_from_slice(&stored_payload[1..1 + len]);
            out
        }
        _ => {
            // Pass-through: return as-is
            compressed.to_vec()
        }
    }
}

/// Determine the byte size of a serialized primitive value based on its
/// physical type. Used to know how many bytes to expect for compression.
pub fn serialized_value_size(physical_type: kuzu_common::types::PhysicalTypeID) -> usize {
    use kuzu_common::types::PhysicalTypeID;
    match physical_type {
        PhysicalTypeID::Int64 | PhysicalTypeID::UInt64 | PhysicalTypeID::Double => 8,
        PhysicalTypeID::Int32 | PhysicalTypeID::UInt32 | PhysicalTypeID::Float => 4,
        PhysicalTypeID::Int16 | PhysicalTypeID::UInt16 => 2,
        PhysicalTypeID::Int8 | PhysicalTypeID::UInt8 | PhysicalTypeID::Bool => 1,
        PhysicalTypeID::Interval => 16,
        _ => 0, // variable-length types (string, list, struct, etc.)
    }
}

/// Constant compression: for columns where all values are the same.
/// Format: [num_vals: u32][value_bytes...]
fn compress_constant(data: &[u8], num_values: usize) -> CompressedChunk {
    let val_size = if data.is_empty() {
        0
    } else {
        data.len() / num_values.max(1)
    };
    let mut compressed = Vec::with_capacity(4 + val_size);
    compressed.extend_from_slice(&(num_values as u32).to_le_bytes());
    if val_size > 0 {
        compressed.extend_from_slice(&data[..val_size]);
    }
    CompressedChunk {
        compression: CompressionType::Constant,
        data: compressed,
        num_values,
    }
}

fn decompress_constant(data: &[u8], expected_size: usize) -> Vec<u8> {
    if data.len() < 4 {
        return Vec::new();
    }
    let mut arr = [0u8; 4];
    arr.copy_from_slice(&data[..4]);
    let num_vals = u32::from_le_bytes(arr) as usize;
    let val_bytes = &data[4..];
    let mut result = Vec::with_capacity(expected_size);
    for _ in 0..num_vals {
        result.extend_from_slice(val_bytes);
    }
    result
}

/// Boolean compression: pack 8 booleans per byte.
fn compress_boolean(data: &[u8], num_values: usize) -> CompressedChunk {
    let packed_len = num_values.div_ceil(8);
    let mut packed = vec![0u8; packed_len];
    for i in 0..num_values.min(data.len()) {
        if data[i] != 0 {
            packed[i / 8] |= 1 << (i % 8);
        }
    }
    CompressedChunk {
        compression: CompressionType::Boolean,
        data: packed,
        num_values,
    }
}

fn decompress_boolean(data: &[u8], num_values: usize) -> Vec<u8> {
    let mut result = vec![0u8; num_values];
    for i in 0..num_values {
        result[i] = if data[i / 8] & (1 << (i % 8)) != 0 { 1 } else { 0 };
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_roundtrip() {
        let original = vec![42u8; 100];
        let chunk = compress(CompressionType::Constant, &original, 100);
        assert!(chunk.data.len() < original.len());
        let dec = decompress(&chunk, 100);
        assert_eq!(dec.len(), 100);
        assert_eq!(dec[0], 42);
        assert_eq!(dec[99], 42);
    }

    #[test]
    fn test_boolean_roundtrip() {
        let mut original = vec![0u8; 16];
        original[0] = 1;
        original[7] = 1;
        original[15] = 1;
        let chunk = compress(CompressionType::Boolean, &original, 16);
        assert!(chunk.data.len() < original.len());
        let dec = decompress(&chunk, 16);
        assert_eq!(dec[0], 1);
        assert_eq!(dec[7], 1);
        assert_eq!(dec[15], 1);
        assert_eq!(dec[1], 0);
        assert_eq!(dec[8], 0);
    }

    #[test]
    fn test_uncompressed_roundtrip() {
        let original = vec![1u8, 2, 3, 4, 5];
        let chunk = compress(CompressionType::Uncompressed, &original, 5);
        assert_eq!(chunk.data, original);
        let dec = decompress(&chunk, 5);
        assert_eq!(dec, original);
    }

    // --- Integer bitpacking ---

    #[test]
    fn test_compress_integer_small() {
        // i64 value 42 = 0x2A in LE → 1 significant byte
        let val = 42i64.to_le_bytes();
        let packed = super::compress_integer_impl(&val);
        assert_eq!(packed[0], 1); // 1 byte used
        assert_eq!(packed[1], 42);
    }

    #[test]
    fn test_compress_integer_large() {
        // i64 value 0x12345678 = needs 4 bytes in LE
        let val = 0x12345678i64.to_le_bytes();
        let packed = super::compress_integer_impl(&val);
        assert_eq!(packed[0], 4); // 4 bytes used
        // First 4 bytes should match
        assert_eq!(&packed[1..5], &val[..4]);
    }

    #[test]
    fn test_compress_integer_negative() {
        // i64 -1 = 0xFF..FF in LE → all 8 bytes significant
        let val = (-1i64).to_le_bytes();
        let packed = super::compress_integer_impl(&val);
        assert_eq!(packed[0], 8); // all 8 bytes
    }

    #[test]
    fn test_integer_roundtrip_batch() {
        let values: Vec<i64> = vec![0, 1, 42, 127, 255, 1000, 65535, 100000, -1, -128];
        let value_size = 8;
        let mut data = Vec::with_capacity(values.len() * value_size);
        for v in &values {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let chunk = compress(CompressionType::IntegerBitpacking, &data, values.len());
        let dec = decompress(&chunk, values.len() * value_size);
        assert_eq!(dec.len(), values.len() * value_size);
        for (i, v) in values.iter().enumerate() {
            let val = i64::from_le_bytes(dec[i * value_size..(i + 1) * value_size].try_into().unwrap());
            assert_eq!(val, *v, "mismatch at index {}", i);
        }
    }

    #[test]
    fn test_single_value_compress_decompress() {
        // Simulate a serialized value with tag byte (Int64 = 0x02)
        let mut raw = vec![0x02u8]; // tag byte
        raw.extend_from_slice(&42i64.to_le_bytes()); // payload
        assert_eq!(raw.len(), 9);

        let compressed = super::compress_serialized_value(
            CompressionType::IntegerBitpacking,
            &raw,
            8, // value_size for i64
        );
        // Tag byte should be preserved
        assert_eq!(compressed[0], 0x02);
        // Compressed payload should be smaller than 8 bytes for value 42
        assert!(compressed.len() < raw.len(), "compression should reduce size");

        let decompressed = super::decompress_serialized_value(CompressionType::IntegerBitpacking, &compressed, 8);
        assert_eq!(decompressed, raw, "full roundtrip should match original");

        // Verify value roundtrips correctly
        let restored = i64::from_le_bytes(decompressed[1..9].try_into().unwrap());
        assert_eq!(restored, 42);
    }

    // --- Float compression ---

    #[test]
    fn test_float_batch_roundtrip_raw() {
        // High variance, will likely use Raw
        let values: Vec<f64> = vec![1.0, 3.15, -2.5, 0.0, 1e10, f64::MAX, f64::MIN];
        let value_size = 8;
        let mut data = Vec::with_capacity(values.len() * value_size);
        for v in &values {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let chunk = compress(CompressionType::Float, &data, values.len());
        assert_eq!(chunk.data[5], FloatCompressionStrategy::Raw as u8);
        let dec = decompress(&chunk, values.len() * value_size);
        for (i, v) in values.iter().enumerate() {
            let val = f64::from_le_bytes(dec[i * value_size..(i + 1) * value_size].try_into().unwrap());
            assert_eq!(val.to_bits(), v.to_bits(), "mismatch at index {}", i);
        }
    }

    #[test]
    fn test_float_batch_roundtrip_delta() {
        // Linearly increasing, suitable for Delta compression
        let mut values: Vec<f32> = Vec::new();
        for i in 0..100 {
            values.push(i as f32 * 1.5);
        }
        let value_size = 4;
        let mut data = Vec::with_capacity(values.len() * value_size);
        for v in &values {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let chunk = compress(CompressionType::Float, &data, values.len());
        assert_eq!(chunk.data[5], FloatCompressionStrategy::Delta as u8);
        assert!(chunk.data.len() < data.len() + 6);
        let dec = decompress(&chunk, values.len() * value_size);
        for (i, v) in values.iter().enumerate() {
            let val = f32::from_le_bytes(dec[i * value_size..(i + 1) * value_size].try_into().unwrap());
            assert_eq!(val.to_bits(), v.to_bits(), "mismatch at index {}", i);
        }
    }

    #[test]
    fn test_float_batch_roundtrip_offset() {
        // Near values, suitable for Offset compression
        let values: Vec<f64> = vec![1000.1, 1000.15, 1000.0, 1000.05, 1000.2];
        let value_size = 8;
        let mut data = Vec::with_capacity(values.len() * value_size);
        for v in &values {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let chunk = compress(CompressionType::Float, &data, values.len());
        // Could be offset or delta depending on sizes, both should compress well
        assert!(chunk.data[5] == FloatCompressionStrategy::Offset as u8 || chunk.data[5] == FloatCompressionStrategy::Delta as u8);
        let dec = decompress(&chunk, values.len() * value_size);
        for (i, v) in values.iter().enumerate() {
            let val = f64::from_le_bytes(dec[i * value_size..(i + 1) * value_size].try_into().unwrap());
            assert_eq!(val.to_bits(), v.to_bits(), "mismatch at index {}", i);
        }
    }

    // --- Pass-through behavior ---

    #[test]
    fn test_pass_through_roundtrip() {
        // String data (tag + payload) with IntegerBitpacking as pass-through
        // since value_size=0 for variable-length types
        let mut raw = vec![0x0D]; // TAG_STRING
        raw.extend_from_slice(b"hello world");
        let compressed = super::compress_serialized_value(
            CompressionType::IntegerBitpacking,
            &raw,
            0, // value_size=0 for variable-length
        );
        // Should be pass-through (unchanged)
        assert_eq!(compressed, raw);
        let decompressed = super::decompress_serialized_value(CompressionType::IntegerBitpacking, &compressed, 0);
        assert_eq!(decompressed, raw);
    }

    #[test]
    fn test_compression_metadata() {
        // Verify Constant compression shrinks data
        let data = vec![42u8; 100];
        let chunk = compress(CompressionType::Constant, &data, 100);
        assert_eq!(chunk.compression, CompressionType::Constant);
        assert_eq!(chunk.num_values, 100);
        assert!(chunk.data.len() < data.len());
    }
}
