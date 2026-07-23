//! Predicate Push-Down — merges Filter predicates into ScanNode.
//!
//! Operates on the flat `Vec<LogicalOperator>` pipeline. Identifies
//! `Filter` operators immediately followed by `ScanNode` operators
//! and merges the filter expression into `ScanNode.predicate`, then
//! removes the `Filter` from the pipeline.
//!
//! This enables `PhysicalScan` to evaluate the predicate during
//! column materialization (lazy materialization), reducing I/O for
//! filtered columns.

use crate::passes::OptimizationPass;
use akar_planner::logical_operator::*;

pub struct PredicatePushDown;

impl OptimizationPass for PredicatePushDown {
    fn name(&self) -> &str {
        "predicate_push_down"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result: Vec<LogicalOperator> = Vec::with_capacity(operators.len());
        let mut i = 0;
        while i < operators.len() {
            if let LogicalOperator::Filter(filter) = &operators[i] {
                if i + 1 < operators.len() {
                    if let LogicalOperator::ScanNode(scan) = &operators[i + 1] {
                        if scan.predicate.is_none() {
                            let mut merged_scan = scan.clone();
                            merged_scan.predicate = Some(filter.expression.clone());
                            result.push(LogicalOperator::ScanNode(merged_scan));
                            i += 2;
                            continue;
                        }
                    }
                }
            }
            result.push(operators[i].clone());
            i += 1;
        }
        result
    }
}
