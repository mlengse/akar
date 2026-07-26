#![no_main]
use libfuzzer_sys::fuzz_target;
use akar_common::enums::CompressionType;
use akar_storage::compression::{compress, decompress, decompress_serialized_value, CompressedChunk};

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // Fuzz decompress with arbitrary data for each compression type
    let types = [
        CompressionType::Uncompressed,
        CompressionType::Constant,
        CompressionType::OneValue,
        CompressionType::Boolean,
        CompressionType::IntegerBitpacking,
        CompressionType::StringDictionary,
        CompressionType::Float,
    ];

    let num_values = if data.len() >= 4 {
        u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize % 1024
    } else {
        data.len()
    };

    for ct in &types {
        let chunk = CompressedChunk {
            compression: *ct,
            data: data.to_vec(),
            num_values,
        };
        // decompress must not panic on any input
        let _ = decompress(&chunk, num_values * 8);

        // decompress_serialized_value must not panic on any input
        for value_size in &[1, 2, 4, 8] {
            let _ = decompress_serialized_value(*ct, data, *value_size);
        }
    }

    // Fuzz roundtrip: compress then decompress with valid-sized input
    if data.len() >= 8 && data.len() % 8 == 0 {
        let num_values = data.len() / 8;
        for ct in &types {
            let chunk = compress(*ct, data, num_values);
            let result = decompress(&chunk, data.len());
            assert_eq!(result.len(), data.len());
        }
    }
});
