//! Auto-extracted from physical_operator.rs
use kuzu_common::types::{LogicalTypeID, PhysicalTypeID, Value};
use kuzu_common::vector::{DataChunk, ValueVector, physical_type_size};
use kuzu_parser::ast::{BinaryOp, Constant, Expression, UnaryOp};
use kuzu_storage::table::ColumnDefinition;
use std::sync::{Arc, Mutex};
use crate::expression_evaluator::ExpressionEvaluator;
use crate::physical::types::{OperatorResult, NodeSemiMask, PhysicalOperatorExec};
use crate::physical::write_ops::PhysicalFtsScan;
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
                for field in &chunk.fields {
                    let phys_type = field.physical_type();
                    let mut new_v = ValueVector::new(phys_type, take);
                    new_v.resize(take);
                    for i in 0..take {
                        let src_row = start_in_chunk + i;
                        if field.is_null(src_row) {
                            new_v.set_null(i, true);
                        } else if let Some(val) = field.get_value(src_row) {
                            store_value_in_vector(&mut new_v, i, &val);
                        }
                    }
                    new_fields.push(new_v);
                }
                output.push(DataChunk::new(new_fields).with_names(chunk.field_names.clone()));
            }
        }
        Ok(output)
    }
}

