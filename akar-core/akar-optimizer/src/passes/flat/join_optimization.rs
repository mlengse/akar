// ========================================================================
// Pass 6: Join Optimization
// Converts filter equality conditions to join conditions.
// Reorders joins so the smallest tables are joined first (cardinality-aware).
// ========================================================================

use crate::passes::OptimizationPass;
use akar_planner::logical_operator::*;

pub struct JoinOptimization;

impl OptimizationPass for JoinOptimization {
    fn name(&self) -> &str {
        "join_optimization"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        // Try cardinality-aware join reordering
        if let Some(reordered) = crate::join_order::reorder_joins_dp_bushy(operators) {
            return reordered;
        }

        // Only drop equality join-condition filters when the plan contains a
        // real join (CrossProduct/HashJoin) that consumes them as keys.
        // A plan WITHOUT a join (single-scan pipelines, WCOJ/Intersect plans)
        // must keep these filters, otherwise `a.id = b.id` silently passes
        // every row (P48.4 BUG-B).
        let has_join = operators
            .iter()
            .any(|op| matches!(op, LogicalOperator::CrossProduct(_) | LogicalOperator::HashJoin(_)));
        if !has_join {
            return operators.to_vec();
        }

        // Fallback: just remove filter conditions that are join conditions
        let mut result: Vec<LogicalOperator> = Vec::new();
        let mut filters_to_remove: Vec<usize> = Vec::new();

        for (i, op) in operators.iter().enumerate() {
            if let LogicalOperator::Filter(f) = op
                && is_join_condition(&f.expression)
            {
                filters_to_remove.push(i);
            }
        }

        for (i, op) in operators.iter().enumerate() {
            if filters_to_remove.contains(&i) {
                continue;
            }
            result.push(op.clone());
        }

        result
    }
}

/// Check if an expression is an equality join condition between two variables.
pub fn is_join_condition(expr: &akar_parser::ast::Expression) -> bool {
    match expr {
        akar_parser::ast::Expression::BinaryOp(akar_parser::ast::BinaryOp::Equal, left, right) => {
            let left_var = extract_root_variable(left);
            let right_var = extract_root_variable(right);
            left_var.is_some() && right_var.is_some() && left_var != right_var
        }
        _ => false,
    }
}

/// Extract the root variable from an expression (e.g., `a.id` → `a`).
pub fn extract_root_variable(expr: &akar_parser::ast::Expression) -> Option<String> {
    match expr {
        akar_parser::ast::Expression::Variable(name) => Some(name.clone()),
        akar_parser::ast::Expression::PropertyAccess(obj, _) => extract_root_variable(obj),
        _ => None,
    }
}
