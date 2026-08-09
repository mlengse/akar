use super::ExecutionContext;
use crate::physical_operator::*;
use akar_common::error::ProcessorError;
use akar_common::vector::DataChunk;
use akar_parser::ast::Expression;
use akar_planner::logical_operator::LogicalOperator;

use crate::processor::join_helpers::derive_join_column_indices;
use crate::processor::union_helpers::{flatten_union_child, merge_optional_chunks};

pub fn map_and_execute_join(
    op: &LogicalOperator,
    current_input: Vec<DataChunk>,
    ctx: &mut ExecutionContext,
) -> Result<Vec<DataChunk>, ProcessorError> {
    match op {
        LogicalOperator::HashJoin(h) => {
            let left_ops = flatten_union_child(&h.build_side);
            let right_ops = flatten_union_child(&h.probe_side);

            let build_chunks = ctx.execute_children(&left_ops)?;
            let probe_chunks = ctx.execute_children(&right_ops)?;

            let (build_cols, probe_cols) = derive_join_column_indices(&h.join_keys, &build_chunks, &probe_chunks);
            let join = PhysicalHashJoin::new(build_cols, probe_cols);
            let result = join.execute_binary(&build_chunks, &probe_chunks)?;
            Ok(result)
        }
        LogicalOperator::SemiJoin(s) => {
            let left_ops = flatten_union_child(&s.left);
            let right_ops = flatten_union_child(&s.right);

            let build_chunks = ctx.execute_children(&left_ops)?;
            let probe_chunks = ctx.execute_children(&right_ops)?;

            let (build_cols, probe_cols) = derive_join_column_indices(&s.join_keys, &build_chunks, &probe_chunks);
            let semi = PhysicalSemiJoin {
                build_columns: build_cols,
                probe_columns: probe_cols,
            };
            let result = semi.execute_binary(&build_chunks, &probe_chunks)?;
            Ok(result)
        }
        LogicalOperator::AntiJoin(a) => {
            let left_ops = flatten_union_child(&a.left);
            let right_ops = flatten_union_child(&a.right);

            let build_chunks = ctx.execute_children(&left_ops)?;
            let probe_chunks = ctx.execute_children(&right_ops)?;

            let (build_cols, probe_cols) = derive_join_column_indices(&a.join_keys, &build_chunks, &probe_chunks);
            let anti = PhysicalAntiJoin {
                build_columns: build_cols,
                probe_columns: probe_cols,
            };
            let result = anti.execute_binary(&build_chunks, &probe_chunks)?;
            Ok(result)
        }
        LogicalOperator::Intersect(ic) => {
            // Build side is a (possibly nested) Union of per-pattern pipelines,
            // each executed independently so the intersect has one hash table per
            // pattern. Probe side is the shared-node scan.
            let build_sides = collect_union_sides(&ic.left);
            let probe_ops = flatten_union_child(&ic.right);

            let build_chunk_sides: Vec<Vec<DataChunk>> = build_sides
                .iter()
                .map(|ops| ctx.execute_children(ops))
                .collect::<Result<_, _>>()?;
            let probe_chunks = ctx.execute_children(&probe_ops)?;

            let (probe_key_col, build_key_col) =
                resolve_intersect_key_cols(&ic.build_key_exprs, &probe_chunks, &build_chunk_sides);

            let intersect = PhysicalIntersect {
                num_build_sides: build_chunk_sides.len() as u32,
                probe_key_col,
                build_key_col,
            };
            let result = intersect.execute_sides(&build_chunk_sides, &probe_chunks)?;
            Ok(result)
        }
        LogicalOperator::CrossProduct(cp) => {
            let left_ops = flatten_union_child(&cp.left);
            let right_ops = flatten_union_child(&cp.right);
            let build_chunks = ctx.execute_children(&left_ops)?;
            let probe_chunks = ctx.execute_children(&right_ops)?;
            let cross = PhysicalCrossProduct;
            let result = cross.execute_binary(&build_chunks, &probe_chunks)?;
            Ok(result)
        }
        LogicalOperator::OptionalMatch(om) => {
            // Execute left (required) subtree
            let left_ops = flatten_union_child(&om.left);
            let left_result = ctx.execute_children(&left_ops)?;

            // Execute right (optional) subtree
            let right_ops = flatten_union_child(&om.right);
            let right_result = ctx.execute_children(&right_ops)?;

            // Combine: use flattened row-level merge
            let merged = merge_optional_chunks(left_result, right_result)?;
            Ok(merged)
        }
        LogicalOperator::RecursiveExtend(re) => {
            let scan = PhysicalRecursiveExtend {
                source_table_id: re.source_table_id,
                rel_table_ids: re.rel_table_ids.clone(),
                lower_bound: re.lower_bound,
                upper_bound: re.upper_bound,
                direction: re.direction,
                semantic: re.semantic,
                table_catalog: ctx.table_catalog.clone(),
                weight_property: re.weight_property.clone(),
                cost_output_name: re.cost_output_name.clone(),
            };
            let result = scan.execute(current_input)?;
            Ok(result)
        }
        _ => Err(format!("Not a join operator: {:?}", op).into()),
    }
}

/// Flatten a (possibly nested) `Union` subtree into a list of independent
/// operator pipelines — one per WCOJ build side.
fn collect_union_sides(op: &LogicalOperator) -> Vec<Vec<LogicalOperator>> {
    match op {
        LogicalOperator::Union(u) => {
            let mut sides = collect_union_sides(&u.left);
            sides.extend(collect_union_sides(&u.right));
            sides
        }
        other => vec![flatten_union_child(other)],
    }
}

/// Resolve the shared-node key column index on the probe and build sides.
///
/// The key is derived from the first build key expression (a reference to the
/// shared variable, e.g. `a`), resolved against `field_names` as `a.id`.
fn resolve_intersect_key_cols(
    build_key_exprs: &[Expression],
    probe_chunks: &[DataChunk],
    build_sides: &[Vec<DataChunk>],
) -> (u32, u32) {
    let var = build_key_exprs.first().and_then(|e| match e {
        Expression::Variable(v) => Some(v.clone()),
        Expression::PropertyAccess(obj, _) => {
            if let Expression::Variable(v) = &**obj {
                Some(v.clone())
            } else {
                None
            }
        }
        _ => None,
    });

    let probe_names: Vec<&str> = probe_chunks
        .first()
        .map(|c| c.field_names.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();
    let build_names: Vec<&str> = build_sides
        .first()
        .and_then(|s| s.first())
        .map(|c| c.field_names.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();

    let mut probe_col = 0u32;
    let mut build_col = 0u32;
    if let Some(var) = var {
        let candidates = [format!("{var}._id"), format!("{var}.id"), var];
        for c in &candidates {
            if let Some(idx) = probe_names.iter().position(|n| n == c) {
                probe_col = idx as u32;
                break;
            }
        }
        for c in &candidates {
            if let Some(idx) = build_names.iter().position(|n| n == c) {
                build_col = idx as u32;
                break;
            }
        }
    }
    (probe_col, build_col)
}
