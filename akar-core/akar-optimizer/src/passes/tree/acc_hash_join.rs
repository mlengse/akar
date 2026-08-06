// ========================================================================
// Pass: Acc Hash Join Optimization
//
// When a hash join's probe side is selective (has filters), this pass:
// 1. Wraps the probe side in an Accumulate operator
// 2. The accumulated result flows through the hash join at execution time
//
// This reduces the amount of data that flows through the hash join probe.
// Ported from C++ `acc_hash_join_optimizer.cpp`.
// ========================================================================

use crate::passes::TreeOptimizationPass;
use akar_planner::logical_operator::*;

pub struct AccHashJoinOptimization;

impl TreeOptimizationPass for AccHashJoinOptimization {
    fn name(&self) -> &str {
        "acc_hash_join"
    }

    fn apply_tree(&self, root: &mut LogicalOperator) {
        LogicalOperator::visit_bottom_up(root, &mut |op| {
            if let LogicalOperator::HashJoin(hj) = op {
                // Check if probe side is selective (contains Filter operators)
                let has_filter = has_filter_in_subtree(&hj.probe_side);
                if !has_filter {
                    return;
                }

                // Wrap probe side in Accumulate
                let probe_card = hj.probe_side.cardinality();
                let probe_op = std::mem::replace(
                    &mut hj.probe_side,
                    Box::new(LogicalOperator::ScanNode(LogicalScanNode {
                        table_name: String::new(),
                        table_id: 0,
                        alias: None,
                        columns: Vec::new(),
                        cardinality: 0,
                        fts_query: None,
                        predicate: None,
                    })),
                );

                let accumulate = LogicalOperator::Accumulate(LogicalAccumulate {
                    accumulate_type: akar_common::enums::AccumulateType::Regular,
                    flat_exprs: Vec::new(),
                    mark: None,
                    children: vec![*probe_op],
                    cardinality: probe_card,
                });

                *hj.probe_side = accumulate;
            }
        });
    }
}

/// Check if a subtree contains any Filter operator.
pub(crate) fn has_filter_in_subtree(op: &LogicalOperator) -> bool {
    match op {
        LogicalOperator::Filter(_) => true,
        _ => {
            for child in op.children() {
                if has_filter_in_subtree(child) {
                    return true;
                }
            }
            false
        }
    }
}
