// ========================================================================
// Pass 5: Aggregate Detection
// Scans Projection operators for aggregate function calls (COUNT, SUM, AVG,
// MIN, MAX, ...) and replaces them with Aggregate operators. This is necessary
// because aggregates must process ALL rows (not per-row like projections).
//
// P53.15 (G4): aggregates nested inside arbitrary expressions (e.g.
// `COALESCE(MAX(x), 0)`, `COALESCE(MAX(s.id), 0) + 1`) are now detected
// recursively. Each aggregate call is extracted into the Aggregate operator and
// replaced in the projection by a positional/field-name reference to its output
// column; a Projection is kept above the Aggregate to evaluate the outer
// expression (`COALESCE(<agg>, 0)`) over the single-row aggregate result.
//
// P53.16 (G5): the Projection is ALWAYS kept above the Aggregate (not only for
// nested aggregates) so that `AS` aliases from RETURN/WITH are honored — the
// projection is the single place that applies alias-aware output column names.
//
// Example plan transformation:
//   [Scan, Projection([COALESCE(MAX(x), 0)])]
//   → [Scan, Aggregate(gb=[], aggs=[MAX(x)]), Projection([COALESCE(<MAX(x)>, 0)])]
// ========================================================================

use crate::passes::OptimizationPass;
use akar_binder::bound_statement::BoundExpression;
use akar_parser::ast::Expression;
use akar_planner::logical_operator::*;

/// Detect aggregate function calls in projections and replace with Aggregate.
pub struct AggregateDetection;

impl OptimizationPass for AggregateDetection {
    fn name(&self) -> &str {
        "aggregate_detection"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result = Vec::with_capacity(operators.len());

        for op in operators {
            match op {
                LogicalOperator::Projection(proj) => {
                // Check if any expression contains an aggregate function call
                // anywhere in its tree (top-level or nested).
                let mut all_aggregates: Vec<(String, Vec<Expression>)> = Vec::new();
                let mut group_by: Vec<Expression> = Vec::new();
                let mut rewritten: Vec<BoundExpression> = Vec::new();
                let mut has_agg = false;

                for be in &proj.expressions {
                        if !contains_aggregate(&be.expression) {
                            // Non-aggregate expressions become GROUP BY keys.
                            // e.g. RETURN p.active, COUNT(p), AVG(p.score)
                            //   → group_by=[p.active], aggregates=[COUNT(p), AVG(p.score)]
                            group_by.push(be.expression.clone());
                            rewritten.push(be.clone());
                            continue;
                        }

                        has_agg = true;
                        match extract_aggregate_function(&be.expression) {
                            // Whole expression is a single aggregate call at the
                            // top level (RETURN COUNT(*), SUM(x), ...).
                            Some((name, args)) => {
                                let ref_name = aggregate_ref_name(&name, &args);
                                all_aggregates.push((name, args));
                                // Refer to the aggregate output column by its
                                // field name (matches aggregate_field_names);
                                // keep the RETURN alias (P53.16).
                                rewritten.push(BoundExpression {
                                    expression: Expression::Variable(ref_name),
                                    resolved_type: be.resolved_type,
                                    is_constant: false,
                                    alias: be.alias.clone(),
                                });
                            }
                            // The aggregate is nested inside another expression
                            // (e.g. COALESCE(MAX(x), 0)) — extract it and keep
                            // the outer expression above the Aggregate.
                            None => {
                                let mut collected: Vec<(String, Vec<Expression>)> = Vec::new();
                                let new_expr = rewrite_aggregates(&be.expression, &mut collected);
                                all_aggregates.extend(collected);
                                rewritten.push(BoundExpression {
                                    expression: new_expr,
                                    resolved_type: be.resolved_type,
                                    is_constant: be.is_constant,
                                    alias: be.alias.clone(),
                                });
                            }
                        }
                    }

                    if !has_agg {
                        // No aggregates — keep as projection
                        result.push(op.clone());
                        continue;
                    }

                    let agg_op = LogicalOperator::Aggregate(LogicalAggregate {
                        group_by,
                        aggregates: all_aggregates,
                        children: proj.children.clone(),
                        cardinality: proj.cardinality,
                    });

                    // Always keep the (rewritten) projection above the Aggregate
                    // so `AS` aliases are applied to the output column names and
                    // outer expressions evaluate over the collapsed rows.
                    result.push(agg_op);
                    result.push(LogicalOperator::Projection(LogicalProjection {
                        expressions: rewritten,
                        children: Vec::new(),
                        cardinality: 0,
                    }));
                }
                _ => {
                    result.push(op.clone());
                }
            }
        }

        result
    }
}

/// Whether `expr` contains an aggregate function call anywhere in its tree.
fn contains_aggregate(expr: &Expression) -> bool {
    match expr {
        Expression::FunctionCall(name, args) => {
            if is_aggregate_name(&name.to_uppercase()) {
                true
            } else {
                args.iter().any(contains_aggregate)
            }
        }
        Expression::PropertyAccess(obj, _) => contains_aggregate(obj),
        Expression::BinaryOp(_, left, right) => contains_aggregate(left) || contains_aggregate(right),
        Expression::UnaryOp(_, inner) => contains_aggregate(inner),
        Expression::List(items) => items.iter().any(contains_aggregate),
        Expression::Map(items) => items.iter().any(|(_, v)| contains_aggregate(v)),
        Expression::Case(c) => {
            c.subject.as_ref().is_some_and(|s| contains_aggregate(s))
                || c.alternatives
                    .iter()
                    .any(|alt| contains_aggregate(&alt.when) || contains_aggregate(&alt.then))
                || c.else_expr.as_ref().is_some_and(|e| contains_aggregate(e))
        }
        Expression::ListPredicate {
            list, predicate, ..
        } => contains_aggregate(list) || contains_aggregate(predicate),
        // Variables, constants, parameters, STAR, subqueries and lambdas cannot
        // carry a liftable aggregate call.
        _ => false,
    }
}

/// Extract a top-level aggregate function from an expression, returning (name, args)
/// if the WHOLE expression is a single aggregate function call.
fn extract_aggregate_function(expr: &Expression) -> Option<(String, Vec<Expression>)> {
    match expr {
        Expression::FunctionCall(name, args) => {
            let upper = name.to_uppercase();
            if is_aggregate_name(&upper) {
                Some((upper, args.clone()))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Is `name` (already uppercased) a known aggregate function?
fn is_aggregate_name(name: &str) -> bool {
    matches!(
        name,
        "COUNT" | "SUM" | "AVG" | "MIN" | "MAX" | "STDDEV" | "VARIANCE" | "COLLECT"
    )
}

/// Rewrite `expr`, extracting every nested aggregate call into `out` and
/// replacing it with a `Variable` reference to the aggregate output column.
fn rewrite_aggregates(expr: &Expression, out: &mut Vec<(String, Vec<Expression>)>) -> Expression {
    match expr {
        Expression::FunctionCall(name, args) => {
            let upper = name.to_uppercase();
            if is_aggregate_name(&upper) {
                // Aggregate args are plain expressions (nested aggregates are
                // not valid Cypher) — no further rewriting needed.
                let ref_name = aggregate_ref_name(&upper, args);
                out.push((upper, args.clone()));
                return Expression::Variable(ref_name);
            }
            let new_args: Vec<Expression> = args.iter().map(|a| rewrite_aggregates(a, out)).collect();
            Expression::FunctionCall(name.clone(), new_args)
        }
        Expression::PropertyAccess(obj, prop) => {
            Expression::PropertyAccess(Box::new(rewrite_aggregates(obj, out)), prop.clone())
        }
        Expression::BinaryOp(op, left, right) => Expression::BinaryOp(
            op.clone(),
            Box::new(rewrite_aggregates(left, out)),
            Box::new(rewrite_aggregates(right, out)),
        ),
        Expression::UnaryOp(op, inner) => Expression::UnaryOp(op.clone(), Box::new(rewrite_aggregates(inner, out))),
        Expression::List(items) => {
            let new_items: Vec<Expression> = items.iter().map(|i| rewrite_aggregates(i, out)).collect();
            Expression::List(new_items)
        }
        Expression::Map(items) => {
            let new_items: Vec<(String, Expression)> = items
                .iter()
                .map(|(k, v)| (k.clone(), rewrite_aggregates(v, out)))
                .collect();
            Expression::Map(new_items)
        }
        Expression::Case(c) => {
            let mut new_c = c.clone();
            new_c.subject = c.subject.as_ref().map(|s| Box::new(rewrite_aggregates(s, out)));
            new_c.alternatives = c
                .alternatives
                .iter()
                .map(|alt| akar_parser::ast::CaseAlternative {
                    when: rewrite_aggregates(&alt.when, out),
                    then: rewrite_aggregates(&alt.then, out),
                })
                .collect();
            new_c.else_expr = c.else_expr.as_ref().map(|e| Box::new(rewrite_aggregates(e, out)));
            Expression::Case(new_c)
        }
        Expression::ListPredicate {
            quantifier,
            list,
            var_name,
            predicate,
        } => Expression::ListPredicate {
            quantifier: quantifier.clone(),
            list: Box::new(rewrite_aggregates(list, out)),
            var_name: var_name.clone(),
            predicate: Box::new(rewrite_aggregates(predicate, out)),
        },
        other => other.clone(),
    }
}

/// The field name of an aggregate's output column. MUST stay in sync with
/// `aggregate_field_names` in akar-processor `map_aggregate.rs` — the rewritten
/// projection resolves the aggregate result by this name.
fn aggregate_ref_name(name: &str, args: &[Expression]) -> String {
    if name == "COUNT" && args.iter().any(|e| matches!(e, Expression::Star)) {
        "COUNT(*)".to_string()
    } else if args.len() == 1 {
        match &args[0] {
            Expression::Variable(v) => format!("{name}({v})"),
            Expression::PropertyAccess(obj, prop) => {
                if let Expression::Variable(base) = &**obj {
                    format!("{name}({base}.{prop})")
                } else {
                    format!("{name}({prop})")
                }
            }
            Expression::Star => format!("{name}(*)"),
            _ => name.to_string(),
        }
    } else {
        name.to_string()
    }
}
