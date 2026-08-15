// ========================================================================
// Pass 8: Constant Folding
// Pre-evaluates constant sub-expressions at optimization time.
// E.g., `1 + 2` → `3`, `TRUE AND FALSE` → `FALSE`, `'he' + 'llo'` → `'hello'`
// ========================================================================

use crate::passes::OptimizationPass;
use akar_binder::bound_statement::BoundExpression;
use akar_planner::logical_operator::*;

pub struct ConstantFolding;

impl OptimizationPass for ConstantFolding {
    fn name(&self) -> &str {
        "constant_folding"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        operators
            .iter()
            .map(|op| match op {
                LogicalOperator::Filter(f) => {
                    let folded = fold_expression(&f.expression);
                    LogicalOperator::Filter(LogicalFilter {
                        expression: folded,
                        children: f.children.clone(),
                        cardinality: f.cardinality,
                    })
                }
                LogicalOperator::Projection(p) => {
                    let exprs: Vec<BoundExpression> = p
                        .expressions
                        .iter()
                        .map(|e| {
                            let folded = fold_expression(&e.expression);
                            BoundExpression {
                                expression: folded,
                                resolved_type: e.resolved_type,
                                is_constant: e.is_constant,
                                alias: e.alias.clone(),
                            }
                        })
                        .collect();
                    LogicalOperator::Projection(LogicalProjection {
                        expressions: exprs,
                        children: p.children.clone(),
                        cardinality: p.cardinality,
                    })
                }
                other => other.clone(),
            })
            .collect()
    }
}

/// Fold constant sub-expressions in an expression tree.
pub fn fold_expression(expr: &akar_parser::ast::Expression) -> akar_parser::ast::Expression {
    use akar_parser::ast::{BinaryOp, Constant, Expression, UnaryOp};

    match expr {
        // Binary operations on two constants
        Expression::BinaryOp(op, left, right) => {
            let left = fold_expression(left);
            let right = fold_expression(right);
            match (&left, &right) {
                (Expression::Constant(Constant::Integer(a)), Expression::Constant(Constant::Integer(b))) => {
                    // Checked arithmetic: folding must not panic (debug) or wrap
                    // (release) on i64 overflow — such expressions are left
                    // unfolded so the runtime evaluates them (P51.26).
                    let result = match op {
                        BinaryOp::Add => a.checked_add(*b).map(Constant::Integer),
                        BinaryOp::Subtract => a.checked_sub(*b).map(Constant::Integer),
                        BinaryOp::Multiply => a.checked_mul(*b).map(Constant::Integer),
                        BinaryOp::Divide if *b != 0 => a.checked_div(*b).map(Constant::Integer),
                        BinaryOp::Modulo if *b != 0 => a.checked_rem(*b).map(Constant::Integer),
                        BinaryOp::Equal => Some(Constant::Bool(a == b)),
                        BinaryOp::NotEqual => Some(Constant::Bool(a != b)),
                        BinaryOp::LessThan => Some(Constant::Bool(a < b)),
                        BinaryOp::LessThanOrEqual => Some(Constant::Bool(a <= b)),
                        BinaryOp::GreaterThan => Some(Constant::Bool(a > b)),
                        BinaryOp::GreaterThanOrEqual => Some(Constant::Bool(a >= b)),
                        _ => None,
                    };
                    if let Some(c) = result {
                        return Expression::Constant(c);
                    }
                }
                (Expression::Constant(Constant::Float(a)), Expression::Constant(Constant::Float(b))) => {
                    // Float equality uses exact comparison to match the runtime
                    // evaluator — an epsilon-tolerant fold would change results
                    // for near-equal constants (P51.25).
                    let result = match op {
                        BinaryOp::Add => Some(Constant::Float(a + b)),
                        BinaryOp::Subtract => Some(Constant::Float(a - b)),
                        BinaryOp::Multiply => Some(Constant::Float(a * b)),
                        BinaryOp::Divide if *b != 0.0 => Some(Constant::Float(a / b)),
                        BinaryOp::Equal => Some(Constant::Bool(a == b)),
                        BinaryOp::NotEqual => Some(Constant::Bool(a != b)),
                        BinaryOp::LessThan => Some(Constant::Bool(a < b)),
                        BinaryOp::LessThanOrEqual => Some(Constant::Bool(a <= b)),
                        BinaryOp::GreaterThan => Some(Constant::Bool(a > b)),
                        BinaryOp::GreaterThanOrEqual => Some(Constant::Bool(a >= b)),
                        _ => None,
                    };
                    if let Some(c) = result {
                        return Expression::Constant(c);
                    }
                }
                (Expression::Constant(Constant::Bool(a)), Expression::Constant(Constant::Bool(b))) => {
                    let result = match op {
                        BinaryOp::And => Some(Constant::Bool(*a && *b)),
                        BinaryOp::Or => Some(Constant::Bool(*a || *b)),
                        BinaryOp::Xor => Some(Constant::Bool(*a ^ *b)),
                        BinaryOp::Equal => Some(Constant::Bool(*a == *b)),
                        BinaryOp::NotEqual => Some(Constant::Bool(*a != *b)),
                        _ => None,
                    };
                    if let Some(c) = result {
                        return Expression::Constant(c);
                    }
                }
                (Expression::Constant(Constant::String(a)), Expression::Constant(Constant::String(b)))
                    if (*op == BinaryOp::Concat || *op == BinaryOp::Add) =>
                {
                    return Expression::Constant(Constant::String(format!("{}{}", a, b)));
                }
                _ => {}
            }
            Expression::BinaryOp(*op, Box::new(left), Box::new(right))
        }
        // Unary operations on constants
        Expression::UnaryOp(op, inner) => {
            let inner = fold_expression(inner);
            match (&inner, op) {
                (Expression::Constant(Constant::Integer(n)), UnaryOp::Negate) => {
                    // `-i64::MIN` overflows — leave it unfolded instead of
                    // panicking in debug or wrapping in release (P51.26).
                    match n.checked_neg() {
                        Some(v) => Expression::Constant(Constant::Integer(v)),
                        None => Expression::UnaryOp(*op, Box::new(inner)),
                    }
                }
                (Expression::Constant(Constant::Float(n)), UnaryOp::Negate) => {
                    Expression::Constant(Constant::Float(-n))
                }
                (Expression::Constant(Constant::Bool(b)), UnaryOp::Not) => Expression::Constant(Constant::Bool(!b)),
                _ => Expression::UnaryOp(*op, Box::new(inner)),
            }
        }
        // Recursively fold sub-expressions
        Expression::PropertyAccess(obj, prop) => {
            Expression::PropertyAccess(Box::new(fold_expression(obj)), prop.clone())
        }
        Expression::FunctionCall(name, args) => {
            let folded_args: Vec<Expression> = args.iter().map(fold_expression).collect();
            Expression::FunctionCall(name.clone(), folded_args)
        }
        Expression::List(items) => Expression::List(items.iter().map(fold_expression).collect()),
        Expression::Map(entries) => {
            Expression::Map(entries.iter().map(|(k, v)| (k.clone(), fold_expression(v))).collect())
        }
        // Leave these unchanged
        Expression::Variable(_) | Expression::Parameter(_) | Expression::Constant(_) => expr.clone(),
        Expression::ExistsSubquery(query) => Expression::ExistsSubquery(Box::new(fold_query(query))),
        Expression::Case(case_expr) => {
            use akar_parser::ast::{CaseAlternative, CaseExpr};
            let subject = case_expr.subject.as_ref().map(|s| Box::new(fold_expression(s)));
            let alternatives = case_expr
                .alternatives
                .iter()
                .map(|alt| CaseAlternative {
                    when: fold_expression(&alt.when),
                    then: fold_expression(&alt.then),
                })
                .collect();
            let else_expr = case_expr.else_expr.as_ref().map(|e| Box::new(fold_expression(e)));
            Expression::Case(CaseExpr {
                subject,
                alternatives,
                else_expr,
            })
        }
        Expression::Star => expr.clone(),
        Expression::ListPredicate {
            quantifier,
            list,
            var_name,
            predicate,
        } => Expression::ListPredicate {
            quantifier: *quantifier,
            list: Box::new(fold_expression(list)),
            var_name: var_name.clone(),
            predicate: Box::new(fold_expression(predicate)),
        },
        Expression::Lambda { var_name, body } => Expression::Lambda {
            var_name: var_name.clone(),
            body: Box::new(fold_expression(body)),
        },
    }
}

/// Fold constant sub-expressions in a Query's clauses.
fn fold_query(query: &akar_parser::ast::Query) -> akar_parser::ast::Query {
    let clauses: Vec<akar_parser::ast::Clause> = query
        .clauses
        .iter()
        .map(|clause| match clause {
            akar_parser::ast::Clause::Where(w) => akar_parser::ast::Clause::Where(akar_parser::ast::WhereClause {
                expression: fold_expression(&w.expression),
            }),
            other => other.clone(),
        })
        .collect();
    akar_parser::ast::Query { clauses }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_parser::ast::{BinaryOp, Constant, Expression, UnaryOp};

    #[test]
    fn test_float_equality_folds_exactly() {
        // P51.25: the fold must use exact `==` (matching the runtime
        // evaluator), not an epsilon tolerance that would change results for
        // near-equal constants.
        let near = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::Constant(Constant::Float(1.0))),
            Box::new(Expression::Constant(Constant::Float(1.0 + f64::EPSILON))),
        );
        assert_eq!(
            fold_expression(&near),
            Expression::Constant(Constant::Bool(false)),
            "near-equal floats must fold to false (exact comparison)"
        );

        let equal = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::Constant(Constant::Float(0.5))),
            Box::new(Expression::Constant(Constant::Float(0.5))),
        );
        assert_eq!(fold_expression(&equal), Expression::Constant(Constant::Bool(true)));
    }

    #[test]
    fn test_integer_overflow_not_folded() {
        // P51.26: i64::MAX + 1 must not panic (debug) or wrap (release) during
        // planning — the expression is left unfolded for the runtime.
        let expr = Expression::BinaryOp(
            BinaryOp::Add,
            Box::new(Expression::Constant(Constant::Integer(i64::MAX))),
            Box::new(Expression::Constant(Constant::Integer(1))),
        );
        let folded = fold_expression(&expr);
        assert!(
            matches!(folded, Expression::BinaryOp(BinaryOp::Add, _, _)),
            "overflow must not be folded"
        );
    }

    #[test]
    fn test_integer_negate_min_not_folded() {
        // P51.26: `-i64::MIN` overflows; the unary op must stay unfolded.
        let expr = Expression::UnaryOp(
            UnaryOp::Negate,
            Box::new(Expression::Constant(Constant::Integer(i64::MIN))),
        );
        let folded = fold_expression(&expr);
        assert!(matches!(folded, Expression::UnaryOp(UnaryOp::Negate, _)));
    }
}
