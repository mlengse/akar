use super::ExecutionContext;
use crate::physical_operator::*;
use crate::processor::plan_serializer::serialize_plan_tree;
use kuzu_common::vector::DataChunk;
use kuzu_planner::logical_operator::LogicalOperator;

pub fn map_and_execute_ddl(
    op: &LogicalOperator,
    current_input: Vec<DataChunk>,
    ctx: &mut ExecutionContext,
) -> Result<Vec<DataChunk>, String> {
    match op {
        LogicalOperator::Explain(ex) => {
            // Serialize the inner plan tree to a string
            let plan_str = serialize_plan_tree(&ex.inner, 0);
            let explain = PhysicalExplain { inner_plan: plan_str };
            let result = explain.execute(vec![])?;
            Ok(result)
        }
        LogicalOperator::StandaloneCall(c) => {
            if let Some(ref handler) = ctx.standalone_call_handler {
                let result = handler.execute_call(&c.function_name, &c.args)?;
                Ok(result)
            } else {
                Err(format!(
                    "No standalone call handler available to execute '{}'",
                    c.function_name
                ))
            }
        }
        LogicalOperator::TableFunctionCall(tf) => {
            let result = ctx.processor.execute_table_function(tf)?;
            Ok(result)
        }
        LogicalOperator::Foreach(fc) => {
            let foreach_op = PhysicalForeach {
                variable: fc.variable.clone(),
                expression: fc.expression.clone(),
                sub_plans: fc.sub_plans.clone(),
                function_registry: ctx.function_registry.clone(),
                table_catalog: ctx.table_catalog.clone(),
                vfs: ctx.vfs.clone(),
            };
            let result = foreach_op.execute(current_input)?;
            Ok(result)
        }
        LogicalOperator::CreateNodeTable(_)
        | LogicalOperator::CreateRelTable(_)
        | LogicalOperator::DropTable(_)
        | LogicalOperator::AlterTable(_)
        | LogicalOperator::CreateIndex(_)
        | LogicalOperator::DropIndex(_)
        | LogicalOperator::CreateVectorIndex(_) => {
            Ok(vec![DataChunk {
                fields: vec![],
                size: 1,
                field_names: vec![],
            sel_vector: None,
            }])
        }
        LogicalOperator::CreateSequence(_)
        | LogicalOperator::DropSequence(_)
        | LogicalOperator::CreateDml(_)
        | LogicalOperator::ExportDatabase(_)
        | LogicalOperator::ImportDatabase(_) => {
            Ok(vec![DataChunk {
                fields: vec![],
                size: 0,
                field_names: vec![],
            sel_vector: None,
            }])
        }
        LogicalOperator::CreateFtsIndex(c) => {
            if let Some(ref tc) = ctx.table_catalog {
                let fts_index = PhysicalCreateFtsIndex {
                    index_name: c.index_name.clone(),
                    table_name: c.table_name.clone(),
                    column_name: c.column_name.clone(),
                    docs_table: c.docs_table.clone(),
                    terms_table: c.terms_table.clone(),
                    posting_table: c.posting_table.clone(),
                    table_catalog: tc.clone(),
                };
                let result = fts_index.execute(current_input)?;
                Ok(result)
            } else {
                Err("CREATE FTS INDEX requires a table catalog".into())
            }
        }
        LogicalOperator::FtsScan(s) => {
            if let Some(ref tc) = ctx.table_catalog {
                let fts_scan = PhysicalFtsScan {
                    index_name: s.index_name.clone(),
                    query_string: s.query_string.clone(),
                    docs_table: s.docs_table.clone(),
                    terms_table: s.terms_table.clone(),
                    posting_table: s.posting_table.clone(),
                    table_catalog: tc.clone(),
                };
                let result = fts_scan.execute(current_input)?;
                Ok(result)
            } else {
                Err("FTS scan requires a table catalog".into())
            }
        }
        LogicalOperator::EmptyResult(_) => {
            let exec = crate::physical::misc::PhysicalEmptyResult;
            let result = exec.execute(current_input)?;
            Ok(result)
        }
        LogicalOperator::MultiplicityReducer(m) => {
            let exec = crate::physical::misc::PhysicalMultiplicityReducer {
                key_columns: m.key_columns.clone(),
            };
            let input = if !m.children.is_empty() {
                ctx.execute_children(&m.children)?
            } else {
                current_input
            };
            let result = exec.execute(input)?;
            Ok(result)
        }
        LogicalOperator::Skip(s) => {
            let exec = crate::physical::misc::PhysicalSkip {
                skip_count: s.offset as usize,
            };
            let input = if !s.children.is_empty() {
                ctx.execute_children(&s.children)?
            } else {
                current_input
            };
            let result = exec.execute(input)?;
            Ok(result)
        }
        LogicalOperator::ExtensionClause(e) => {
            let exec = crate::physical::misc::PhysicalExtensionClause {
                action: e.action.clone(),
                extension_name: e.extension_name.clone(),
            };
            let result = exec.execute(current_input)?;
            Ok(result)
        }
        _ => Err(format!("DDL operator not implemented in mapper: {:?}", op)),
    }
}
