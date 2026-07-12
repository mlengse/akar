use super::ExecutionContext;
use crate::physical_operator::*;
use kuzu_common::vector::DataChunk;
use kuzu_planner::logical_operator::LogicalOperator;

use crate::processor::join_helpers::derive_join_column_indices;
use crate::processor::union_helpers::{flatten_union_child, merge_optional_chunks};

pub fn map_and_execute_join(
    op: &LogicalOperator,
    current_input: Vec<DataChunk>,
    ctx: &mut ExecutionContext,
) -> Result<Vec<DataChunk>, String> {
    match op {
        LogicalOperator::HashJoin(h) => {
            let left_ops = flatten_union_child(&h.build_side);
            let right_ops = flatten_union_child(&h.probe_side);

            let build_chunks = ctx.execute_children(&left_ops)?;
            let probe_chunks = ctx.execute_children(&right_ops)?;

            let (build_cols, probe_cols) =
                derive_join_column_indices(&h.join_keys, &build_chunks, &probe_chunks);
            let join = PhysicalHashJoin {
                build_columns: build_cols,
                probe_columns: probe_cols,
                semi_mask: None,
            };
            let result = join.execute_binary(&build_chunks, &probe_chunks)?;
            Ok(result)
        }
        LogicalOperator::SemiJoin(s) => {
            let left_ops = flatten_union_child(&s.left);
            let right_ops = flatten_union_child(&s.right);

            let build_chunks = ctx.execute_children(&left_ops)?;
            let probe_chunks = ctx.execute_children(&right_ops)?;

            let semi = PhysicalSemiJoin {
                build_columns: vec![0],
                probe_columns: vec![0],
            };
            let result = semi.execute_binary(&build_chunks, &probe_chunks)?;
            Ok(result)
        }
        LogicalOperator::AntiJoin(a) => {
            let left_ops = flatten_union_child(&a.left);
            let right_ops = flatten_union_child(&a.right);

            let build_chunks = ctx.execute_children(&left_ops)?;
            let probe_chunks = ctx.execute_children(&right_ops)?;

            let anti = PhysicalAntiJoin {
                build_columns: vec![0],
                probe_columns: vec![0],
            };
            let result = anti.execute_binary(&build_chunks, &probe_chunks)?;
            Ok(result)
        }
        LogicalOperator::Intersect(ic) => {
            let left_ops = flatten_union_child(&ic.left);
            let right_ops = flatten_union_child(&ic.right);

            let build_chunks = ctx.execute_children(&left_ops)?;
            let probe_chunks = ctx.execute_children(&right_ops)?;

            let intersect = PhysicalIntersect {
                num_build_sides: ic.num_build_sides,
                probe_key_col: 0,
                build_key_col: 0,
            };
            let result = intersect.execute_binary(&build_chunks, &probe_chunks)?;
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
        _ => Err(format!("Not a join operator: {:?}", op)),
    }
}
