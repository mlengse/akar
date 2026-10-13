//! Auto-extracted from physical_operator.rs
use crate::physical::common::take_global_rows;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use akar_common::vector::DataChunk;

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
            // P136 / T034: a zero-row chunk carries schema but no rows to
            // limit, so it must pass through unchanged. Without this it is
            // swallowed by the offset branch below (`skipped + 0 <= skip` is
            // true even for `skip = 0`), and an all-empty input produces zero
            // chunks — the consumer then sees no schema at all and name-based
            // reads fail with `Variable 'X' not found (chunk has no
            // field_names)`. `PhysicalSkip` already passes such chunks through.
            if chunk.size == 0 {
                output.push(chunk);
                continue;
            }
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
                // Partial chunk: slice rows via Arrow `take`, which preserves
                // complex types (List/Struct) that the legacy ValueVector
                // round-trip would drop to NULL.
                let indices: Vec<usize> = (start_in_chunk..start_in_chunk + take).collect();
                output.push(take_global_rows(
                    std::slice::from_ref(&chunk),
                    &indices,
                    chunk.field_names.clone(),
                )?);
            }
        }
        Ok(output)
    }
}
