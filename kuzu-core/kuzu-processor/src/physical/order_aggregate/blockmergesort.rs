//! Auto-extracted from physical_operator.rs
use crate::physical::order_aggregate::{is_radix_eligible, radix_sort_indices};
use kuzu_common::types::Value;
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_function::AggregateFunction;
use kuzu_function::aggregate::AggValueState;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::physical::common::{store_value_in_vector, value_cmp, value_hash};
use std::collections::BinaryHeap;


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

    /// K-way merge of sorted blocks using linear scan for minimum.
    fn k_way_merge(
        &self,
        blocks: &[Vec<usize>],
        all_values: &[Vec<(Value, bool)>],
        num_fields: usize,
        total_rows: usize,
    ) -> Vec<usize> {
        let mut result = Vec::with_capacity(total_rows);
        let mut positions: Vec<usize> = vec![0usize; blocks.len()];

        for _ in 0..total_rows {
            // Find the block with the minimum head value
            let mut best_block: Option<usize> = None;
            for bi in 0..blocks.len() {
                if positions[bi] >= blocks[bi].len() {
                    continue; // Block exhausted
                }
                match best_block {
                    None => best_block = Some(bi),
                    Some(bb) => {
                        let cmp = self.compare_rows(
                            blocks[bb][positions[bb]],
                            blocks[bi][positions[bi]],
                            all_values,
                            num_fields,
                        );
                        if cmp == std::cmp::Ordering::Greater {
                            best_block = Some(bi);
                        }
                    }
                }
            }

            if let Some(bi) = best_block {
                result.push(blocks[bi][positions[bi]]);
                positions[bi] += 1;
            } else {
                break;
            }
        }

        result
    }

    fn compare_rows(
        &self,
        a: usize,
        b: usize,
        all_values: &[Vec<(Value, bool)>],
        num_fields: usize,
    ) -> std::cmp::Ordering {
        for &(k, ascending) in &self.sort_keys {
            let k = k as usize;
            if k >= num_fields {
                continue;
            }
            let cmp = value_cmp(&all_values[k][a].0, &all_values[k][b].0);
            if cmp != std::cmp::Ordering::Equal {
                return if ascending { cmp } else { cmp.reverse() };
            }
        }
        std::cmp::Ordering::Equal
    }
}


