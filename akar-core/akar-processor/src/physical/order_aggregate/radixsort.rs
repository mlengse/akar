//! Auto-extracted from physical_operator.rs
use akar_common::types::Value;

// ==================== RadixSort ====================

const RADIX_BITS: u32 = 8;
const RADIX_BUCKETS: usize = 1 << RADIX_BITS; // 256

/// LSD radix sort for Int64 indices. Sorts `indices` by the values in `keys`
/// (converted to u64 with sign flip so negative values sort before positive).
pub fn radix_sort_indices(indices: &mut [usize], keys: &[i64]) {
    let len = indices.len();
    if len < 2 {
        return;
    }

    let mut tmp_indices = vec![0usize; len];
    let mut tmp_keys = vec![0u64; len];

    // Flip sign bit so ordering is correct: smallest → largest
    for (i, &k) in keys.iter().enumerate() {
        tmp_keys[i] = (k as u64) ^ (1u64 << 63);
    }

    let mut counts = [0u32; RADIX_BUCKETS];

    for pass in 0..8u32 {
        // Count
        counts.fill(0);
        for &k in &tmp_keys {
            let bucket = ((k >> (pass * RADIX_BITS)) & 0xFF) as usize;
            counts[bucket] += 1;
        }

        // Prefix sum
        let mut total = 0u32;
        for c in counts.iter_mut() {
            let prev = *c;
            *c = total;
            total += prev;
        }

        // Scatter — move both tmp_keys and indices together
        let mut next_keys = vec![0u64; len];
        for (i, &k) in tmp_keys.iter().enumerate() {
            let bucket = ((k >> (pass * RADIX_BITS)) & 0xFF) as usize;
            let pos = counts[bucket] as usize;
            tmp_indices[pos] = indices[i];
            next_keys[pos] = k;
            counts[bucket] += 1;
        }
        indices.copy_from_slice(&tmp_indices);
        tmp_keys = next_keys;
    }
}

/// Check if a sort key column contains only Int64 values (eligible for radix sort).
pub fn is_radix_eligible(values: &[(Value, bool)]) -> bool {
    values.iter().all(|(v, _)| matches!(v, Value::Int64(_) | Value::Null))
}
