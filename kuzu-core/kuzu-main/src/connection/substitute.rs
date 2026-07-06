use kuzu_binder::bound_statement::{
    BoundClause, BoundExpression, BoundQuery, BoundReturnClause, BoundStatement, BoundWhereClause,
};
use kuzu_common::types::Value;
use std::collections::HashMap;

use super::utils::value_to_ast_constant;

/// Substitute parameter references in a BoundStatement with concrete values.
pub(crate) fn substitute_params_in_statement(
    bound: &BoundStatement,
    params: &HashMap<String, Value>,
) -> Result<BoundStatement, String> {
    match bound {
        BoundStatement::BoundQuery(q) => {
            let mut new_clauses = Vec::new();
            for clause in &q.clauses {
                let new_clause = match clause {
                    BoundClause::BoundReturn(r) => {
                        let new_exprs: Result<Vec<_>, _> = r
                            .expressions
                            .iter()
                            .map(|e| substitute_in_bound_expr(e, params))
                            .collect();
                        BoundClause::BoundReturn(BoundReturnClause {
                            expressions: new_exprs?,
                        })
                    }
                    BoundClause::BoundWhere(w) => {
                        let new_expr = substitute_in_bound_expr(&w.expression, params)?;
                        BoundClause::BoundWhere(BoundWhereClause { expression: new_expr })
                    }
                    other => other.clone(),
                };
                new_clauses.push(new_clause);
            }
            Ok(BoundStatement::BoundQuery(BoundQuery {
                variables: q.variables.clone(),
                clauses: new_clauses,
            }))
        }
        other => Ok(other.clone()),
    }
}

fn substitute_in_bound_expr(
    expr: &BoundExpression,
    params: &HashMap<String, Value>,
) -> Result<BoundExpression, String> {
    let new_expr = crate::prepared_statement::substitute_params(&expr.expression, params)?;
    Ok(BoundExpression {
        expression: new_expr,
        resolved_type: expr.resolved_type,
        is_constant: expr.is_constant,
    })
}

/// Substitute a FOREACH loop variable with a concrete value in a BoundStatement.
pub(crate) fn substitute_foreach_var(
    bound: &BoundStatement,
    var_name: &str,
    val: &Value,
) -> Result<BoundStatement, String> {
    match bound {
        BoundStatement::BoundCreateDml(c) => {
            let new_props: Vec<(String, kuzu_parser::ast::Expression)> = c
                .properties
                .iter()
                .map(|(k, v)| {
                    let new_v = substitute_var_in_expr(v, var_name, val);
                    (k.clone(), new_v)
                })
                .collect();
            Ok(BoundStatement::BoundCreateDml(
                kuzu_binder::bound_statement::BoundCreateDml {
                    table_name: c.table_name.clone(),
                    table_id: c.table_id,
                    properties: new_props,
                },
            ))
        }
        BoundStatement::BoundQuery(q) => {
            let mut new_clauses = Vec::new();
            for clause in &q.clauses {
                match clause {
                    kuzu_binder::bound_statement::BoundClause::BoundSet(s) => {
                        let new_items: Vec<_> = s
                            .items
                            .iter()
                            .map(|item| kuzu_binder::bound_statement::BoundSetItem {
                                property: substitute_var_in_expr(&item.property, var_name, val),
                                value: substitute_var_in_expr(&item.value, var_name, val),
                                column_name: item.column_name.clone(),
                                column_idx: item.column_idx,
                                table_name: item.table_name.clone(),
                                table_id: item.table_id,
                                is_node: item.is_node,
                            })
                            .collect();
                        new_clauses.push(kuzu_binder::bound_statement::BoundClause::BoundSet(
                            kuzu_binder::bound_statement::BoundSetClause { items: new_items },
                        ));
                    }
                    other => new_clauses.push(other.clone()),
                }
            }
            Ok(BoundStatement::BoundQuery(kuzu_binder::bound_statement::BoundQuery {
                clauses: new_clauses,
                variables: q.variables.clone(),
            }))
        }
        // For other statement types, pass through unchanged
        _ => Ok(bound.clone()),
    }
}

/// Substitute a variable reference with a constant Value in an AST expression.
pub(crate) fn substitute_var_in_expr(
    expr: &kuzu_parser::ast::Expression,
    var_name: &str,
    val: &Value,
) -> kuzu_parser::ast::Expression {
    match expr {
        kuzu_parser::ast::Expression::Variable(name) if name == var_name => value_to_ast_constant(val),
        kuzu_parser::ast::Expression::BinaryOp(op, left, right) => kuzu_parser::ast::Expression::BinaryOp(
            *op,
            Box::new(substitute_var_in_expr(left, var_name, val)),
            Box::new(substitute_var_in_expr(right, var_name, val)),
        ),
        kuzu_parser::ast::Expression::UnaryOp(op, inner) => {
            kuzu_parser::ast::Expression::UnaryOp(*op, Box::new(substitute_var_in_expr(inner, var_name, val)))
        }
        kuzu_parser::ast::Expression::List(items) => {
            kuzu_parser::ast::Expression::List(items.iter().map(|i| substitute_var_in_expr(i, var_name, val)).collect())
        }
        kuzu_parser::ast::Expression::PropertyAccess(obj, prop) => kuzu_parser::ast::Expression::PropertyAccess(
            Box::new(substitute_var_in_expr(obj, var_name, val)),
            prop.clone(),
        ),
        other => other.clone(),
    }
}
