// ========================================================================
// Pass 8: Constant Folding
// Pre-evaluates constant sub-expressions at optimization time.
// E.g., `1 + 2` → `3`, `TRUE AND FALSE` → `FALSE`, `'he' + 'llo'` → `'hello'`
// ========================================================================

use crate::passes::OptimizationPass;
use kuzu_binder::bound_statement::BoundExpression;
use kuzu_planner::logical_operator::*;

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
pub fn fold_expression(expr: &kuzu_parser::ast::Expression) -> kuzu_parser::ast::Expression {
    use kuzu_parser::ast::{BinaryOp, Constant, Expression, UnaryOp};

    match expr {
        // Binary operations on two constants
        Expression::BinaryOp(op, left, right) => {
            let left = fold_expression(left);
            let right = fold_expression(right);
            match (&left, &right) {
                (Expression::Constant(Constant::Integer(a)), Expression::Constant(Constant::Integer(b))) => {
                    let result = match op {
                        BinaryOp::Add => Some(Constant::Integer(a + b)),
                        BinaryOp::Subtract => Some(Constant::Integer(a - b)),
                        BinaryOp::Multiply => Some(Constant::Integer(a * b)),
                        BinaryOp::Divide if *b != 0 => Some(Constant::Integer(a / b)),
                        BinaryOp::Modulo if *b != 0 => Some(Constant::Integer(a % b)),
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
                    let result = match op {
                        BinaryOp::Add => Some(Constant::Float(a + b)),
                        BinaryOp::Subtract => Some(Constant::Float(a - b)),
                        BinaryOp::Multiply => Some(Constant::Float(a * b)),
                        BinaryOp::Divide if *b != 0.0 => Some(Constant::Float(a / b)),
                        BinaryOp::Equal => Some(Constant::Bool((a - b).abs() < f64::EPSILON)),
                        BinaryOp::NotEqual => Some(Constant::Bool((a - b).abs() >= f64::EPSILON)),
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
                    Expression::Constant(Constant::Integer(-n))
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
            use kuzu_parser::ast::{CaseAlternative, CaseExpr};
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
fn fold_query(query: &kuzu_parser::ast::Query) -> kuzu_parser::ast::Query {
    let clauses: Vec<kuzu_parser::ast::Clause> = query
        .clauses
        .iter()
        .map(|clause| match clause {
            kuzu_parser::ast::Clause::Where(w) => kuzu_parser::ast::Clause::Where(kuzu_parser::ast::WhereClause {
                expression: fold_expression(&w.expression),
            }),
            other => other.clone(),
        })
        .collect();
    kuzu_parser::ast::Query { clauses }
}
