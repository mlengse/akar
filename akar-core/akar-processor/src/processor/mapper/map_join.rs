use super::ExecutionContext;
use crate::physical_operator::*;
use akar_common::error::ProcessorError;
use akar_common::types::{PhysicalTypeID, Value};
use akar_common::vector::DataChunk;
use akar_parser::ast::Expression;
use akar_planner::logical_operator::LogicalOperator;

use crate::processor::join_helpers::{JoinKeyBinding, derive_join_bindings};
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

            let (build_orig, probe_orig) = (field_count(&build_chunks), field_count(&probe_chunks));
            let (build_chunks, build_cols, build_appended, probe_chunks, probe_cols, probe_appended) =
                prepare_join_sides(&h.join_keys, build_chunks, probe_chunks)?;

            let join = PhysicalHashJoin::new(build_cols, probe_cols);
            let result = join.execute_binary(&build_chunks, &probe_chunks)?;
            Ok(strip_join_synthetic_columns(
                result,
                build_orig,
                build_appended,
                probe_orig,
                probe_appended,
                true,
            ))
        }
        LogicalOperator::SemiJoin(s) => {
            let left_ops = flatten_union_child(&s.left);
            let right_ops = flatten_union_child(&s.right);

            let build_chunks = ctx.execute_children(&left_ops)?;
            let probe_chunks = ctx.execute_children(&right_ops)?;

            let (_build_orig, probe_orig) = (field_count(&build_chunks), field_count(&probe_chunks));
            let (build_chunks, build_cols, _build_appended, probe_chunks, probe_cols, probe_appended) =
                prepare_join_sides(&s.join_keys, build_chunks, probe_chunks)?;

            let semi = PhysicalSemiJoin {
                build_columns: build_cols,
                probe_columns: probe_cols,
            };
            let result = semi.execute_binary(&build_chunks, &probe_chunks)?;
            Ok(strip_join_synthetic_columns(result, 0, 0, probe_orig, probe_appended, false))
        }
        LogicalOperator::AntiJoin(a) => {
            let left_ops = flatten_union_child(&a.left);
            let right_ops = flatten_union_child(&a.right);

            let build_chunks = ctx.execute_children(&left_ops)?;
            let probe_chunks = ctx.execute_children(&right_ops)?;

            let (_build_orig, probe_orig) = (field_count(&build_chunks), field_count(&probe_chunks));
            let (build_chunks, build_cols, _build_appended, probe_chunks, probe_cols, probe_appended) =
                prepare_join_sides(&a.join_keys, build_chunks, probe_chunks)?;

            let anti = PhysicalAntiJoin {
                build_columns: build_cols,
                probe_columns: probe_cols,
            };
            let result = anti.execute_binary(&build_chunks, &probe_chunks)?;
            Ok(strip_join_synthetic_columns(result, 0, 0, probe_orig, probe_appended, false))
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

fn field_count(chunks: &[DataChunk]) -> usize {
    chunks.first().map(|c| c.fields.len()).unwrap_or(0)
}

/// Split join bindings into per-side (column, map_key) lists.
fn split_bindings(bindings: &[JoinKeyBinding]) -> (Vec<u32>, Vec<Option<String>>, Vec<u32>, Vec<Option<String>>) {
    let mut build_cols = Vec::new();
    let mut build_keys = Vec::new();
    let mut probe_cols = Vec::new();
    let mut probe_keys = Vec::new();
    for b in bindings {
        build_cols.push(b.build_col);
        build_keys.push(b.build_map_key.clone());
        probe_cols.push(b.probe_col);
        probe_keys.push(b.probe_map_key.clone());
    }
    (build_cols, build_keys, probe_cols, probe_keys)
}

/// Resolve join key columns and materialize map/struct key extraction (P53.26).
/// Returns the (possibly extended) build/probe chunks with the resolved column
/// indices, plus how many synthetic columns were appended to each side.
fn prepare_join_sides(
    join_keys: &[Expression],
    build_chunks: Vec<DataChunk>,
    probe_chunks: Vec<DataChunk>,
) -> Result<(Vec<DataChunk>, Vec<u32>, usize, Vec<DataChunk>, Vec<u32>, usize), ProcessorError> {
    let bindings = derive_join_bindings(join_keys, &build_chunks, &probe_chunks);
    let (build_cols, build_keys, probe_cols, probe_keys) = split_bindings(&bindings);
    let (build_chunks, build_cols, build_appended) = materialize_map_keys(&build_chunks, &build_cols, &build_keys)?;
    let (probe_chunks, probe_cols, probe_appended) = materialize_map_keys(&probe_chunks, &probe_cols, &probe_keys)?;
    Ok((build_chunks, build_cols, build_appended, probe_chunks, probe_cols, probe_appended))
}

/// Append synthetic columns holding map/struct key values for every binding
/// that needs extraction, and point the join column at them. Complex values are
/// preserved because `build_arrow_from_values` emits Arrow arrays directly
/// (unlike `store_value_in_vector`, which drops them to NULL).
fn materialize_map_keys(
    chunks: &[DataChunk],
    cols: &[u32],
    keys: &[Option<String>],
) -> Result<(Vec<DataChunk>, Vec<u32>, usize), ProcessorError> {
    let mut out: Vec<DataChunk> = chunks.to_vec();
    let mut new_cols: Vec<u32> = cols.to_vec();
    let base_count = out.first().map(|c| c.fields.len()).unwrap_or(0);
    let mut appended = 0usize;
    // Dedupe repeated (column, key) extractions so each synthetic column is
    // added only once and shared across join keys.
    let mut done: std::collections::HashMap<(u32, String), u32> = std::collections::HashMap::new();

    for (i, key) in keys.iter().enumerate() {
        let Some(key_name) = key else { continue };
        let col = cols[i];
        if let Some(&idx) = done.get(&(col, key_name.clone())) {
            new_cols[i] = idx;
            continue;
        }
        let new_idx = (base_count + appended) as u32;
        let mut ok = false;
        for chunk in out.iter_mut() {
            if col as usize >= chunk.fields.len() {
                continue;
            }
            let extracted: Vec<Value> = (0..chunk.size)
                .map(|row| {
                    crate::expression_evaluator::map_property_value(
                        &chunk.get_value(col as usize, row).unwrap_or(Value::Null),
                        key_name,
                    )
                })
                .collect();
            let t = extracted
                .iter()
                .find(|v| !matches!(v, Value::Null))
                .map(|v| v.physical_type())
                .unwrap_or(PhysicalTypeID::Int64);
            let arr = crate::expression_evaluator::build_arrow_from_values(&extracted, t, chunk.size)
                .map_err(|e| e.to_string())?;
            chunk.fields.push(arr.array);
            chunk.field_types.push(arr.physical_type);
            chunk.field_names.push(format!("__join_extract_{}_{}", col, key_name));
            ok = true;
        }
        if ok {
            appended += 1;
        }
        done.insert((col, key_name.clone()), new_idx);
        new_cols[i] = new_idx;
    }
    Ok((out, new_cols, appended))
}

/// Remove synthetic extraction columns from join output chunks.
///
/// For hash joins the output is `[build columns..., probe columns...]`; for
/// semi/anti joins it is `[probe columns...]`. Synthetic columns are the last
/// `build_appended` / `probe_appended` fields of their side.
fn strip_join_synthetic_columns(
    result: Vec<DataChunk>,
    build_orig: usize,
    build_appended: usize,
    probe_orig: usize,
    probe_appended: usize,
    output_has_build: bool,
) -> Vec<DataChunk> {
    let mut to_remove: Vec<usize> = Vec::new();
    if output_has_build {
        to_remove.extend(build_orig..build_orig + build_appended);
        let probe_base = build_orig + build_appended + probe_orig;
        to_remove.extend(probe_base..probe_base + probe_appended);
    } else {
        to_remove.extend(probe_orig..probe_orig + probe_appended);
    }
    if to_remove.is_empty() {
        return result;
    }
    result
        .into_iter()
        .map(|mut chunk| {
            for &idx in to_remove.iter().rev() {
                if idx < chunk.fields.len() {
                    chunk.fields.remove(idx);
                    if idx < chunk.field_types.len() {
                        chunk.field_types.remove(idx);
                    }
                    if idx < chunk.field_names.len() {
                        chunk.field_names.remove(idx);
                    }
                }
            }
            chunk
        })
        .collect()
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
