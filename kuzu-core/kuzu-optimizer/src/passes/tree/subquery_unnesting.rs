// ========================================================================
// Pass: Correlated Subquery Unnesting
//
// Wires ExpressionsScan operators in the build side of an AccHashJoin
// to the outer Accumulate operator on the probe side. This enables
// correlated subquery execution where the inner query reads correlated
// variables from the outer query's accumulated context.
//
// Ported from C++ `correlated_subquery_unnest_solver.cpp`.
// ========================================================================

use crate::passes::tree::foreign_join::is_accumulate;
use crate::passes::TreeOptimizationPass;
use kuzu_planner::logical_operator::*;

pub struct CorrelatedSubqueryUnnesting;

impl CorrelatedSubqueryUnnesting {
    /// Recursively find and wire ExpressionsScan in a subtree to the given accumulate index.
    fn wire_expressions_scans(op: &mut LogicalOperator, acc_idx: usize) {
        if let LogicalOperator::ExpressionsScan(es) = op {
            es.outer_accumulate_idx = Some(acc_idx);
        }
        let children = op.children_mut();
        for child in children {
            Self::wire_expressions_scans(child, acc_idx);
        }
    }
}

impl TreeOptimizationPass for CorrelatedSubqueryUnnesting {
    fn name(&self) -> &str {
        "correlated_subquery_unnesting"
    }

    fn apply_tree(&self, root: &mut LogicalOperator) {
        // First pass: collect (accumulate_idx, build_side_ptr) pairs
        // to avoid borrow conflicts with the closure
        #[allow(dead_code)]
        struct AccHashJoinInfo {
            build_side_idx: usize,
        }

        let mut infos: Vec<AccHashJoinInfo> = Vec::new();
        let mut acc_counter = 0usize;

        LogicalOperator::visit_bottom_up(root, &mut |op| {
            if let LogicalOperator::HashJoin(hj) = op
                && is_accumulate(&hj.probe_side)
            {
                infos.push(AccHashJoinInfo {
                    build_side_idx: acc_counter,
                });
                acc_counter += 1;
            }
        });

        // Second pass: wire ExpressionsScans in build sides
        for info in &infos {
            // Find the HashJoin again and wire its build side
            Self::wire_build_side(root, info.build_side_idx);
        }
    }
}

impl CorrelatedSubqueryUnnesting {
    /// Find the Nth HashJoin with Accumulate on its probe side, then wire ExpressionsScans
    /// in its build side.
    fn wire_build_side(root: &mut LogicalOperator, target_idx: usize) {
        let mut found = 0usize;
        LogicalOperator::visit_bottom_up(root, &mut |op| {
            if let LogicalOperator::HashJoin(hj) = op
                && is_accumulate(&hj.probe_side)
            {
                if found == target_idx {
                    Self::wire_expressions_scans(&mut hj.build_side, target_idx);
                }
                found += 1;
            }
        });
    }
}
