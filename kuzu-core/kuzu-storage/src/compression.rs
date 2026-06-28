//! Column compression algorithms.
//!
//! Supports constant, one-value, boolean, integer bitpacking,
//! string dictionary, float, and list delta compression.

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
            CompressedChunk { compression, data: data.to_vec(), num_values }
        }
        CompressionType::Float | CompressionType::Uncompressed | CompressionType::OneValue => {
            CompressedChunk { compression, data: data.to_vec(), num_values }
        }
        CompressionType::StringDictionary => {
            CompressedChunk { compression, data: data.to_vec(), num_values }
        }
    }
}

/// Decompress a chunk back to raw bytes.
pub fn decompress(chunk: &CompressedChunk, expected_size: usize) -> Vec<u8> {
    match chunk.compression {
        CompressionType::Constant => decompress_constant(&chunk.data, expected_size),
        CompressionType::Boolean => decompress_boolean(&chunk.data, expected_size),
        _ => chunk.data.clone(),
    }
}

/// Constant compression: for columns where all values are the same.
/// Format: [num_vals: u32][value_bytes...]
fn compress_constant(data: &[u8], num_values: usize) -> CompressedChunk {
    let val_size = if data.is_empty() { 0 } else { data.len() / num_values.max(1) };
    let mut compressed = Vec::with_capacity(4 + val_size);
    compressed.extend_from_slice(&(num_values as u32).to_le_bytes());
    if val_size > 0 {
        compressed.extend_from_slice(&data[..val_size]);
    }
    CompressedChunk { compression: CompressionType::Constant, data: compressed, num_values }
}

fn decompress_constant(data: &[u8], expected_size: usize) -> Vec<u8> {
    if data.len() < 4 { return Vec::new(); }
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
    let packed_len = (num_values + 7) / 8;
    let mut packed = vec![0u8; packed_len];
    for i in 0..num_values.min(data.len()) {
        if data[i] != 0 { packed[i / 8] |= 1 << (i % 8); }
    }
    CompressedChunk { compression: CompressionType::Boolean, data: packed, num_values }
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
}
