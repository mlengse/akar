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
        LogicalOperator::CreateNodeTable(c) => {
            let tc = ctx.table_catalog.as_ref()
                .ok_or("CREATE NODE TABLE requires a table catalog")?;
            let columns: Vec<kuzu_storage::table::ColumnDefinition> = c.columns.iter().map(|col| {
                kuzu_storage::table::ColumnDefinition {
                    name: col.name.clone(),
                    logical_type: col.logical_type,
                    is_primary_key: col.is_primary_key,
                    compression: col.compression,
                }
            }).collect();
            tc.create_node_table(c.name.clone(), columns);
            tracing::info!("Pipeline: Created node table '{}'", c.name);
            Ok(ddl_success_chunk(&format!("Node table '{}' created", c.name)))
        }
        LogicalOperator::CreateRelTable(c) => {
            let tc = ctx.table_catalog.as_ref()
                .ok_or("CREATE REL TABLE requires a table catalog")?;
            let from_id = tc.get_node_table_by_name(&c.from)
                .map(|t| t.table_id)
                .ok_or_else(|| format!("From table '{}' not found", c.from))?;
            let to_id = tc.get_node_table_by_name(&c.to)
                .map(|t| t.table_id)
                .ok_or_else(|| format!("To table '{}' not found", c.to))?;
            let columns: Vec<kuzu_storage::table::ColumnDefinition> = c.columns.iter().map(|col| {
                kuzu_storage::table::ColumnDefinition {
                    name: col.name.clone(),
                    logical_type: col.logical_type,
                    is_primary_key: col.is_primary_key,
                    compression: col.compression,
                }
            }).collect();
            tc.create_rel_table(c.name.clone(), from_id, to_id, columns);
            tracing::info!("Pipeline: Created rel table '{}' ({} -> {})", c.name, c.from, c.to);
            Ok(ddl_success_chunk(&format!("Rel table '{}' created", c.name)))
        }
        LogicalOperator::DropTable(d) => {
            let tc = ctx.table_catalog.as_ref()
                .ok_or("DROP TABLE requires a table catalog")?;
            let dropped = tc.drop_node_table(&d.name) || tc.drop_rel_table(&d.name);
            if dropped {
                tracing::info!("Pipeline: Dropped table '{}'", d.name);
                Ok(ddl_success_chunk(&format!("Table '{}' dropped", d.name)))
            } else {
                Err(format!("Table '{}' not found", d.name))
            }
        }
        LogicalOperator::AlterTable(a) => {
            let tc = ctx.table_catalog.as_ref()
                .ok_or("ALTER TABLE requires a table catalog")?;
            match &a.action {
                kuzu_parser::ast::AlterAction::AddColumn { name, type_name } => {
                    let logical_type = parse_type_simple(type_name)?;
                    let mut table = tc.get_node_table_by_name_mut(&a.table_name)
                        .ok_or_else(|| format!("Table '{}' not found", a.table_name))?;
                    if table.columns.iter().any(|c| c.name.eq_ignore_ascii_case(name)) {
                        return Err(format!("Column '{}' already exists in '{}'", name, a.table_name));
                    }
                    table.columns.push(kuzu_storage::table::ColumnDefinition {
                        name: name.clone(),
                        logical_type,
                        is_primary_key: false,
                        compression: kuzu_common::enums::CompressionType::Uncompressed,
                    });
                    tracing::info!("Pipeline: Added column '{}' to '{}'", name, a.table_name);
                    Ok(ddl_success_chunk(&format!("Column '{}' added to table '{}'", name, a.table_name)))
                }
                kuzu_parser::ast::AlterAction::DropColumn { name } => {
                    let mut table = tc.get_node_table_by_name_mut(&a.table_name)
                        .ok_or_else(|| format!("Table '{}' not found", a.table_name))?;
                    let pos = table.columns.iter().position(|c| c.name == *name)
                        .ok_or_else(|| format!("Column '{}' not found in '{}'", name, a.table_name))?;
                    if table.columns[pos].is_primary_key {
                        return Err(format!("Cannot drop primary key column '{}'", name));
                    }
                    table.columns.remove(pos);
                    tracing::info!("Pipeline: Dropped column '{}' from '{}'", name, a.table_name);
                    Ok(ddl_success_chunk(&format!("Column '{}' dropped from table '{}'", name, a.table_name)))
                }
                kuzu_parser::ast::AlterAction::RenameColumn { old_name, new_name } => {
                    {
                        let table = tc.get_node_table_by_name(&a.table_name)
                            .ok_or_else(|| format!("Table '{}' not found", a.table_name))?;
                        if !table.columns.iter().any(|c| c.name == *old_name) {
                            return Err(format!("Column '{}' not found in '{}'", old_name, a.table_name));
                        }
                        if table.columns.iter().any(|c| c.name == *new_name) {
                            return Err(format!("Column '{}' already exists in '{}'", new_name, a.table_name));
                        }
                    }
                    let mut table = tc.get_node_table_by_name_mut(&a.table_name).unwrap();
                    let col = table.columns.iter_mut().find(|c| c.name == *old_name).unwrap();
                    col.name = new_name.clone();
                    tracing::info!("Pipeline: Renamed column '{}' to '{}' in '{}'", old_name, new_name, a.table_name);
                    Ok(ddl_success_chunk(&format!("Column '{}' renamed to '{}' in table '{}'", old_name, new_name, a.table_name)))
                }
                kuzu_parser::ast::AlterAction::RenameTable { new_name } => {
                    if tc.get_node_table_by_name(new_name).is_some() || tc.get_rel_table_by_name(new_name).is_some() {
                        return Err(format!("Table '{}' already exists", new_name));
                    }
                    if let Some(mut table) = tc.get_node_table_by_name_mut(&a.table_name) {
                        table.name = new_name.clone();
                    } else if let Some(mut table) = tc.get_rel_table_by_name_mut(&a.table_name) {
                        table.name = new_name.clone();
                    } else {
                        return Err(format!("Table '{}' not found", a.table_name));
                    }
                    tracing::info!("Pipeline: Renamed table '{}' to '{}'", a.table_name, new_name);
                    Ok(ddl_success_chunk(&format!("Table '{}' renamed to '{}'", a.table_name, new_name)))
                }
            }
        }
        LogicalOperator::CreateIndex(idx) => {
            let tc = ctx.table_catalog.as_ref()
                .ok_or("CREATE INDEX requires a table catalog")?;
            tc.create_art_index(&idx.table_name, &idx.index_name)?;
            tracing::info!("Pipeline: Created ART index '{}' on '{}'", idx.index_name, idx.table_name);
            Ok(ddl_success_chunk(&format!("ART index '{}' created on table '{}'", idx.index_name, idx.table_name)))
        }
        LogicalOperator::DropIndex(idx) => {
            let tc = ctx.table_catalog.as_ref()
                .ok_or("DROP INDEX requires a table catalog")?;
            tc.drop_art_index(&idx.table_name)?;
            tracing::info!("Pipeline: Dropped index '{}' from '{}'", idx.index_name, idx.table_name);
            Ok(ddl_success_chunk(&format!("Index '{}' dropped from table '{}'", idx.index_name, idx.table_name)))
        }
        LogicalOperator::CreateVectorIndex(vi) => {
            Ok(ddl_success_chunk(&format!(
                "Vector index '{}' on '{}' will be created at connection level",
                vi.index_name, vi.table_name
            )))
        }
        LogicalOperator::CreateSequence(_)
        | LogicalOperator::DropSequence(_)
        | LogicalOperator::CreateDml(_)
        | LogicalOperator::ExportDatabase(_)
        | LogicalOperator::ImportDatabase(_) => {
            Ok(vec![DataChunk {
                fields: vec![],
                field_types: vec![],
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

/// Build a success DataChunk with a single message column.
fn ddl_success_chunk(message: &str) -> Vec<DataChunk> {
    let mut v = kuzu_common::vector::ValueVector::new(
        kuzu_common::types::PhysicalTypeID::String, 1,
    );
    v.resize(1);
    v.set_value(0, &kuzu_common::types::Value::String(message.to_string())).unwrap();
    let arr = kuzu_common::arrow_vector::ArrowVector::from_legacy(&v).array;
    let mut chunk = DataChunk::new(vec![arr], vec![kuzu_common::types::PhysicalTypeID::String]);
    chunk.size = 1;
    chunk.field_names = vec!["result".to_string()];
    vec![chunk]
}

/// Minimal type parser for ALTER TABLE ADD COLUMN (avoids kuzu-binder dependency).
fn parse_type_simple(type_name: &str) -> Result<kuzu_common::types::LogicalTypeID, String> {
    let upper = type_name.trim().to_uppercase();
    match upper.as_str() {
        "BOOL" | "BOOLEAN" => Ok(kuzu_common::types::LogicalTypeID::Bool),
        "INT64" => Ok(kuzu_common::types::LogicalTypeID::Int64),
        "INT32" => Ok(kuzu_common::types::LogicalTypeID::Int32),
        "INT16" => Ok(kuzu_common::types::LogicalTypeID::Int16),
        "INT8" => Ok(kuzu_common::types::LogicalTypeID::Int8),
        "UINT64" => Ok(kuzu_common::types::LogicalTypeID::UInt64),
        "UINT32" => Ok(kuzu_common::types::LogicalTypeID::UInt32),
        "UINT16" => Ok(kuzu_common::types::LogicalTypeID::UInt16),
        "UINT8" => Ok(kuzu_common::types::LogicalTypeID::UInt8),
        "DOUBLE" => Ok(kuzu_common::types::LogicalTypeID::Double),
        "FLOAT" => Ok(kuzu_common::types::LogicalTypeID::Float),
        "STRING" => Ok(kuzu_common::types::LogicalTypeID::String),
        "BLOB" => Ok(kuzu_common::types::LogicalTypeID::Blob),
        "DATE" => Ok(kuzu_common::types::LogicalTypeID::Date),
        "TIMESTAMP" => Ok(kuzu_common::types::LogicalTypeID::Timestamp),
        "INTERVAL" => Ok(kuzu_common::types::LogicalTypeID::Interval),
        "UUID" => Ok(kuzu_common::types::LogicalTypeID::Uuid),
        _ => Err(format!("Unknown type '{type_name}'")),
    }
}
