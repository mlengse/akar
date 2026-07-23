use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use akar_common::vector::DataChunk;

pub struct PhysicalFlatten {
    pub group_pos: usize,
}

impl PhysicalFlatten {
    pub fn new(group_pos: usize) -> Self {
        Self { group_pos }
    }
}

impl PhysicalOperatorExec for PhysicalFlatten {
    fn operator_type(&self) -> &str {
        "flatten"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        Ok(input)
    }
}
