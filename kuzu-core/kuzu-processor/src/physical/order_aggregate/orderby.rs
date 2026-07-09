//! Auto-extracted from physical_operator.rs
use crate::physical::order_aggregate::BlockMergeSorter;
use kuzu_common::types::Value;
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_function::AggregateFunction;
use kuzu_function::aggregate::AggValueState;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::physical::common::{store_value_in_vector, value_cmp, value_hash};
use std::collections::BinaryHeap;


// ==================== OrderBy ====================

pub struct PhysicalOrderBy {
    pub sort_keys: Vec<(u32, bool)>,
}

impl PhysicalOperatorExec for PhysicalOrderBy {
    fn operator_type(&self) -> &str {
        "order_by"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        if input.is_empty() {
            return Ok(Vec::new());
        }

        let total_rows: usize = input.iter().map(|c| c.size).sum();
        if total_rows == 0 {
            return Ok(input);
        }

        // Collect all values per column as Value (supports all types)
        let num_fields = input[0].num_fields();
        let mut all_values: Vec<Vec<(Value, bool)>> = (0..num_fields).map(|_| Vec::with_capacity(total_rows)).collect();

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

        // Use BlockMergeSorter for large data, simple sort for small
        let block_size = 10000usize;
        let indices = if total_rows > block_size && !self.sort_keys.is_empty() {
            let sorter = BlockMergeSorter::new(block_size, self.sort_keys.clone());
            sorter.sort(&all_values, num_fields)
        } else {
            let mut indices: Vec<usize> = (0..total_rows).collect();
            if !self.sort_keys.is_empty() {
                indices.sort_by(|a, b| {
                    for &(col, ascending) in &self.sort_keys {
                        let col = col as usize;
                        if col >= num_fields {
                            continue;
                        }
                        let cmp = value_cmp(&all_values[col][*a].0, &all_values[col][*b].0);
                        if cmp != std::cmp::Ordering::Equal {
                            return if ascending { cmp } else { cmp.reverse() };
                        }
                    }
                    std::cmp::Ordering::Equal
                });
            }
            indices
        };

        // Build sorted output chunks (up to 100 rows per chunk)
        let chunk_size = 100usize;
        let mut output = Vec::new();
        for chunk_start in (0..total_rows).step_by(chunk_size) {
            let chunk_end = (chunk_start + chunk_size).min(total_rows);
            let size = chunk_end - chunk_start;
            let mut fields = Vec::new();
            for col in 0..num_fields {
                let first_val = &all_values[col][indices[chunk_start]].0;
                let phys_type = first_val.physical_type();
                let mut v = ValueVector::new(phys_type, size);
                v.resize(size);
                for (out_idx, &src_idx) in indices[chunk_start..chunk_end].iter().enumerate() {
                    let (ref val, is_null) = all_values[col][src_idx];
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




