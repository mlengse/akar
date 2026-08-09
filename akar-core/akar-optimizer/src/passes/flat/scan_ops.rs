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
// ========================================================================
//
// This pass is a NO-OP. The flat pipeline references columns positionally:
// every operator below a Projection (ORDER BY sort keys, LIMIT, subsequent
// Projections, aggregates) reads columns by index and the topmost Projection's
// arity IS the RETURN schema. Removing duplicate projection expressions changes
// the output column count (e.g. `RETURN a.name, a.name` → 1 column), but the
// `mapping` computed by the old implementation was never applied to any
// downstream consumer — every consumer then read the wrong column or ran out
// of bounds. True CSE needs a column-reference/alias layer that this flat
// architecture does not have, so every expression is kept untouched to
// guarantee correctness.

pub struct CommonSubexpressionElimination;

impl OptimizationPass for CommonSubexpressionElimination {
    fn name(&self) -> &str {
        "common_subexpression_elimination"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        operators.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_binder::bound_statement::BoundExpression;
    use akar_parser::ast::Expression;

    fn expr(s: &str) -> Expression {
        Expression::Variable(s.into())
    }

    fn make_projection(expressions: Vec<Expression>) -> LogicalOperator {
        LogicalOperator::Projection(LogicalProjection {
            expressions: expressions
                .into_iter()
                .map(|e| BoundExpression {
                    expression: e,
                    resolved_type: akar_common::types::LogicalTypeID::Any,
                    is_constant: false,
                })
                .collect(),
            children: vec![],
            cardinality: 0,
        })
    }

    #[test]
    fn test_cse_preserves_projection_arity() {
        let pass = CommonSubexpressionElimination;
        // `RETURN a.name, a.name` must stay a 2-column projection — deduping
        // to 1 column would change the RETURN schema and shift every positional
        // column reference below it.
        let plan = vec![make_projection(vec![expr("a.name"), expr("a.name")])];
        let result = pass.apply(&plan);
        if let LogicalOperator::Projection(p) = &result[0] {
            assert_eq!(p.expressions.len(), 2, "projection arity must be preserved");
        } else {
            panic!("expected Projection");
        }
    }

    #[test]
    fn test_cse_keeps_distinct_expressions() {
        let pass = CommonSubexpressionElimination;
        let plan = vec![make_projection(vec![expr("a.name"), expr("a.age")])];
        let result = pass.apply(&plan);
        if let LogicalOperator::Projection(p) = &result[0] {
            assert_eq!(p.expressions.len(), 2);
        } else {
            panic!("expected Projection");
        }
    }
}
