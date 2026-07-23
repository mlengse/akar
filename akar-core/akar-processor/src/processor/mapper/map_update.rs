use super::ExecutionContext;
use crate::physical_operator::*;
use akar_common::vector::DataChunk;
use akar_planner::logical_operator::LogicalOperator;

pub fn map_and_execute_update(
    op: &LogicalOperator,
    current_input: Vec<DataChunk>,
    ctx: &mut ExecutionContext,
) -> Result<Vec<DataChunk>, String> {
    match op {
        LogicalOperator::Set(sl) => {
            let table_catalog = ctx
                .table_catalog
                .clone()
                .ok_or_else(|| "No table catalog available for SET".to_string())?;

            let set_op = PhysicalSet {
                table_name: sl.table_name.clone(),
                table_id: sl.table_id,
                column_name: sl.column_name.clone(),
                column_idx: sl.column_idx,
                value: sl.value.clone(),
                is_node: sl.is_node,
                table_catalog,
            };
            let result = set_op.execute(current_input)?;
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
            };
            let result = delete_op.execute(current_input)?;
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
            };
            let result = create_node_op.execute(current_input)?;
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
            };
            let result = create_rel_op.execute(current_input)?;
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
                bound_node_var: ex.bound_node_var.clone(),
                direction: ex.direction.clone(),
                dst_node_var: ex.dst_node_var.clone(),
                dst_table_name: ex.dst_table_name.clone(),
                dst_table_id: ex.dst_table_id,
                table_catalog,
            };
            let result = extend_op.execute(current_input)?;
            Ok(result)
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
                    column_name: set_item.column_name.clone(),
                    column_idx: set_item.column_idx,
                    value: set_item.value.clone(),
                    is_node: set_item.is_node,
                    table_catalog: table_catalog.clone(),
                });
            }

            let mut on_create_ops = Vec::new();
            for set_item in &m.on_create {
                on_create_ops.push(PhysicalSet {
                    table_name: set_item.table_name.clone(),
                    table_id: set_item.table_id,
                    column_name: set_item.column_name.clone(),
                    column_idx: set_item.column_idx,
                    value: set_item.value.clone(),
                    is_node: set_item.is_node,
                    table_catalog: table_catalog.clone(),
                });
            }

            let merge_op = PhysicalMerge {
                table_name: m.table_name.clone(),
                table_id: m.table_id,
                properties: m.properties.clone(),
                on_match: on_match_ops,
                on_create: on_create_ops,
                table_catalog,
            };
            let result = merge_op.execute(current_input)?;
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
                return Err(format!("Table '{}' not found in storage catalog", cf.table_name));
            };

            let copy_op = PhysicalCopyFrom {
                table_name: cf.table_name.clone(),
                table_id: cf.table_id,
                file_path: cf.file_path.clone(),
                columns,
                options: cf.options.clone(),
                table_catalog,
                vfs: ctx.vfs.clone().expect("VFS not initialized in processor"),
            };
            let result = copy_op.execute(current_input)?;
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
            };
            let result = batch_op.execute(current_input)?;
            Ok(result)
        }
        LogicalOperator::Insert(i) => {
            let exec = crate::physical::misc::PhysicalInsert {
                table_name: i.table_name.clone(),
                table_id: i.table_id,
                columns: i.columns.clone(),
                values: i.values.clone(),
                table_catalog: ctx.table_catalog.clone().unwrap(),
            };
            let result = exec.execute(current_input)?;
            Ok(result)
        }
        _ => Err(format!("Not an update operator: {:?}", op)),
    }
}
