//! Individual optimizer passes for logical plan transformation.
//!
//! Each pass implements `OptimizationPass` and transforms a logical plan.
//! Passes are applied in order of registration in the Optimizer.
//!
//! ## Module structure
//!
//! - `flat/` — 14 flat passes that operate on `&[LogicalOperator]`
//! - `tree/` — 7 tree passes that operate on the operator tree in-place

pub mod flat;
pub mod tree;

pub use flat::*;
pub use tree::*;

use akar_planner::logical_operator::LogicalOperator;

/// An optimization pass transforms a logical plan.
pub trait OptimizationPass {
    fn name(&self) -> &str;
    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator>;
}

/// A tree-based optimization pass that transforms a logical plan tree.
///
/// Unlike `OptimizationPass` which works on flat `&[LogicalOperator]`,
/// this trait operates on the operator tree directly, enabling
/// bottom-up traversals and child insertion/deletion.
pub trait TreeOptimizationPass {
    fn name(&self) -> &str;

    /// Apply this pass to the root of a logical plan tree.
    /// The pass may recursively transform the tree in-place.
    fn apply_tree(&self, root: &mut LogicalOperator);
}
