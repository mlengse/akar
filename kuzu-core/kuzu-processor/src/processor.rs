//! Query processor — maps logical operators to physical operators and executes them.

use kuzu_common::vector::DataChunk;
use kuzu_planner::logical_operator::LogicalOperator;

/// The query processor executes a physical plan and produces result chunks.
pub struct QueryProcessor;

impl QueryProcessor {
    pub fn new() -> Self {
        Self
    }

    /// Execute a sequence of logical operators and return result data chunks.
    pub fn execute(
        &self,
        operators: &[LogicalOperator],
    ) -> Result<Vec<DataChunk>, String> {
        // TODO: implement actual physical operator execution
        tracing::debug!("Executing {} logical operators", operators.len());
        Ok(Vec::new())
    }
}

impl Default for QueryProcessor {
    fn default() -> Self {
        Self::new()
    }
}
