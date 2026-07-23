// ========================================================================
// Pass: Foreign Join PushDown Detection
//
// Detects hash joins where one or both sides are backed by foreign tables
// (via TableFunctionCall). When detected, marks the join as eligible for
// push-down optimization, allowing the physical operator to delegate the
// join execution to the foreign database.
//
// Ported from LadybugDB `foreign_join_push_down_optimizer.cpp`.
// ========================================================================

use crate::passes::TreeOptimizationPass;
use akar_planner::logical_operator::*;

pub struct ForeignJoinPushDown;

impl TreeOptimizationPass for ForeignJoinPushDown {
    fn name(&self) -> &str {
        "foreign_join_push_down"
    }

    fn apply_tree(&self, root: &mut LogicalOperator) {
        LogicalOperator::visit_bottom_up(root, &mut |op| {
            if let LogicalOperator::HashJoin(hj) = op {
                // Check if either side is a TableFunctionCall with foreign function name
                let probe_foreign = is_foreign_table_function_call(&hj.probe_side);
                let build_foreign = is_foreign_table_function_call(&hj.build_side);

                if probe_foreign || build_foreign {
                    hj.push_down_eligible = true;
                }
            }
        });
    }
}

/// Known foreign table function name prefixes.
const FOREIGN_FUNCTION_PREFIXES: &[&str] = &["duckdb_", "postgres_", "sqlite_", "neo4j_"];

/// Check if a logical operator is a TableFunctionCall for a foreign database.
fn is_foreign_table_function_call(op: &LogicalOperator) -> bool {
    if let LogicalOperator::TableFunctionCall(tf) = op {
        let lower = tf.function_name.to_lowercase();
        FOREIGN_FUNCTION_PREFIXES
            .iter()
            .any(|&prefix| lower.starts_with(prefix))
    } else {
        false
    }
}

/// Check if a logical operator is an Accumulate.
pub(crate) fn is_accumulate(op: &LogicalOperator) -> bool {
    matches!(op, LogicalOperator::Accumulate(_))
}
