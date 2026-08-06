//! Core types and traits for physical operators.

use akar_common::error::ProcessorError;
use akar_common::types::Value;
use akar_common::vector::DataChunk;
use std::collections::HashMap;

/// Result of executing a physical operator.
pub type OperatorResult = Result<Vec<DataChunk>, ProcessorError>;

pub type HashJoinBucket = Vec<(Value, Vec<(usize, usize)>)>;
pub type HashJoinTable = HashMap<u64, HashJoinBucket>;

pub trait PhysicalOperatorExec: Send + Sync {
    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult;
    fn operator_type(&self) -> &str;
}
