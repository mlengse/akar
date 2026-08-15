use super::ExecutionContext;
use crate::expression_evaluator::ExpressionEvaluator;
use crate::physical_operator::*;
use crate::processor::projection_helper::resolve_projection_column_index;
use akar_common::error::ProcessorError;
use akar_common::vector::DataChunk;
use akar_parser::ast::Expression;
use akar_planner::logical_operator::LogicalOperator;
use std::sync::{Arc, Mutex};

fn projection_needs_expression_eval(expr: &Expression) -> bool {
    matches!(
        expr,
        Expression::FunctionCall(_, _)
            | Expression::Constant(_)
            | Expression::BinaryOp(_, _, _)
            | Expression::UnaryOp(_, _)
            | Expression::List(_)
            | Expression::Map(_)
            | Expression::Parameter(_)
            | Expression::ExistsSubquery(_)
            | Expression::ListPredicate { .. }
    )
}

/// Derive the output column name of a projected expression, so downstream
/// operators (ORDER BY / TOP-K) can resolve sort keys by name rather than by
/// position (P52.1). An `AS alias` overrides the derived name (P53.16, G5).
fn expression_field_name(alias: Option<&str>, expr: &Expression) -> String {
    if let Some(a) = alias {
        return a.to_string();
    }
    match expr {
        Expression::PropertyAccess(obj, prop) => {
            if let Expression::Variable(var) = &**obj {
                format!("{var}.{prop}")
            } else {
                prop.clone()
            }
        }
        Expression::Variable(name) => name.clone(),
        _ => String::new(),
    }
}

/// Resolve ORDER BY / TOP-K sort keys to column indices.
///
/// Sort keys are expressions (e.g. `p.age`); they must be mapped to the actual
/// output column they refer to. Positional mapping sorts by the i-th key's
/// position, which is wrong whenever ORDER BY references a non-first column
/// (e.g. `RETURN p.name, p.age ORDER BY p.age` sorted by name).
///
/// Sort keys that are computed expressions (`array_cosine_similarity(...)`,
/// `a + b`, ...) cannot be mapped to a column: `resolve_projection_column_index`
/// only handles PropertyAccess/Variable. They are evaluated per row via the
/// `ExpressionEvaluator` and appended to a copy of the input as synthetic
/// trailing columns (P53.23). The returned `Vec<DataChunk>` is the (possibly
/// augmented) input to sort on, and `usize` is the number of appended columns
/// that must be stripped from the operator output.
fn resolve_sort_keys(
    sort_keys: &[(Expression, bool)],
    input: &[DataChunk],
    ctx: &mut ExecutionContext,
) -> Result<(Vec<(u32, bool)>, Vec<DataChunk>, usize), ProcessorError> {
    let base_cols = input.first().map(|c| c.num_fields()).unwrap_or(0);
    let mut resolved = Vec::with_capacity(sort_keys.len());
    let mut computed: Vec<Expression> = Vec::new();
    for (expr, asc) in sort_keys.iter() {
        let col = input.first().and_then(|c| resolve_projection_column_index(expr, c));
        match col {
            Some(idx) => resolved.push((idx as u32, *asc)),
            None => {
                resolved.push(((base_cols + computed.len()) as u32, *asc));
                computed.push(expr.clone());
            }
        }
    }
    if computed.is_empty() {
        return Ok((resolved, input.to_vec(), 0));
    }
    let registry = ctx
        .function_registry
        .clone()
        .ok_or_else(|| "No function registry available for computed ORDER BY key".to_string())?;
    let mut eval = ExpressionEvaluator::new(registry);
    if let Some(ref seq_fn) = ctx.sequence_fn {
        eval = eval.with_sequence_fn(seq_fn.clone());
    }
    if let Some(ref subquery_fn) = ctx.subquery_fn {
        eval = eval.with_subquery_fn(subquery_fn.clone());
    }
    let mut augmented = Vec::with_capacity(input.len());
    for chunk in input {
        let mut fields = chunk.fields.clone();
        let mut field_types = chunk.field_types.clone();
        for expr in &computed {
            let vv = eval.evaluate_arrow(expr, chunk)?;
            fields.push(vv.array);
            field_types.push(vv.physical_type);
        }
        augmented.push(DataChunk {
            fields,
            field_types,
            size: chunk.size,
            field_names: chunk.field_names.clone(),
            sel_vector: None,
        });
    }
    Ok((resolved, augmented, computed.len()))
}

/// Drop the synthetic computed sort-key columns appended by `resolve_sort_keys`
/// (P53.23) from the operator output chunks.
fn strip_sort_columns(chunks: &mut [DataChunk], extra: usize) {
    if extra == 0 {
        return;
    }
    for chunk in chunks {
        let keep = chunk.fields.len().saturating_sub(extra);
        chunk.fields.truncate(keep);
        chunk.field_types.truncate(keep);
        chunk.field_names.truncate(keep);
    }
}

pub fn map_and_execute_projection(
    op: &LogicalOperator,
    current_input: Vec<DataChunk>,
    ctx: &mut ExecutionContext,
) -> Result<Vec<DataChunk>, ProcessorError> {
    match op {
        LogicalOperator::Projection(p) => {
            let input = if p.children.is_empty() {
                current_input
            } else {
                ctx.execute_children(&p.children)?
            };

            let result = if p.expressions.is_empty() {
                input
            } else {
                let needs_eval = p
                    .expressions
                    .iter()
                    .any(|be| projection_needs_expression_eval(&be.expression));

                if needs_eval {
                    let registry = ctx
                        .function_registry
                        .clone()
                        .ok_or_else(|| "No function registry available for expression projection".to_string())?;

                    let mut eval = ExpressionEvaluator::new(registry);
                    if let Some(ref seq_fn) = ctx.sequence_fn {
                        eval = eval.with_sequence_fn(seq_fn.clone());
                    }
                    if let Some(ref subquery_fn) = ctx.subquery_fn {
                        eval = eval.with_subquery_fn(subquery_fn.clone());
                    }

                    let mut output = Vec::with_capacity(input.len());
                    for chunk in input {
                        let mut fields = Vec::with_capacity(p.expressions.len());
                        let mut field_types = Vec::with_capacity(p.expressions.len());
                        for be in &p.expressions {
                            // Use the Arrow-native evaluator so complex-typed
                            // results (List/Struct columns and literals) round-trip
                            // without a ValueVector (which has no side-storage).
                            let result_vec = eval.evaluate_arrow(&be.expression, &chunk)?;
                            let pt = result_vec.physical_type;
                            let arr = result_vec.array.clone();
                            fields.push(arr);
                            field_types.push(pt);
                        }
                        let size = fields.first().map(|f| f.len()).unwrap_or(chunk.size);
                        let field_names = p
                            .expressions
                            .iter()
                            .map(|be| expression_field_name(be.alias.as_deref(), &be.expression))
                            .collect();
                        output.push(DataChunk {
                            fields,
                            field_types,
                            size,
                            field_names,
                            sel_vector: None,
                        });
                    }
                    output
                } else {
                    let column_indices: Vec<usize> = if let Some(first_chunk) = input.first() {
                        p.expressions
                            .iter()
                            .filter_map(|be| resolve_projection_column_index(&be.expression, first_chunk))
                            .collect()
                    } else {
                        Vec::new()
                    };
                    let column_indices = if column_indices.len() == p.expressions.len() {
                        column_indices
                    } else {
                        (0..p.expressions.len()).collect()
                    };
                    let rename = column_indices.len() == p.expressions.len() && !p.expressions.is_empty();
                    let proj = PhysicalProjection { column_indices };
                    let mut result = proj.execute(input)?;
                    // Rename output columns to alias-aware names (P53.16): the
                    // plain-column path copies input field_names, so `RETURN
                    // m.name AS nm` would otherwise keep `m.name` as the label.
                    if rename {
                        let names: Vec<String> = p
                            .expressions
                            .iter()
                            .map(|be| expression_field_name(be.alias.as_deref(), &be.expression))
                            .collect();
                        for chunk in &mut result {
                            if chunk.fields.len() == names.len() {
                                chunk.field_names = names.clone();
                            }
                        }
                    }
                    result
                }
            };
            Ok(result)
        }
        LogicalOperator::Filter(f) => {
            let evaluator = ctx.function_registry.clone().map(|reg| {
                let mut eval = ExpressionEvaluator::new(reg);
                if let Some(ref seq_fn) = ctx.sequence_fn {
                    eval = eval.with_sequence_fn(seq_fn.clone());
                }
                if let Some(ref subquery_fn) = ctx.subquery_fn {
                    eval = eval.with_subquery_fn(subquery_fn.clone());
                }
                Arc::new(Mutex::new(eval))
            });
            let filter = if let Some(eval) = evaluator {
                PhysicalFilter::with_evaluator(f.expression.clone(), eval)
            } else {
                PhysicalFilter::new(f.expression.clone())
            };
            let result = filter.execute(current_input)?;
            Ok(result)
        }
        LogicalOperator::Limit(l) => {
            let limit = PhysicalLimit {
                limit: l.limit,
                offset: l.offset,
            };
            let result = limit.execute(current_input)?;
            Ok(result)
        }
        LogicalOperator::TopK(tk) => {
            let (sort_keys, augmented, extra) = resolve_sort_keys(&tk.sort_keys, &current_input, ctx)?;
            let topk = PhysicalTopK {
                sort_keys,
                limit: tk.limit,
                offset: tk.offset,
            };
            let mut result = topk.execute(augmented)?;
            strip_sort_columns(&mut result, extra);
            Ok(result)
        }
        LogicalOperator::OrderBy(o) => {
            let (sort_keys, augmented, extra) = resolve_sort_keys(&o.sort_keys, &current_input, ctx)?;
            let order = PhysicalOrderBy { sort_keys };
            let mut result = order.execute(augmented)?;
            strip_sort_columns(&mut result, extra);
            Ok(result)
        }
        LogicalOperator::Flatten(f) => {
            let input = if f.children.is_empty() {
                current_input
            } else {
                ctx.execute_children(&f.children)?
            };
            let flatten = PhysicalFlatten::new(f.group_pos);
            flatten.execute(input)
        }
        LogicalOperator::Unwind(uw) => {
            let unwind = PhysicalUnwind {
                expression: uw.expression.clone(),
                variable: uw.variable.clone(),
            };
            let result = unwind.execute(current_input)?;
            Ok(result)
        }
        LogicalOperator::Partitioner(p) => {
            const MORSEL_SIZE: usize = 1024;
            let partitioner = Partitioner::new(MORSEL_SIZE);
            let morsels = partitioner.execute(current_input)?;

            let mut results = Vec::new();
            for _morsel in morsels {
                let child_result = ctx.execute_children(&p.children)?;
                results.extend(child_result);
            }
            Ok(results)
        }
        _ => Err(format!("Not a projection/filter operator: {:?}", op).into()),
    }
}
