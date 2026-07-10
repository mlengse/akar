//! Miscellaneous physical operators (EmptyResult, MultiplicityReducer, Skip, UnionAllScan).

use kuzu_common::vector::{DataChunk, ValueVector};
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};

/// Physical operator that always returns an empty result.
pub struct PhysicalEmptyResult;

impl PhysicalOperatorExec for PhysicalEmptyResult {
    fn operator_type(&self) -> &str {
        "empty_result"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        Ok(vec![])
    }
}

/// Physical operator that reduces the multiplicity of paths (e.g., DISTINCT).
pub struct PhysicalMultiplicityReducer;

impl PhysicalOperatorExec for PhysicalMultiplicityReducer {
    fn operator_type(&self) -> &str {
        "multiplicity_reducer"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        // Simple stub: ideally we track distinct row hashes or node identities.
        // For now, it passes the input directly.
        Ok(input)
    }
}

/// Physical operator for SKIP (OFFSET) in queries.
pub struct PhysicalSkip {
    pub skip_count: usize,
}

impl PhysicalOperatorExec for PhysicalSkip {
    fn operator_type(&self) -> &str {
        "skip"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let mut remaining_skip = self.skip_count;
        let mut output = Vec::new();

        for chunk in input {
            if remaining_skip == 0 {
                output.push(chunk);
                continue;
            }

            if chunk.size <= remaining_skip {
                remaining_skip -= chunk.size;
                continue;
            }

            // Partially skip this chunk
            let keep_size = chunk.size - remaining_skip;
            let mut sliced_fields = Vec::new();

            for field in &chunk.fields {
                let mut new_field = ValueVector::new(field.physical_type(), keep_size);
                for i in 0..keep_size {
                    new_field.set_value(i, &field.get_value(remaining_skip + i).unwrap_or(kuzu_common::types::Value::Null)).unwrap();
                }
                sliced_fields.push(new_field);
            }

            output.push(DataChunk {
                fields: sliced_fields,
                size: keep_size,
                field_names: chunk.field_names.clone(),
            });
            remaining_skip = 0;
        }

        Ok(output)
    }
}

/// Physical operator for scanning from a UNION ALL.
pub struct PhysicalUnionAllScan;

impl PhysicalOperatorExec for PhysicalUnionAllScan {
    fn operator_type(&self) -> &str {
        "union_all_scan"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        Ok(input)
    }
}
