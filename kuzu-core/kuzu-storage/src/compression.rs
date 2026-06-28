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
pub fn compress(compression: CompressionType, data: &[u8]) -> CompressedChunk {
    // TODO: implement actual compression algorithms
    CompressedChunk {
        compression,
        data: data.to_vec(),
        num_values: data.len(),
    }
}

/// Decompress a chunk back to raw bytes.
pub fn decompress(chunk: &CompressedChunk) -> Vec<u8> {
    chunk.data.clone()
}
