use super::ExecutionContext;
use crate::physical_operator::*;
use akar_common::error::ProcessorError;
use akar_common::vector::DataChunk;
use akar_planner::logical_operator::LogicalOperator;

pub fn map_and_execute_update(
    op: &LogicalOperator,
    current_input: Vec<DataChunk>,
    ctx: &mut ExecutionContext,
) -> Result<Vec<DataChunk>, ProcessorError> {
    match op {
        LogicalOperator::Set(sl) => {
            let table_catalog = ctx
                .table_catalog
                .clone()
                .ok_or_else(|| "No table catalog available for SET".to_string())?;

            let set_op = PhysicalSet {
                table_name: sl.table_name.clone(),
                table_id: sl.table_id,
                is_node: sl.is_node,
                items: sl.items.clone(),
                table_catalog,
                txn_id: ctx.txn_id,
                undo_sink: Some(ctx.processor.undo_sink()),
                function_registry: ctx.function_registry.clone(),
                emit_count: sl.emit_count,
            };
            let result = set_op.execute(current_input)?;
            // Record written rows for OCC conflict detection
            record_set_writes(sl.table_id, &result, ctx);
            Ok(result)
        }
        LogicalOperator::Delete(dl) => {
            let table_catalog = ctx
                .table_catalog
                .clone()
                .ok_or_else(|| "No table catalog available for DELETE".to_string())?;

            let delete_op = PhysicalDelete {
                table_name: dl.table_name.clone(),
                table_id: dl.table_id,
                primary_key_column: dl.primary_key_column.clone(),
                is_node: dl.is_node,
                detach: dl.detach,
                row_indices: Vec::new(),
                table_catalog,
                txn_id: ctx.txn_id,
                undo_sink: Some(ctx.processor.undo_sink()),
            };
            let result = delete_op.execute(current_input)?;
            // Record written rows for OCC conflict detection
            record_delete_writes(dl.table_id, &result, ctx);
            Ok(result)
        }
        LogicalOperator::CreateNode(cn) => {
            let table_catalog = ctx
                .table_catalog
                .clone()
                .ok_or_else(|| "No table catalog available for CREATE".to_string())?;

            let create_node_op = PhysicalInsertNode {
                table_name: cn.table_name.clone(),
                table_id: cn.table_id,
                out_var_name: cn.out_var_name.clone(),
                properties: cn.properties.clone(),
                table_catalog,
                txn_id: ctx.txn_id,
                undo_sink: Some(ctx.processor.undo_sink()),
            };
            let result = create_node_op.execute(current_input)?;
            // Record written rows for OCC conflict detection
            record_insert_writes(cn.table_id, &result, ctx);
            Ok(result)
        }
        LogicalOperator::CreateRel(cr) => {
            let table_catalog = ctx
                .table_catalog
                .clone()
                .ok_or_else(|| "No table catalog available for CREATE".to_string())?;

            let create_rel_op = PhysicalInsertRel {
                table_name: cr.table_name.clone(),
                table_id: cr.table_id,
                src_node_name: cr.src_node_name.clone(),
                dst_node_name: cr.dst_node_name.clone(),
                properties: cr.properties.clone(),
                table_catalog,
                txn_id: ctx.txn_id,
                undo_sink: Some(ctx.processor.undo_sink()),
            };
            let result = create_rel_op.execute(current_input)?;
            // Record written rows for OCC conflict detection
            record_insert_writes(cr.table_id, &result, ctx);
            Ok(result)
        }
        LogicalOperator::Extend(ex) => {
            let table_catalog = ctx
                .table_catalog
                .clone()
                .ok_or_else(|| "No table catalog available for Extend".to_string())?;

            let extend_op = PhysicalExtend {
                rel_table_name: ex.rel_table_name.clone(),
                rel_table_id: ex.rel_table_id,
                rel_var: ex.rel_var.clone(),
                bound_node_var: ex.bound_node_var.clone(),
                direction: ex.direction.clone(),
                dst_node_var: ex.dst_node_var.clone(),
                dst_table_name: ex.dst_table_name.clone(),
                dst_table_id: ex.dst_table_id,
                table_catalog,
            };
            let result = extend_op.execute(current_input)?;
            // Record written rows for OCC conflict detection
            record_insert_writes(ex.rel_table_id, &result, ctx);
            Ok(result)
        }
        LogicalOperator::OptionalExtend(oe) => {
            let table_catalog = ctx
                .table_catalog
                .clone()
                .ok_or_else(|| "No table catalog available for OptionalExtend".to_string())?;

            let input = if oe.children.is_empty() {
                current_input
            } else {
                ctx.execute_children(&oe.children)?
            };

            let optional_extend_op = PhysicalOptionalExtend {
                rel_table_name: oe.rel_table_name.clone(),
                rel_table_id: oe.rel_table_id,
                rel_var: oe.rel_var.clone(),
                src_node_var: oe.src_node_var.clone(),
                dst_node_var: oe.dst_node_var.clone(),
                direction: oe.direction.clone(),
                table_catalog,
            };
            Ok(optional_extend_op.execute(input)?)
        }
        LogicalOperator::Merge(m) => {
            let table_catalog = ctx
                .table_catalog
                .clone()
                .ok_or_else(|| "No table catalog available for MERGE".to_string())?;

            let mut on_match_ops = Vec::new();
            for set_item in &m.on_match {
                on_match_ops.push(PhysicalSet {
                    table_name: set_item.table_name.clone(),
                    table_id: set_item.table_id,
                    is_node: set_item.is_node,
                    items: set_item.items.clone(),
                    table_catalog: table_catalog.clone(),
                    txn_id: ctx.txn_id,
                    undo_sink: Some(ctx.processor.undo_sink()),
                    function_registry: ctx.function_registry.clone(),
                    emit_count: false,
                });
            }

            let mut on_create_ops = Vec::new();
            for set_item in &m.on_create {
                on_create_ops.push(PhysicalSet {
                    table_name: set_item.table_name.clone(),
                    table_id: set_item.table_id,
                    is_node: set_item.is_node,
                    items: set_item.items.clone(),
                    table_catalog: table_catalog.clone(),
                    txn_id: ctx.txn_id,
                    undo_sink: Some(ctx.processor.undo_sink()),
                    function_registry: ctx.function_registry.clone(),
                    emit_count: false,
                });
            }

            let merge_op = PhysicalMerge {
                table_name: m.table_name.clone(),
                table_id: m.table_id,
                properties: m.properties.clone(),
                on_match: on_match_ops,
                on_create: on_create_ops,
                table_catalog,
                txn_id: ctx.txn_id,
                undo_sink: Some(ctx.processor.undo_sink()),
            };
            let result = merge_op.execute(current_input)?;
            // Record written rows for OCC conflict detection
            record_insert_writes(m.table_id, &result, ctx);
            Ok(result)
        }
        LogicalOperator::MergeRel(mr) => {
            let table_catalog = ctx
                .table_catalog
                .clone()
                .ok_or_else(|| "No table catalog available for MERGE".to_string())?;

            let mut on_match_ops = Vec::new();
            for set_item in &mr.on_match {
                on_match_ops.push(PhysicalSet {
                    table_name: set_item.table_name.clone(),
                    table_id: set_item.table_id,
                    is_node: set_item.is_node,
                    items: set_item.items.clone(),
                    table_catalog: table_catalog.clone(),
                    txn_id: ctx.txn_id,
                    undo_sink: Some(ctx.processor.undo_sink()),
                    function_registry: ctx.function_registry.clone(),
                    emit_count: false,
                });
            }

            let mut on_create_ops = Vec::new();
            for set_item in &mr.on_create {
                on_create_ops.push(PhysicalSet {
                    table_name: set_item.table_name.clone(),
                    table_id: set_item.table_id,
                    is_node: set_item.is_node,
                    items: set_item.items.clone(),
                    table_catalog: table_catalog.clone(),
                    txn_id: ctx.txn_id,
                    undo_sink: Some(ctx.processor.undo_sink()),
                    function_registry: ctx.function_registry.clone(),
                    emit_count: false,
                });
            }

            let merge_rel_op = PhysicalMergeRel {
                rel_table_name: mr.rel_table_name.clone(),
                rel_table_id: mr.rel_table_id,
                edge_var: mr.edge_var.clone(),
                src_node_var: mr.src_node_var.clone(),
                dst_node_var: mr.dst_node_var.clone(),
                direction: akar_parser::ast::EdgeDirection::LeftToRight,
                properties: mr.properties.clone(),
                on_match: on_match_ops,
                on_create: on_create_ops,
                table_catalog,
                txn_id: ctx.txn_id,
                undo_sink: Some(ctx.processor.undo_sink()),
            };
            let result = merge_rel_op.execute(current_input)?;
            // Record written rows for OCC conflict detection
            record_insert_writes(mr.rel_table_id, &result, ctx);
            Ok(result)
        }
        LogicalOperator::CopyFrom(cf) => {
            let table_catalog = ctx
                .table_catalog
                .clone()
                .ok_or_else(|| "No table catalog available for COPY FROM".to_string())?;

            // Get column definitions from the table catalog
            let columns = if let Some(node_table) = table_catalog.get_node_table_by_name(&cf.table_name) {
                node_table.columns.clone()
            } else if let Some(rel_table) = table_catalog.get_rel_table_by_name(&cf.table_name) {
                rel_table.columns.clone()
            } else {
                return Err(format!("Table '{}' not found in storage catalog", cf.table_name).into());
            };

            let copy_op = PhysicalCopyFrom {
                table_name: cf.table_name.clone(),
                table_id: cf.table_id,
                file_path: cf.file_path.clone(),
                columns,
                options: cf.options.clone(),
                table_catalog,
                vfs: ctx
                    .vfs
                    .clone()
                    .ok_or_else(|| "VFS not initialized in processor".to_string())?,
                txn_id: ctx.txn_id,
                undo_sink: Some(ctx.processor.undo_sink()),
            };
            let result = copy_op.execute(current_input)?;
            // Record written rows for OCC conflict detection
            record_insert_writes(cf.table_id, &result, ctx);
            Ok(result)
        }
        LogicalOperator::BatchInsert(bi) => {
            let table_catalog = ctx
                .table_catalog
                .clone()
                .ok_or_else(|| "No table catalog available for BATCH INSERT".to_string())?;

            let batch_op = PhysicalBatchInsert {
                table_name: bi.table_name.clone(),
                table_id: bi.table_id,
                rows: bi.rows.clone(),
                table_catalog,
                txn_id: ctx.txn_id,
                undo_sink: Some(ctx.processor.undo_sink()),
            };
            let result = batch_op.execute(current_input)?;
            // Record written rows for OCC conflict detection
            record_insert_writes(bi.table_id, &result, ctx);
            Ok(result)
        }
        LogicalOperator::Insert(i) => {
            let exec = crate::physical::misc::PhysicalInsert {
                table_name: i.table_name.clone(),
                table_id: i.table_id,
                columns: i.columns.clone(),
                values: i.values.clone(),
                table_catalog: ctx.table_catalog.clone().unwrap(),
                txn_id: ctx.txn_id,
                undo_sink: Some(ctx.processor.undo_sink()),
            };
            let result = exec.execute(current_input)?;
            // Record written rows for OCC conflict detection
            record_insert_writes(i.table_id, &result, ctx);
            Ok(result)
        }
        _ => Err(format!("Not an update operator: {:?}", op).into()),
    }
}

/// Record rows written by a SET operation for OCC conflict detection.
/// The result DataChunk carries the updated row indices under the `_id`
/// pseudo-column (P53.30); older outputs put a single updated-count in column 0.
fn record_set_writes(table_id: u64, result: &[DataChunk], ctx: &mut ExecutionContext) {
    if let Some(chunk) = result.first() {
        let id_col = chunk
            .field_names
            .iter()
            .position(|n| n == "_id" || n.ends_with("._id"))
            .unwrap_or(0);
        for row in 0..chunk.size {
            if !chunk.fields.is_empty() {
                if let Some(akar_common::types::Value::Int64(row_idx)) = chunk.get_value(id_col, row) {
                    ctx.written_rows.push((table_id, row_idx as u64));
                }
            }
        }
    }
}

/// Record rows written by a DELETE operation for OCC conflict detection.
/// The result DataChunk contains the row indices that were deleted (first column).
fn record_delete_writes(table_id: u64, result: &[DataChunk], ctx: &mut ExecutionContext) {
    if let Some(chunk) = result.first() {
        for row in 0..chunk.size {
            if !chunk.fields.is_empty() {
                if let Some(akar_common::types::Value::Int64(row_idx)) = chunk.get_value(0, row) {
                    ctx.written_rows.push((table_id, row_idx as u64));
                }
            }
        }
    }
}

/// Record rows written by an INSERT operation for OCC conflict detection.
/// When the result chunk contains an `_id` pseudo-column (Merge output, P53.31)
/// or a second column with assigned row IDs (Create/BatchInsert), tracks at row
/// level. Otherwise, row-level inserts are not tracked (PK uniqueness is
/// enforced by the storage layer's hash index).
fn record_insert_writes(table_id: u64, result: &[DataChunk], ctx: &mut ExecutionContext) {
    if let Some(chunk) = result.first() {
        // Column 0 = inserted_count, Column 1 = assigned row IDs (legacy); a
        // Merge output names the row ids `_id` at its last column instead.
        if let Some(id_col) = chunk.field_names.iter().position(|n| n == "_id" || n.ends_with("._id")) {
            for row in 0..chunk.size {
                if let Some(akar_common::types::Value::Int64(row_id)) = chunk.get_value(id_col, row) {
                    ctx.written_rows.push((table_id, row_id as u64));
                }
            }
        } else if chunk.fields.len() > 1 {
            for row in 0..chunk.fields[1].len() {
                if let Some(akar_common::types::Value::Int64(row_id)) = chunk.get_value(1, row) {
                    ctx.written_rows.push((table_id, row_id as u64));
                }
            }
        }
    }
}