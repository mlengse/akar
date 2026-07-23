//! Auto-extracted from physical_operator.rs
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use akar_common::vector::DataChunk;

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
                let fields = self
                    .column_indices
                    .iter()
                    .filter_map(|&i| chunk.fields.get(i).cloned())
                    .collect::<Vec<_>>();
                let field_types = self
                    .column_indices
                    .iter()
                    .filter_map(|&i| chunk.field_types.get(i).cloned())
                    .collect::<Vec<_>>();
                let size = if fields.is_empty() {
                    chunk.size
                } else {
                    fields.first().map(|f| f.len()).unwrap_or(0)
                };
                let names = self
                    .column_indices
                    .iter()
                    .filter_map(|&i| chunk.field_names.get(i).cloned())
                    .collect();
                DataChunk {
                    fields,
                    field_types,
                    size,
                    field_names: names,
                    sel_vector: None,
                }
            })
            .collect();

        if output.is_empty() {
            Ok(vec![DataChunk {
                fields: vec![],
                field_types: vec![],
                size: 0,
                field_names: vec![],
                sel_vector: None,
            }])
        } else {
            Ok(output)
        }
    }
}
