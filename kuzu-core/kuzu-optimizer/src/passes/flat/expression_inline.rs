use crate::passes::OptimizationPass;
use kuzu_binder::bound_statement::BoundExpression;
use kuzu_parser::ast::Expression;
use kuzu_planner::logical_operator::*;

pub struct ExpressionInline;

impl OptimizationPass for ExpressionInline {
    fn name(&self) -> &str {
        "expression_inline"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result = Vec::with_capacity(operators.len());
        let mut i = 0;
        while i < operators.len() {
            if i + 1 < operators.len() {
                if let (LogicalOperator::Projection(outer), LogicalOperator::Projection(inner)) =
                    (&operators[i], &operators[i + 1])
                {
                    if can_inline_projections(outer, inner) {
                        result.push(inline_projections(outer, inner));
                        i += 2;
                        continue;
                    }
                }
                if let (LogicalOperator::Filter(filter), LogicalOperator::Projection(proj)) =
                    (&operators[i], &operators[i + 1])
                {
                    if let Some(inlined) = inline_filter_through_projection(filter, proj) {
                        result.push(inlined);
                        i += 2;
                        continue;
                    }
                }
            }
            result.push(operators[i].clone());
            i += 1;
        }
        result
    }
}

fn can_inline_projections(outer: &LogicalProjection, inner: &LogicalProjection) -> bool {
    if outer.expressions.len() != 1 {
        return false;
    }
    if let Expression::Variable(name) = &outer.expressions[0].expression {
        inner.expressions.iter().any(|be| {
            matches!(&be.expression, Expression::Variable(n) if n == name)
        })
    } else {
        false
    }
}

fn inline_projections(outer: &LogicalProjection, inner: &LogicalProjection) -> LogicalOperator {
    LogicalOperator::Projection(LogicalProjection {
        expressions: inner.expressions.clone(),
        children: inner.children.clone(),
        cardinality: outer.cardinality,
    })
}

fn inline_filter_through_projection(
    filter: &LogicalFilter,
    proj: &LogicalProjection,
) -> Option<LogicalOperator> {
    let inlined = inline_expression(&filter.expression, &proj.expressions);
    if inlined == filter.expression {
        return None;
    }
    Some(LogicalOperator::Filter(LogicalFilter {
        expression: inlined,
        children: proj.children.clone(),
        cardinality: filter.cardinality,
    }))
}

fn inline_expression(expr: &Expression, proj_exprs: &[BoundExpression]) -> Expression {
    match expr {
        Expression::Variable(name) => {
            for be in proj_exprs {
                if let Expression::Variable(n) = &be.expression {
                    if n == name {
                        return be.expression.clone();
                    }
                }
            }
            expr.clone()
        }
        Expression::BinaryOp(op, left, right) => {
            let new_left = inline_expression(left, proj_exprs);
            let new_right = inline_expression(right, proj_exprs);
            Expression::BinaryOp(*op, Box::new(new_left), Box::new(new_right))
        }
        Expression::UnaryOp(op, inner) => {
            Expression::UnaryOp(*op, Box::new(inline_expression(inner, proj_exprs)))
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_var(name: &str) -> Expression {
        Expression::Variable(name.into())
    }

    fn make_proj(exprs: Vec<Expression>, children: Vec<LogicalOperator>) -> LogicalOperator {
        let bound: Vec<BoundExpression> = exprs
            .into_iter()
            .map(|e| BoundExpression {
                expression: e,
                resolved_type: kuzu_common::types::LogicalTypeID::Int64,
                is_constant: false,
            })
            .collect();
        LogicalOperator::Projection(LogicalProjection {
            expressions: bound,
            children,
            cardinality: 100,
        })
    }

    fn make_scan(name: &str) -> LogicalOperator {
        LogicalOperator::ScanNode(LogicalScanNode {
            table_name: name.into(),
            table_id: 0,
            alias: None,
            columns: vec![],
            cardinality: 10,
            fts_query: None,
            predicate: None,
        })
    }

    #[test]
    fn test_inline_identity_projection() {
        let pass = ExpressionInline;
        let scan = make_scan("t");
        let inner = make_proj(vec![make_var("x"), make_var("y")], vec![scan]);
        let outer = make_proj(vec![make_var("x")], vec![]);
        let result = pass.apply(&[outer, inner]);
        assert_eq!(result.len(), 1);
        if let LogicalOperator::Projection(p) = &result[0] {
            assert_eq!(p.expressions.len(), 2);
        } else {
            panic!("Expected Projection");
        }
    }

    #[test]
    fn test_no_inline_outer_multi_expression() {
        let pass = ExpressionInline;
        let scan = make_scan("t");
        let inner = make_proj(vec![make_var("x"), make_var("y")], vec![scan]);
        let outer = make_proj(vec![make_var("x"), make_var("z")], vec![]);
        let result = pass.apply(&[outer, inner]);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_no_inline_non_variable() {
        let pass = ExpressionInline;
        let scan = make_scan("t");
        let inner = make_proj(vec![make_var("x")], vec![scan.clone()]);
        let outer = make_proj(
            vec![Expression::BinaryOp(
                kuzu_parser::ast::BinaryOp::Add,
                Box::new(make_var("x")),
                Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(1))),
            )],
            vec![inner],
        );
        let result = pass.apply(&[outer]);
        assert_eq!(result.len(), 1);
    }
}
