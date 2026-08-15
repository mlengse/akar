use akar_binder::bound_statement::{
    BoundClause, BoundDeleteClause, BoundDeleteItem, BoundEdgePattern, BoundExpression, BoundForeachClause,
    BoundMatchClause, BoundPattern, BoundQuery, BoundReturnClause, BoundSetClause, BoundStatement,
    BoundUnwindClause, BoundWhereClause,
};
use akar_common::types::Value;
use std::collections::HashMap;

use super::utils::value_to_ast_constant;
use crate::prepared_statement::substitute_params;

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
                        let new_order_by = r
                            .order_by
                            .as_ref()
                            .map(|items| {
                                items
                                    .iter()
                                    .map(|item| {
                                        Ok(akar_binder::bound_statement::BoundOrderByItem {
                                            expression: substitute_in_bound_expr(&item.expression, params)?,
                                            ascending: item.ascending,
                                        })
                                    })
                                    .collect::<Result<Vec<_>, String>>()
                            })
                            .transpose()?;
                        BoundClause::BoundReturn(BoundReturnClause {
                            expressions: new_exprs?,
                            distinct: r.distinct,
                            order_by: new_order_by,
                            limit: r.limit,
                            skip: r.skip,
                        })
                    }
                    BoundClause::BoundWhere(w) => {
                        let new_expr = substitute_in_bound_expr(&w.expression, params)?;
                        BoundClause::BoundWhere(BoundWhereClause { expression: new_expr })
                    }
                    BoundClause::BoundMatch(m) => {
                        BoundClause::BoundMatch(substitute_in_match_clause(m, params)?)
                    }
                    BoundClause::BoundOptionalMatch(m) => {
                        BoundClause::BoundOptionalMatch(substitute_in_match_clause(m, params)?)
                    }
                    BoundClause::BoundCreate(m) => {
                        BoundClause::BoundCreate(substitute_in_match_clause(m, params)?)
                    }
                    BoundClause::BoundSet(s) => BoundClause::BoundSet(BoundSetClause {
                        items: substitute_in_set_items(&s.items, params)?,
                    }),
                    BoundClause::BoundDelete(d) => {
                        let items = d
                            .items
                            .iter()
                            .map(|item| {
                                Ok(BoundDeleteItem {
                                    expression: substitute_params(&item.expression, params)?,
                                    table_name: item.table_name.clone(),
                                    table_id: item.table_id,
                                    primary_key_column: item.primary_key_column.clone(),
                                    is_node: item.is_node,
                                })
                            })
                            .collect::<Result<Vec<_>, String>>()?;
                        BoundClause::BoundDelete(BoundDeleteClause {
                            detach: d.detach,
                            items,
                        })
                    }
                    BoundClause::BoundUnwind(u) => BoundClause::BoundUnwind(BoundUnwindClause {
                        expression: substitute_params(&u.expression, params)?,
                        variable: u.variable.clone(),
                    }),
                    BoundClause::BoundForeach(f) => {
                        let sub_statements = f
                            .sub_statements
                            .iter()
                            .map(|sub| substitute_params_in_statement(sub, params))
                            .collect::<Result<Vec<_>, String>>()?;
                        BoundClause::BoundForeach(BoundForeachClause {
                            variable: f.variable.clone(),
                            expression: substitute_params(&f.expression, params)?,
                            sub_statements,
                        })
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
        BoundStatement::BoundCreateDml(c) => {
            let patterns = c
                .patterns
                .iter()
                .map(|p| {
                    let node = p
                        .node
                        .as_ref()
                        .map(|n| -> Result<akar_binder::bound_statement::BoundNodeCreate, String> {
                            let properties = n
                                .properties
                                .iter()
                                .map(|(k, v)| Ok((k.clone(), substitute_params(v, params)?)))
                                .collect::<Result<Vec<_>, String>>()?;
                            Ok(akar_binder::bound_statement::BoundNodeCreate {
                                variable: n.variable.clone(),
                                table_name: n.table_name.clone(),
                                table_id: n.table_id,
                                properties,
                            })
                        })
                        .transpose()?;
                    let edge = p
                        .edge
                        .as_ref()
                        .map(|e| -> Result<akar_binder::bound_statement::BoundEdgeCreate, String> {
                            let properties = e
                                .properties
                                .iter()
                                .map(|(k, v)| Ok((k.clone(), substitute_params(v, params)?)))
                                .collect::<Result<Vec<_>, String>>()?;
                            Ok(akar_binder::bound_statement::BoundEdgeCreate {
                                variable: e.variable.clone(),
                                table_name: e.table_name.clone(),
                                table_id: e.table_id,
                                src_var: e.src_var.clone(),
                                dst_var: e.dst_var.clone(),
                                properties,
                            })
                        })
                        .transpose()?;
                    Ok(akar_binder::bound_statement::BoundCreatePattern { node, edge })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(BoundStatement::BoundCreateDml(
                akar_binder::bound_statement::BoundCreateDml { patterns },
            ))
        }
        BoundStatement::BoundMerge(m) => {
            let properties = m
                .properties
                .iter()
                .map(|(k, v)| Ok((k.clone(), substitute_params(v, params)?)))
                .collect::<Result<Vec<_>, String>>()?;
            let patterns = m
                .patterns
                .iter()
                .map(|p| {
                    let node = p
                        .node
                        .as_ref()
                        .map(|n| -> Result<akar_binder::bound_statement::BoundNodeCreate, String> {
                            let properties = n
                                .properties
                                .iter()
                                .map(|(k, v)| Ok((k.clone(), substitute_params(v, params)?)))
                                .collect::<Result<Vec<_>, String>>()?;
                            Ok(akar_binder::bound_statement::BoundNodeCreate {
                                variable: n.variable.clone(),
                                table_name: n.table_name.clone(),
                                table_id: n.table_id,
                                properties,
                            })
                        })
                        .transpose()?;
                    let edge = p
                        .edge
                        .as_ref()
                        .map(|e| -> Result<akar_binder::bound_statement::BoundEdgeCreate, String> {
                            let properties = e
                                .properties
                                .iter()
                                .map(|(k, v)| Ok((k.clone(), substitute_params(v, params)?)))
                                .collect::<Result<Vec<_>, String>>()?;
                            Ok(akar_binder::bound_statement::BoundEdgeCreate {
                                variable: e.variable.clone(),
                                table_name: e.table_name.clone(),
                                table_id: e.table_id,
                                src_var: e.src_var.clone(),
                                dst_var: e.dst_var.clone(),
                                properties,
                            })
                        })
                        .transpose()?;
                    Ok(akar_binder::bound_statement::BoundCreatePattern { node, edge })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let on_create = substitute_in_set_items(&m.on_create, params)?;
            let on_match = substitute_in_set_items(&m.on_match, params)?;
            Ok(BoundStatement::BoundMerge(akar_binder::bound_statement::BoundMerge {
                table_name: m.table_name.clone(),
                table_id: m.table_id,
                properties,
                patterns,
                on_create,
                on_match,
            }))
        }
        other => Ok(other.clone()),
    }
}

/// Substitute parameters inside a bound MATCH/OPTIONAL MATCH/CREATE clause's
/// pattern properties (node + edge).
fn substitute_in_match_clause(
    m: &BoundMatchClause,
    params: &HashMap<String, Value>,
) -> Result<BoundMatchClause, String> {
    let patterns = m
        .patterns
        .iter()
        .map(|p| {
            let properties = p
                .properties
                .iter()
                .map(|(k, v)| Ok((k.clone(), substitute_params(v, params)?)))
                .collect::<Result<Vec<_>, String>>()?;
            let edge = p
                .edge
                .as_ref()
                .map(|e| -> Result<BoundEdgePattern, String> {
                    let properties = e
                        .properties
                        .iter()
                        .map(|(k, v)| Ok((k.clone(), substitute_params(v, params)?)))
                        .collect::<Result<Vec<_>, String>>()?;
                    Ok(BoundEdgePattern {
                        variable: e.variable.clone(),
                        label: e.label.clone(),
                        rel_table_id: e.rel_table_id,
                        direction: e.direction.clone(),
                        properties,
                        lower_bound: e.lower_bound,
                        upper_bound: e.upper_bound,
                    })
                })
                .transpose()?;
            Ok(BoundPattern {
                node_variable: p.node_variable.clone(),
                node_label: p.node_label.clone(),
                node_table_id: p.node_table_id,
                properties,
                edge,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(BoundMatchClause {
        patterns,
        new_variables: m.new_variables.clone(),
        fts_query: m.fts_query.clone(),
    })
}

/// Substitute parameters inside a slice of bound SET items.
fn substitute_in_set_items(
    items: &[akar_binder::bound_statement::BoundSetItem],
    params: &HashMap<String, Value>,
) -> Result<Vec<akar_binder::bound_statement::BoundSetItem>, String> {
    items
        .iter()
        .map(|item| {
            Ok(akar_binder::bound_statement::BoundSetItem {
                property: substitute_params(&item.property, params)?,
                value: substitute_params(&item.value, params)?,
                column_name: item.column_name.clone(),
                column_idx: item.column_idx,
                table_name: item.table_name.clone(),
                table_id: item.table_id,
                is_node: item.is_node,
            })
        })
        .collect()
}

fn substitute_in_bound_expr(
    expr: &BoundExpression,
    params: &HashMap<String, Value>,
) -> Result<BoundExpression, String> {
    let new_expr = substitute_params(&expr.expression, params)?;
    Ok(BoundExpression {
        expression: new_expr,
        resolved_type: expr.resolved_type,
        is_constant: expr.is_constant,
        alias: expr.alias.clone(),
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
            let new_patterns: Vec<_> = c
                .patterns
                .iter()
                .map(|p| akar_binder::bound_statement::BoundCreatePattern {
                    node: p.node.as_ref().map(|n| {
                        akar_binder::bound_statement::BoundNodeCreate {
                            variable: n.variable.clone(),
                            table_name: n.table_name.clone(),
                            table_id: n.table_id,
                            properties: n
                                .properties
                                .iter()
                                .map(|(k, v)| (k.clone(), substitute_var_in_expr(v, var_name, val)))
                                .collect(),
                        }
                    }),
                    edge: p.edge.as_ref().map(|e| {
                        akar_binder::bound_statement::BoundEdgeCreate {
                            variable: e.variable.clone(),
                            table_name: e.table_name.clone(),
                            table_id: e.table_id,
                            src_var: e.src_var.clone(),
                            dst_var: e.dst_var.clone(),
                            properties: e
                                .properties
                                .iter()
                                .map(|(k, v)| (k.clone(), substitute_var_in_expr(v, var_name, val)))
                                .collect(),
                        }
                    }),
                })
                .collect();
            Ok(BoundStatement::BoundCreateDml(
                akar_binder::bound_statement::BoundCreateDml {
                    patterns: new_patterns,
                },
            ))
        }
        BoundStatement::BoundQuery(q) => {
            let mut new_clauses = Vec::new();
            for clause in &q.clauses {
                match clause {
                    akar_binder::bound_statement::BoundClause::BoundSet(s) => {
                        let new_items: Vec<_> = s
                            .items
                            .iter()
                            .map(|item| akar_binder::bound_statement::BoundSetItem {
                                property: substitute_var_in_expr(&item.property, var_name, val),
                                value: substitute_var_in_expr(&item.value, var_name, val),
                                column_name: item.column_name.clone(),
                                column_idx: item.column_idx,
                                table_name: item.table_name.clone(),
                                table_id: item.table_id,
                                is_node: item.is_node,
                            })
                            .collect();
                        new_clauses.push(akar_binder::bound_statement::BoundClause::BoundSet(
                            akar_binder::bound_statement::BoundSetClause { items: new_items },
                        ));
                    }
                    other => new_clauses.push(other.clone()),
                }
            }
            Ok(BoundStatement::BoundQuery(akar_binder::bound_statement::BoundQuery {
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
    expr: &akar_parser::ast::Expression,
    var_name: &str,
    val: &Value,
) -> akar_parser::ast::Expression {
    match expr {
        akar_parser::ast::Expression::Variable(name) if name == var_name => value_to_ast_constant(val),
        akar_parser::ast::Expression::BinaryOp(op, left, right) => akar_parser::ast::Expression::BinaryOp(
            *op,
            Box::new(substitute_var_in_expr(left, var_name, val)),
            Box::new(substitute_var_in_expr(right, var_name, val)),
        ),
        akar_parser::ast::Expression::UnaryOp(op, inner) => {
            akar_parser::ast::Expression::UnaryOp(*op, Box::new(substitute_var_in_expr(inner, var_name, val)))
        }
        akar_parser::ast::Expression::List(items) => {
            akar_parser::ast::Expression::List(items.iter().map(|i| substitute_var_in_expr(i, var_name, val)).collect())
        }
        akar_parser::ast::Expression::PropertyAccess(obj, prop) => akar_parser::ast::Expression::PropertyAccess(
            Box::new(substitute_var_in_expr(obj, var_name, val)),
            prop.clone(),
        ),
        other => other.clone(),
    }
}
