//! Auto-extracted from physical_operator.rs
use crate::physical::common::value_cmp;
use crate::physical::order_aggregate::{is_radix_eligible, radix_sort_indices};
use akar_common::types::Value;

// ==================== BlockMergeSort ====================

/// Block-based parallel sort with k-way merge.
/// Splits data into blocks, sorts each block in parallel, then merges.
pub struct BlockMergeSorter {
    block_size: usize,
    sort_keys: Vec<(u32, bool)>,
}

impl BlockMergeSorter {
    pub fn new(block_size: usize, sort_keys: Vec<(u32, bool)>) -> Self {
        Self { block_size, sort_keys }
    }

    /// Sort data using block-based parallel sort + k-way merge.
    /// `all_values` is a per-column vector of (value, is_null).
    pub fn sort(&self, all_values: &[Vec<(Value, bool)>], num_fields: usize) -> Vec<usize> {
        let total_rows = all_values[0].len();
        if total_rows == 0 {
            return Vec::new();
        }

        let num_blocks = total_rows.div_ceil(self.block_size);

        if num_blocks <= 1 {
            // Single block: sort directly (possibly with radix)
            let mut indices: Vec<usize> = (0..total_rows).collect();
            self.sort_block(&mut indices, all_values, num_fields, 0, total_rows);
            return indices;
        }

        // Sort each block
        let sort_keys = self.sort_keys.clone();
        let block_size = self.block_size;
        let blocks: Vec<Vec<usize>> = (0..num_blocks)
            .map(|bi| {
                let start = bi * block_size;
                let end = (start + block_size).min(total_rows);
                let mut block_indices: Vec<usize> = (start..end).collect();
                Self::sort_block_static(&mut block_indices, all_values, num_fields, &sort_keys);
                block_indices
            })
            .collect();

        // K-way merge
        self.k_way_merge(&blocks, all_values, num_fields, total_rows)
    }

    fn sort_block(
        &self,
        indices: &mut [usize],
        all_values: &[Vec<(Value, bool)>],
        num_fields: usize,
        _start: usize,
        _end: usize,
    ) {
        Self::sort_block_static(indices, all_values, num_fields, &self.sort_keys);
    }

    fn sort_block_static(
        indices: &mut [usize],
        all_values: &[Vec<(Value, bool)>],
        num_fields: usize,
        sort_keys: &[(u32, bool)],
    ) {
        if sort_keys.is_empty() {
            return;
        }
        let (col, ascending) = sort_keys[0];
        let col = col as usize;
        if col >= num_fields {
            return;
        }

        // Try radix sort for Int64 keys
        if is_radix_eligible(&all_values[col]) {
            let keys: Vec<i64> = indices
                .iter()
                .map(|&i| match &all_values[col][i].0 {
                    Value::Int64(v) => *v,
                    _ => i64::MAX, // NULLs sort last
                })
                .collect();
            radix_sort_indices(indices, &keys);
            if !ascending {
                indices.reverse();
            }
            // Tie-break with remaining keys
            if sort_keys.len() > 1 {
                indices.sort_by(|a, b| {
                    for &(k, asc) in sort_keys {
                        let k = k as usize;
                        if k >= num_fields {
                            continue;
                        }
                        let cmp = value_cmp(&all_values[k][*a].0, &all_values[k][*b].0);
                        if cmp != std::cmp::Ordering::Equal {
                            return if asc { cmp } else { cmp.reverse() };
                        }
                    }
                    std::cmp::Ordering::Equal
                });
            }
        } else {
            // Comparison sort
            indices.sort_by(|a, b| {
                for &(k, ascending) in sort_keys {
                    let k = k as usize;
                    if k >= num_fields {
                        continue;
                    }
                    let cmp = value_cmp(&all_values[k][*a].0, &all_values[k][*b].0);
                    if cmp != std::cmp::Ordering::Equal {
                        return if ascending { cmp } else { cmp.reverse() };
                    }
                }
                std::cmp::Ordering::Equal
            });
        }
    }

    /// K-way merge of sorted blocks using BinaryHeap for O(log k) per row.
    /// Stores the first sort key inline to avoid Vec allocation for single-key sorts.
    fn k_way_merge(
        &self,
        blocks: &[Vec<usize>],
        all_values: &[Vec<(Value, bool)>],
        _num_fields: usize,
        total_rows: usize,
    ) -> Vec<usize> {
        use std::collections::BinaryHeap;

        struct HeapEntry {
            block_idx: usize,
            primary: Value,
            primary_asc: bool,
            rest: Vec<Value>,
            rest_asc: Vec<bool>,
        }

        impl Ord for HeapEntry {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                // BinaryHeap pops the "greatest" entry, so an ASC key pops the
                // smallest value (reverse the comparison) while a DESC key pops
                // the largest value (compare directly) (P52.10).
                let cmp = value_cmp(&self.primary, &other.primary);
                if cmp != std::cmp::Ordering::Equal {
                    return if self.primary_asc { cmp.reverse() } else { cmp };
                }
                for ((a, b), asc) in self.rest.iter().zip(other.rest.iter()).zip(self.rest_asc.iter()) {
                    let cmp = value_cmp(a, b);
                    if cmp != std::cmp::Ordering::Equal {
                        return if *asc { cmp.reverse() } else { cmp };
                    }
                }
                self.block_idx.cmp(&other.block_idx).reverse()
            }
        }
        impl PartialOrd for HeapEntry {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }
        impl Eq for HeapEntry {}
        impl PartialEq for HeapEntry {
            fn eq(&self, other: &Self) -> bool {
                self.cmp(other) == std::cmp::Ordering::Equal
            }
        }

        let mut result = Vec::with_capacity(total_rows);
        let mut positions: Vec<usize> = vec![0usize; blocks.len()];
        let mut heap: BinaryHeap<HeapEntry> = BinaryHeap::with_capacity(blocks.len());

        let sk = &self.sort_keys;
        for bi in 0..blocks.len() {
            if !blocks[bi].is_empty() {
                let row = blocks[bi][0];
                let primary = all_values[sk[0].0 as usize][row].0.clone();
                let rest: Vec<Value> = sk[1..]
                    .iter()
                    .map(|&(k, _)| all_values[k as usize][row].0.clone())
                    .collect();
                let rest_asc: Vec<bool> = sk[1..].iter().map(|&(_, asc)| asc).collect();
                heap.push(HeapEntry {
                    block_idx: bi,
                    primary,
                    primary_asc: sk[0].1,
                    rest,
                    rest_asc,
                });
            }
        }

        while let Some(entry) = heap.pop() {
            let bi = entry.block_idx;
            let pos = &mut positions[bi];
            result.push(blocks[bi][*pos]);
            *pos += 1;
            if *pos < blocks[bi].len() {
                let row = blocks[bi][*pos];
                let primary = all_values[sk[0].0 as usize][row].0.clone();
                let rest: Vec<Value> = sk[1..]
                    .iter()
                    .map(|&(k, _)| all_values[k as usize][row].0.clone())
                    .collect();
                let rest_asc: Vec<bool> = sk[1..].iter().map(|&(_, asc)| asc).collect();
                heap.push(HeapEntry {
                    block_idx: bi,
                    primary,
                    primary_asc: sk[0].1,
                    rest,
                    rest_asc,
                });
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_common::types::Value;

    fn to_all_values(cols: Vec<Vec<i64>>) -> Vec<Vec<(Value, bool)>> {
        cols.into_iter()
            .map(|c| c.into_iter().map(|v| (Value::Int64(v), false)).collect())
            .collect()
    }

    fn assert_sorted(
        indices: &[usize],
        all_values: &[Vec<(Value, bool)>],
        sort_keys: &[(u32, bool)],
    ) {
        for w in indices.windows(2) {
            for &(col, asc) in sort_keys {
                let va = &all_values[col as usize][w[0]].0;
                let vb = &all_values[col as usize][w[1]].0;
                let cmp = value_cmp(va, vb);
                if cmp != std::cmp::Ordering::Equal {
                    let ok = if asc { cmp.is_le() } else { cmp.is_ge() };
                    assert!(ok, "wrong order at {w:?} (col {col}, asc {asc}): {va:?} vs {vb:?}");
                    break;
                }
            }
        }
    }

    #[test]
    fn test_block_merge_sort_descending_multi_block() {
        let n = 25_000usize;
        let vals: Vec<i64> = (0..n as i64).collect();
        let all_values = to_all_values(vec![vals]);
        let sorter = BlockMergeSorter::new(10_000, vec![(0u32, false)]);
        let indices = sorter.sort(&all_values, 1);
        assert_eq!(indices.len(), n);
        assert_sorted(&indices, &all_values, &[(0, false)]);
        for w in indices.windows(2) {
            let cmp = value_cmp(&all_values[0][w[0]].0, &all_values[0][w[1]].0);
            assert!(
                cmp.is_gt(),
                "DESC must be strictly decreasing at {w:?}"
            );
        }
    }

    #[test]
    fn test_block_merge_sort_ascending_multi_block() {
        let n = 25_000usize;
        let mut vals: Vec<i64> = (0..n as i64).collect();
        vals.sort_by_key(|&v| v.rotate_left(17));
        let all_values = to_all_values(vec![vals]);
        let sorter = BlockMergeSorter::new(10_000, vec![(0u32, true)]);
        let indices = sorter.sort(&all_values, 1);
        assert_eq!(indices.len(), n);
        assert_sorted(&indices, &all_values, &[(0, true)]);
    }

    #[test]
    fn test_block_merge_sort_secondary_key_direction() {
        let n = 30_000usize;
        let mut prim = Vec::with_capacity(n);
        let mut sec = Vec::with_capacity(n);
        for i in 0..n as i64 {
            prim.push(i / 7);
            sec.push(i % 13);
        }
        let all_values = to_all_values(vec![prim, sec]);
        let sorter = BlockMergeSorter::new(10_000, vec![(0u32, true), (1u32, false)]);
        let indices = sorter.sort(&all_values, 2);
        assert_eq!(indices.len(), n);
        assert_sorted(&indices, &all_values, &[(0, true), (1, false)]);
    }
}
