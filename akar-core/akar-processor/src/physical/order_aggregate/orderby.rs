//! Auto-extracted from physical_operator.rs
use crate::physical::common::{store_value_in_vector, value_cmp};
use crate::physical::order_aggregate::BlockMergeSorter;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use akar_common::types::Value;
use akar_common::vector::{DataChunk, ValueVector};

// ==================== ChunkAccessor ====================

/// Provides random-access by global row index across multiple DataChunks.
/// Eliminates the need to pre-collect all values into Vec<Vec<(Value, bool)>>.
struct ChunkAccessor<'a> {
    chunks: &'a [DataChunk],
    offsets: Vec<usize>,
    num_fields: usize,
}

impl<'a> ChunkAccessor<'a> {
    fn new(chunks: &'a [DataChunk]) -> Self {
        let mut offsets = Vec::with_capacity(chunks.len());
        let mut cum = 0usize;
        for c in chunks {
            offsets.push(cum);
            cum += c.size;
        }
        let num_fields = chunks.first().map(|c| c.num_fields()).unwrap_or(0);
        Self { chunks, offsets, num_fields }
    }

    fn total_rows(&self) -> usize {
        self.offsets.last().map(|&o| o + self.chunks.last().unwrap().size).unwrap_or(0)
    }

    fn resolve(&self, global_row: usize) -> (usize, usize) {
        for (ci, chunk) in self.chunks.iter().enumerate() {
            let offset = self.offsets[ci];
            if global_row < offset + chunk.size {
                return (ci, global_row - offset);
            }
        }
        (self.chunks.len() - 1, 0)
    }

    fn get_value(&self, col: usize, global_row: usize) -> Value {
        let (ci, local) = self.resolve(global_row);
        self.chunks[ci].get_value(col, local).unwrap_or(Value::Null)
    }

    fn is_null(&self, col: usize, global_row: usize) -> bool {
        let (ci, local) = self.resolve(global_row);
        self.chunks[ci].is_null(col, local)
    }

    fn physical_type(&self, col: usize, global_row: usize) -> akar_common::types::PhysicalTypeID {
        let (ci, local) = self.resolve(global_row);
        self.chunks[ci].get_value(col, local).map(|v| v.physical_type()).unwrap_or(akar_common::types::PhysicalTypeID::Int64)
    }
}

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

        let accessor = ChunkAccessor::new(&input);
        let total_rows = accessor.total_rows();
        if total_rows == 0 {
            return Ok(input);
        }

        let num_fields = accessor.num_fields;
        let field_names = input[0].field_names.clone();

        // Use BlockMergeSorter for large data, simple sort for small
        let block_size = 10000usize;
        let indices = if total_rows > block_size && !self.sort_keys.is_empty() {
            let sorter = BlockMergeSorter::new(block_size, self.sort_keys.clone());
            // Multi-block path still needs collected key values for k-way merge
            let mut all_values: Vec<Vec<(Value, bool)>> = (0..num_fields).map(|_| Vec::with_capacity(total_rows)).collect();
            for global_row in 0..total_rows {
                for col in 0..num_fields {
                    let val = accessor.get_value(col, global_row);
                    let is_null = accessor.is_null(col, global_row);
                    all_values[col].push((val, is_null));
                }
            }
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
                        let va = accessor.get_value(col, *a);
                        let vb = accessor.get_value(col, *b);
                        let cmp = value_cmp(&va, &vb);
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
                let phys_type = accessor.physical_type(col, indices[chunk_start]);
                let mut v = ValueVector::new(phys_type, size);
                v.resize(size);
                for (out_idx, &src_idx) in indices[chunk_start..chunk_end].iter().enumerate() {
                    if accessor.is_null(col, src_idx) {
                        v.set_null(out_idx, true);
                    } else {
                        let val = accessor.get_value(col, src_idx);
                        store_value_in_vector(&mut v, out_idx, &val)?;
                    }
                }
                fields.push(v);
            }
            let arrow_fields = fields
                .iter()
                .map(|v| akar_common::arrow_vector::ArrowVector::from_legacy(v).array)
                .collect::<Vec<_>>();
            let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
            output.push(DataChunk::new(arrow_fields, arrow_field_types).with_names(field_names.clone()));
        }
        Ok(output)
    }
}
