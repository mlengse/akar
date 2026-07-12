use super::ExecutionContext;
use crate::expression_evaluator::ExpressionEvaluator;
use crate::physical_operator::*;
use crate::processor::projection_helper::resolve_projection_column_index;
use kuzu_common::vector::DataChunk;
use kuzu_parser::ast::Expression;
use kuzu_planner::logical_operator::LogicalOperator;
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

pub fn map_and_execute_projection(
    op: &LogicalOperator,
    current_input: Vec<DataChunk>,
    ctx: &mut ExecutionContext,
) -> Result<Vec<DataChunk>, String> {
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
                    let registry = ctx.function_registry.clone().ok_or_else(|| {
                        "No function registry available for expression projection".to_string()
                    })?;

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
                        for be in &p.expressions {
                            let result_vec = eval.evaluate(&be.expression, &chunk)?;
                            fields.push(result_vec);
                        }
                        let size = fields.first().map(|f| f.size()).unwrap_or(chunk.size);
                        output.push(DataChunk {
                            fields,
                            size,
                            field_names: vec![],
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
                    let proj = PhysicalProjection { column_indices };
                    proj.execute(input)?
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
            let sort_keys: Vec<(u32, bool)> = tk
                .sort_keys
                .iter()
                .enumerate()
                .map(|(i, _s)| (i as u32, tk.sort_keys.get(i).map(|s| s.1).unwrap_or(true)))
                .collect();
            let topk = PhysicalTopK {
                sort_keys,
                limit: tk.limit,
                offset: tk.offset,
            };
            let result = topk.execute(current_input)?;
            Ok(result)
        }
        LogicalOperator::OrderBy(o) => {
            let sort_keys: Vec<(u32, bool)> = o
                .sort_keys
                .iter()
                .enumerate()
                .map(|(i, _s)| (i as u32, o.sort_keys.get(i).map(|s| s.1).unwrap_or(true)))
                .collect();
            let order = PhysicalOrderBy { sort_keys };
            let result = order.execute(current_input)?;
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
        LogicalOperator::SemiMasker(s) => {
            let mask = NodeSemiMask::new(s.table_id);
            let masker = PhysicalSemiMasker {
                key_column: s.key_column,
                mask: mask.clone(),
            };
            let result = masker.execute(current_input)?;
            ctx.sip_masks.insert(s.table_id, mask);
            Ok(result)
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
        _ => Err(format!("Not a projection/filter operator: {:?}", op)),
    }
}
