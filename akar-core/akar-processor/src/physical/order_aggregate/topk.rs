//! Auto-extracted from physical_operator.rs
use crate::physical::common::{take_global_rows, value_cmp};
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use akar_common::types::Value;
use akar_common::vector::DataChunk;
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
    fn eq(&self, other: &Self) -> bool {
        self.sort_key == other.sort_key
    }
}
impl PartialOrd for TopKHeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for TopKHeapEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        for (a, b) in self.sort_key.iter().zip(other.sort_key.iter()) {
            let cmp = a.cmp(b);
            if cmp != std::cmp::Ordering::Equal {
                return cmp;
            }
        }
        std::cmp::Ordering::Equal
    }
}

impl PhysicalOperatorExec for PhysicalTopK {
    fn operator_type(&self) -> &str {
        "top_k"
    }

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

        // Stream rows into a bounded max-heap directly from the chunks instead
        // of materializing all values into a full O(rows × fields) matrix. Only
        // the sort-key values are needed for the heap (P79).
        let mut heap: BinaryHeap<TopKHeapEntry> = BinaryHeap::with_capacity(capacity.min(total_rows) + 1);
        let mut row_idx: usize = 0;
        for chunk in &input {
            for row in 0..chunk.size {
                let sort_key: Vec<DirectedSortKey> = self
                    .sort_keys
                    .iter()
                    .map(|&(col, asc)| {
                        let val = if col as usize >= num_fields {
                            Value::Null
                        } else {
                            chunk.get_value(col as usize, row).unwrap_or(Value::Null)
                        };
                        if asc {
                            DirectedSortKey::Asc(val)
                        } else {
                            DirectedSortKey::Desc(val)
                        }
                    })
                    .collect();
                heap.push(TopKHeapEntry { sort_key, row_idx });
                row_idx += 1;
                if heap.len() > capacity {
                    heap.pop();
                }
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

        // Capture field_names from input chunks for output propagation
        let field_names = input[0].field_names.clone();

        // Build output chunks (up to 100 rows each). Slice the source fields
        // via Arrow `take`, preserving complex types (List/Struct).
        let global_indices: Vec<usize> = entries.iter().map(|e| e.row_idx).collect();
        let chunk_size = 100usize;
        let mut output = Vec::new();
        for chunk_start in (0..global_indices.len()).step_by(chunk_size) {
            let chunk_end = (chunk_start + chunk_size).min(global_indices.len());
            output.push(take_global_rows(
                &input,
                &global_indices[chunk_start..chunk_end],
                field_names.clone(),
            )?);
        }

        Ok(output)
    }
}
