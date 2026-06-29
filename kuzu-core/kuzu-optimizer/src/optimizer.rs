//! Optimizer — runs a sequence of optimization passes on a logical plan.

use crate::passes::*;
use kuzu_planner::logical_operator::LogicalOperator;
use kuzu_storage::stats::StatsStore;
use std::sync::{Arc, Mutex};

/// The optimizer applies a chain of optimization passes to a logical plan.
///
/// Two kinds of passes are supported:
/// - **Flat passes** (`OptimizationPass`): transform a flat `Vec<LogicalOperator>`.
///   These are applied first to the entire plan.
/// - **Tree passes** (`TreeOptimizationPass`): transform the operator tree in-place
///   via bottom-up traversal. Applied after flat passes.
pub struct Optimizer {
    passes: Vec<Box<dyn OptimizationPass>>,
    tree_passes: Vec<Box<dyn TreeOptimizationPass>>,
}

impl Optimizer {
    pub fn new() -> Self {
        let passes: Vec<Box<dyn OptimizationPass>> = vec![
            // Pass 1: Remove obviously unnecessary operators early
            Box::new(RemoveUnnecessaryOperators),
            // Pass 2: Push filters toward scan nodes (reduces intermediate rows)
            Box::new(FilterPushDown),
            // Pass 3: Remove unused columns from scans (reduces I/O)
            Box::new(ProjectionPushDown),
            // Pass 4: Fold constant expressions
            Box::new(ConstantFolding),
            // Pass 5: Detect aggregate functions in projections
            Box::new(AggregateDetection),
            // Pass 6: Reorder joins for efficiency
            Box::new(JoinOptimization),
            // Pass 7: Detect and combine top-k patterns (ORDER BY + LIMIT)
            Box::new(TopKOptimization),
            // Pass 8: Detect vector similarity search patterns and use index
            Box::new(VectorSimilarityDetection),
            // Pass 9: Detect range scans on PK columns with ART index
            Box::new(ArtRangeScanDetection),
        ];
        let tree_passes: Vec<Box<dyn TreeOptimizationPass>> = vec![
            // Tree pass 1: Insert flatten operators for factorization
            Box::new(FactorizationRewriting),
            // Tree pass 2: Annotate operators with estimated row counts (static heuristics)
            Box::new(CardinalityEstimation::new(None)),
        ];
        Self { passes, tree_passes }
    }

    /// Create an optimizer with a stats store for storage-backed cardinality estimation.
    pub fn with_stats(stats: Arc<Mutex<StatsStore>>) -> Self {
        let passes: Vec<Box<dyn OptimizationPass>> = vec![
            Box::new(RemoveUnnecessaryOperators),
            Box::new(FilterPushDown),
            Box::new(ProjectionPushDown),
            Box::new(ConstantFolding),
            Box::new(AggregateDetection),
            Box::new(JoinOptimization),
            Box::new(TopKOptimization),
            Box::new(VectorSimilarityDetection),
            Box::new(ArtRangeScanDetection),
        ];
        let tree_passes: Vec<Box<dyn TreeOptimizationPass>> = vec![
            Box::new(FactorizationRewriting),
            // Use storage-backed cardinality estimation with real stats
            Box::new(CardinalityEstimation::new(Some(stats))),
        ];
        Self { passes, tree_passes }
    }

    pub fn optimize(&self, operators: Vec<LogicalOperator>) -> Vec<LogicalOperator> {
        // Phase 1: Flat passes (work on the top-level operator list)
        let mut result = operators;
        for pass in &self.passes {
            tracing::debug!("Running optimizer pass: {}", pass.name());
            result = pass.apply(&result);
        }

        // Phase 2: Tree passes (work on the operator tree in-place)
        // Apply to each top-level operator as a subtree root.
        for tree_pass in &self.tree_passes {
            tracing::debug!("Running tree optimizer pass: {}", tree_pass.name());
            for op in &mut result {
                tree_pass.apply_tree(op);
            }
        }

        result
    }

    /// Get the list of registered passes (for debugging/inspection).
    pub fn pass_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.passes.iter().map(|p| p.name()).collect();
        names.extend(self.tree_passes.iter().map(|p| p.name()));
        names
    }
}

impl Default for Optimizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_optimizer_registers_all_passes() {
        let opt = Optimizer::new();
        let names = opt.pass_names();
        assert!(names.contains(&"remove_unnecessary"));
        assert!(names.contains(&"filter_push_down"));
        assert!(names.contains(&"projection_push_down"));
        assert!(names.contains(&"constant_folding"));
        assert!(names.contains(&"join_optimization"));
        assert!(names.contains(&"top_k_optimization"));
        // Tree passes
        assert!(names.contains(&"cardinality_estimation"));
        assert!(names.contains(&"factorization_rewriting"));
        assert!(names.contains(&"aggregate_detection"));
        assert!(names.contains(&"vector_similarity_detection"));
        assert!(names.contains(&"art_range_scan_detection"));
        assert_eq!(names.len(), 11);
    }

    #[test]
    fn test_optimizer_empty_plan() {
        let opt = Optimizer::new();
        let result = opt.optimize(vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_optimizer_preserves_valid_plan() {
        use kuzu_planner::logical_operator::*;
        let plan = vec![LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("a".into()),
            columns: vec!["name".into()],
            cardinality: 0,
        })];
        let opt = Optimizer::new();
        let result = opt.optimize(plan);
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0], LogicalOperator::ScanNode(_)));
    }
}
