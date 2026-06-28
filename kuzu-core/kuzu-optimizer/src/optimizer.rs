//! Optimizer — runs a sequence of optimization passes on a logical plan.

use crate::passes::{FilterPushDown, OptimizationPass, ProjectionPushDown};
use kuzu_planner::logical_operator::LogicalOperator;

/// The optimizer applies a chain of optimization passes to a logical plan.
pub struct Optimizer {
    passes: Vec<Box<dyn OptimizationPass>>,
}

impl Optimizer {
    pub fn new() -> Self {
        let passes: Vec<Box<dyn OptimizationPass>> = vec![
            Box::new(FilterPushDown),
            Box::new(ProjectionPushDown),
        ];
        Self { passes }
    }

    pub fn optimize(&self, operators: Vec<LogicalOperator>) -> Vec<LogicalOperator> {
        let mut result = operators;
        for pass in &self.passes {
            tracing::debug!("Running optimizer pass: {}", pass.name());
            result = pass.apply(&result);
        }
        result
    }
}

impl Default for Optimizer {
    fn default() -> Self {
        Self::new()
    }
}
