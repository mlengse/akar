//! Auto-extracted from physical_operator.rs
use kuzu_common::vector::{DataChunk, ValueVector};
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::physical::common::store_value_in_vector;

// ==================== Limit ====================

#[derive(Debug, Clone)]
pub struct PhysicalLimit {
    pub limit: u64,
    pub offset: u64,
}

impl PhysicalOperatorExec for PhysicalLimit {
    fn operator_type(&self) -> &str {
        "limit"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let mut remaining = self.limit;
        let skip = self.offset;
        let mut output = Vec::new();
        let mut skipped: u64 = 0;

        for chunk in input {
            if remaining == 0 {
                break;
            }
            let chunk_size = chunk.size as u64;

            // Apply offset: skip entire chunks before the offset
            if skipped + chunk_size <= skip {
                skipped += chunk_size;
                continue;
            }

            // Calculate start position within this chunk
            let start_in_chunk = if skipped < skip { (skip - skipped) as usize } else { 0 };

            // Mark this chunk as processed
            skipped += chunk_size;

            // Calculate how many rows to take from this chunk
            let available = (chunk_size as usize).saturating_sub(start_in_chunk);
            let take = available.min(remaining as usize);

            if take == 0 {
                continue;
            }

            remaining -= take as u64;

            if start_in_chunk == 0 && take == chunk.size {
                // Full chunk, no truncation needed
                output.push(chunk);
            } else {
                // Partial chunk: copy row-by-row using get_value/store_value_in_vector
                // This correctly handles all Value types (including variable-length ones)
                let mut new_fields = Vec::with_capacity(chunk.fields.len());
                for (col, field) in chunk.fields.iter().enumerate() {
                    let phys_type = chunk.field_types[col];
                    let mut new_v = ValueVector::new(phys_type, take);
                    new_v.resize(take);
                    for i in 0..take {
                        let src_row = start_in_chunk + i;
                        if field.is_null(src_row) {
                            new_v.set_null(i, true);
                        } else if let Some(val) = chunk.get_value(col, src_row) {
                            store_value_in_vector(&mut new_v, i, &val);
                        }
                    }
                    new_fields.push(new_v);
                }
                output.push({ let arrow_fields = new_fields.iter().map(|v| kuzu_common::arrow_vector::ArrowVector::from_legacy(v).array).collect::<Vec<_>>(); let arrow_field_types = new_fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>(); DataChunk::new(arrow_fields, arrow_field_types) }.with_names(chunk.field_names.clone()));
            }
        }
        Ok(output)
    }
}

