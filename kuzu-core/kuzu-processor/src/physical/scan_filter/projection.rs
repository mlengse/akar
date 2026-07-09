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

// ==================== Projection ====================

pub struct PhysicalProjection {
    /// Column indices to include (in order).
    pub column_indices: Vec<usize>,
}

impl PhysicalOperatorExec for PhysicalProjection {
    fn operator_type(&self) -> &str {
        "projection"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let output: Vec<DataChunk> = input
            .into_iter()
            .map(|chunk| {
                let fields: Vec<ValueVector> = self
                    .column_indices
                    .iter()
                    .filter_map(|&i| chunk.fields.get(i).cloned())
                    .collect();
                let size = if fields.is_empty() {
                    chunk.size
                } else {
                    fields.first().map(|f| f.size()).unwrap_or(0)
                };
                let names = self
                    .column_indices
                    .iter()
                    .filter_map(|&i| chunk.field_names.get(i).cloned())
                    .collect();
                DataChunk {
                    fields,
                    size,
                    field_names: names,
                }
            })
            .collect();

        if output.is_empty() {
            Ok(vec![DataChunk {
                fields: vec![],
                size: 0,
                field_names: vec![],
            }])
        } else {
            Ok(output)
        }
    }
}


