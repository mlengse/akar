//! Auto-extracted from physical_operator.rs
use kuzu_common::types::Value;
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_function::AggregateFunction;
use kuzu_function::aggregate::AggValueState;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::physical::common::{store_value_in_vector, value_cmp, value_hash};
use std::collections::BinaryHeap;


// ==================== TopK ====================

/// Fused ORDER BY + LIMIT using a BinaryHeap (O(n log k) vs O(n log n)).
///
/// Maintains a max-heap of size (limit + offset). Pops the worst entry
/// when capacity is exceeded. Uses `DirectedSortKey` to encode sort
/// direction into the comparison, so the BinaryHeap's natural max-heap
/// behavior correctly retains the best entries.
pub struct PhysicalTopK {
    pub sort_keys: Vec<(u32, bool)>,
    pub limit: u64,
    pub offset: u64,
}

/// Wrapper for a sort-key value that embeds sort direction.
#[derive(Debug, Clone)]
enum DirectedSortKey {
    Asc(Value),
    Desc(Value),
}

impl Eq for DirectedSortKey {}
impl PartialEq for DirectedSortKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}
impl PartialOrd for DirectedSortKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for DirectedSortKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (DirectedSortKey::Asc(a), DirectedSortKey::Asc(b)) => value_cmp(a, b),
            (DirectedSortKey::Desc(a), DirectedSortKey::Desc(b)) => value_cmp(b, a),
            _ => std::cmp::Ordering::Equal,
        }
    }
}

#[derive(Debug, Clone)]
struct TopKHeapEntry {
    sort_key: Vec<DirectedSortKey>,
    row_idx: usize,
}

impl Eq for TopKHeapEntry {}
impl PartialEq for TopKHeapEntry {
    fn eq(&self, other: &Self) -> bool { self.sort_key == other.sort_key }
}
impl PartialOrd for TopKHeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> { Some(self.cmp(other)) }
}
impl Ord for TopKHeapEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        for (a, b) in self.sort_key.iter().zip(other.sort_key.iter()) {
            let cmp = a.cmp(b);
            if cmp != std::cmp::Ordering::Equal { return cmp; }
        }
        std::cmp::Ordering::Equal
    }
}

impl PhysicalOperatorExec for PhysicalTopK {
    fn operator_type(&self) -> &str { "top_k" }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let capacity = (self.limit + self.offset) as usize;
        if capacity == 0 || input.is_empty() {
            return Ok(Vec::new());
        }

        let total_rows: usize = input.iter().map(|c| c.size).sum();
        if total_rows == 0 {
            return Ok(Vec::new());
        }

        let num_fields = input[0].num_fields();

        // Collect all values for random access
        let mut all_values: Vec<Vec<(Value, bool)>> = (0..num_fields)
            .map(|_| Vec::with_capacity(total_rows))
            .collect();
        for chunk in &input {
            for row in 0..chunk.size {
                for col in 0..num_fields {
                    if let Some(field) = chunk.fields.get(col) {
                        let val = field.get_value(row).unwrap_or(Value::Null);
                        let is_null = field.is_null(row);
                        all_values[col].push((val, is_null));
                    }
                }
            }
        }

        // BinaryHeap (max-heap): worst entry at top, popped when > capacity.
        let mut heap: BinaryHeap<TopKHeapEntry> =
            BinaryHeap::with_capacity(capacity.min(total_rows) + 1);

        for row_idx in 0..total_rows {
            let sort_key: Vec<DirectedSortKey> = self
                .sort_keys
                .iter()
                .map(|&(col, asc)| {
                    let val = if col as usize >= num_fields {
                        Value::Null
                    } else {
                        all_values[col as usize][row_idx].0.clone()
                    };
                    if asc { DirectedSortKey::Asc(val) } else { DirectedSortKey::Desc(val) }
                })
                .collect();

            heap.push(TopKHeapEntry { sort_key, row_idx });
            if heap.len() > capacity {
                heap.pop();
            }
        }

        // into_sorted_vec returns ascending DirectedSortKey order = best-first
        let sorted: Vec<TopKHeapEntry> = heap.into_sorted_vec();

        // Apply offset + limit
        let start = (self.offset as usize).min(sorted.len());
        let end = (start + self.limit as usize).min(sorted.len());
        let entries = &sorted[start..end];

        if entries.is_empty() {
            return Ok(Vec::new());
        }

        // Build output chunks (up to 100 rows each)
        let chunk_size = 100usize;
        let mut output = Vec::new();
        for chunk_start in (0..entries.len()).step_by(chunk_size) {
            let chunk_end = (chunk_start + chunk_size).min(entries.len());
            let size = chunk_end - chunk_start;
            let mut fields = Vec::new();
            for col in 0..num_fields {
                let first_row = entries[chunk_start].row_idx;
                let first_val = &all_values[col][first_row].0;
                let phys_type = first_val.physical_type();
                let mut v = ValueVector::new(phys_type, size);
                v.resize(size);
                for (out_idx, entry) in entries[chunk_start..chunk_end].iter().enumerate() {
                    let (ref val, is_null) = all_values[col][entry.row_idx];
                    if is_null || matches!(val, Value::Null) {
                        v.set_null(out_idx, true);
                    } else {
                        store_value_in_vector(&mut v, out_idx, val);
                    }
                }
                fields.push(v);
            }
            output.push(DataChunk::new(fields));
        }

        Ok(output)
    }
}

