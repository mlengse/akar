//! Miscellaneous flat optimization passes: RemoveUnnecessaryOperators, LimitPushDown, CSE.

use crate::passes::OptimizationPass;
use akar_planner::logical_operator::*;

// ========================================================================
// Pass 7: Remove Unnecessary Operators
// ========================================================================

pub struct RemoveUnnecessaryOperators;

impl OptimizationPass for RemoveUnnecessaryOperators {
    fn name(&self) -> &str {
        "remove_unnecessary"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        operators
            .iter()
            .filter(|op| match op {
                LogicalOperator::ScanNode(s) => !s.table_name.is_empty(),
                LogicalOperator::Projection(p) => !p.expressions.is_empty(),
                LogicalOperator::Filter(f) => !is_tautology(&f.expression),
                _ => true,
            })
            .cloned()
            .collect()
    }
}

/// Check if a filter expression is a tautology (always true).
pub fn is_tautology(expr: &akar_parser::ast::Expression) -> bool {
    match expr {
        akar_parser::ast::Expression::Constant(akar_parser::ast::Constant::Bool(true)) => true,
        akar_parser::ast::Expression::BinaryOp(akar_parser::ast::BinaryOp::Equal, left, right) => {
            // `1 = 1` is a tautology
            match (&**left, &**right) {
                (
                    akar_parser::ast::Expression::Constant(akar_parser::ast::Constant::Integer(a)),
                    akar_parser::ast::Expression::Constant(akar_parser::ast::Constant::Integer(b)),
                ) => a == b,
                _ => false,
            }
        }
        _ => false,
    }
}

// ========================================================================
// Pass 10: Limit Push-Down
// Pushes Limit operators below Filter/Projection when safe.
// ========================================================================

pub struct LimitPushDown;

impl OptimizationPass for LimitPushDown {
    fn name(&self) -> &str {
        "limit_push_down"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result: Vec<LogicalOperator> = Vec::with_capacity(operators.len());
        let mut i = 0;
        while i < operators.len() {
            if i + 1 < operators.len() {
                if matches!(operators[i], LogicalOperator::Limit(_))
                    && matches!(operators[i + 1], LogicalOperator::Filter(_))
                {
                    // Swap: push Limit below Filter
                    result.push(operators[i + 1].clone()); // Filter first
                    result.push(operators[i].clone()); // then Limit
                    i += 2;
                    continue;
                }
                if matches!(operators[i], LogicalOperator::Limit(_))
                    && matches!(operators[i + 1], LogicalOperator::Projection(_))
                {
                    // Swap: push Limit below Projection (safe for simple projections)
                    result.push(operators[i + 1].clone());
                    result.push(operators[i].clone());
                    i += 2;
                    continue;
                }
            }
            result.push(operators[i].clone());
            i += 1;
        }
        result
    }
}

// ========================================================================
// Pass 11: Common Subexpression Elimination (CSE)
// Detects duplicate expressions in Projection and caches results.
// ========================================================================

pub struct CommonSubexpressionElimination;

impl OptimizationPass for CommonSubexpressionElimination {
    fn name(&self) -> &str {
        "common_subexpression_elimination"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        operators
            .iter()
            .map(|op| {
                match op {
                    LogicalOperator::Projection(p) => {
                        // Check for duplicate expressions
                        let mut seen_exprs: Vec<&akar_binder::bound_statement::BoundExpression> = Vec::new();
                        let mut unique_exprs: Vec<akar_binder::bound_statement::BoundExpression> = Vec::new();
                        let mut mapping: Vec<usize> = Vec::new();
                        for expr in &p.expressions {
                            if let Some(pos) = seen_exprs.iter().position(|e| e.expression == expr.expression) {
                                mapping.push(pos);
                            } else {
                                seen_exprs.push(expr);
                                unique_exprs.push(expr.clone());
                                mapping.push(unique_exprs.len() - 1);
                            }
                        }
                        // Only rewrite if dedup happened
                        if unique_exprs.len() < p.expressions.len() {
                            LogicalOperator::Projection(LogicalProjection {
                                expressions: unique_exprs,
                                children: p.children.clone(),
                                cardinality: p.cardinality,
                            })
                        } else {
                            op.clone()
                        }
                    }
                    _ => op.clone(),
                }
            })
            .collect()
    }
}
