use super::ExecutionContext;
use crate::physical_operator::*;
use crate::processor::SchemaDdlOp;
use crate::processor::plan_serializer::serialize_plan_tree;
use akar_common::error::ProcessorError;
use akar_common::vector::DataChunk;
use akar_planner::logical_operator::LogicalOperator;

pub fn map_and_execute_ddl(
    op: &LogicalOperator,
    current_input: Vec<DataChunk>,
    ctx: &mut ExecutionContext,
) -> Result<Vec<DataChunk>, ProcessorError> {
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
                ).into())
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
            let tc = ctx
                .table_catalog
                .as_ref()
                .ok_or("CREATE NODE TABLE requires a table catalog")?;
            let columns: Vec<akar_storage::table::ColumnDefinition> = c
                .columns
                .iter()
                .map(|col| akar_storage::table::ColumnDefinition {
                    name: col.name.clone(),
                    logical_type: col.logical_type,
                    is_primary_key: col.is_primary_key,
                    compression: col.compression,
                })
                .collect();
            tc.create_node_table(c.name.clone(), columns);

            // Auto-create ART index for primary key (matches connection/ddl.rs behavior)
            if c.columns.iter().any(|col| col.is_primary_key) {
                let index_name = format!("{}_pk_idx", c.name);
                tc.create_art_index(&c.name, &index_name).map_err(|e| {
                    format!("Failed to auto-create ART PK index for table '{}': {e}", c.name)
                })?;
            }

            tracing::info!("Pipeline: Created node table '{}'", c.name);
            Ok(ddl_success_chunk(&format!("Node table '{}' created", c.name)))
        }
        LogicalOperator::CreateRelTable(c) => {
            let tc = ctx
                .table_catalog
                .as_ref()
                .ok_or("CREATE REL TABLE requires a table catalog")?;
            let from_id = tc
                .get_node_table_by_name(&c.from)
                .map(|t| t.table_id)
                .ok_or_else(|| format!("From table '{}' not found", c.from))?;
            let to_id = tc
                .get_node_table_by_name(&c.to)
                .map(|t| t.table_id)
                .ok_or_else(|| format!("To table '{}' not found", c.to))?;
            let columns: Vec<akar_storage::table::ColumnDefinition> = c
                .columns
                .iter()
                .map(|col| akar_storage::table::ColumnDefinition {
                    name: col.name.clone(),
                    logical_type: col.logical_type,
                    is_primary_key: col.is_primary_key,
                    compression: col.compression,
                })
                .collect();
            tc.create_rel_table(c.name.clone(), from_id, to_id, columns);
            tracing::info!("Pipeline: Created rel table '{}' ({} -> {})", c.name, c.from, c.to);
            Ok(ddl_success_chunk(&format!("Rel table '{}' created", c.name)))
        }
        LogicalOperator::DropTable(d) => {
            let tc = ctx
                .table_catalog
                .as_ref()
                .ok_or("DROP TABLE requires a table catalog")?;
            let dropped = tc.drop_node_table(&d.name) || tc.drop_rel_table(&d.name);
            if dropped {
                tracing::info!("Pipeline: Dropped table '{}'", d.name);
                Ok(ddl_success_chunk(&format!("Table '{}' dropped", d.name)))
            } else {
                Err(format!("Table '{}' not found", d.name).into())
            }
        }
        LogicalOperator::AlterTable(a) => {
            let tc = ctx
                .table_catalog
                .as_ref()
                .ok_or("ALTER TABLE requires a table catalog")?;
            match &a.action {
                akar_parser::ast::AlterAction::AddColumn { name, type_name } => {
                    let logical_type = parse_type_simple(type_name)?;
                    let mut table = tc
                        .get_node_table_by_name_mut(&a.table_name)
                        .ok_or_else(|| format!("Table '{}' not found", a.table_name))?;
                    if table.columns.iter().any(|c| c.name.eq_ignore_ascii_case(name)) {
                        return Err(format!("Column '{}' already exists in '{}'", name, a.table_name).into());
                    }
                    table.columns.push(akar_storage::table::ColumnDefinition {
                        name: name.clone(),
                        logical_type,
                        is_primary_key: false,
                        compression: akar_common::enums::CompressionType::Uncompressed,
                    });
                    tracing::info!("Pipeline: Added column '{}' to '{}'", name, a.table_name);
                    Ok(ddl_success_chunk(&format!(
                        "Column '{}' added to table '{}'",
                        name, a.table_name
                    )))
                }
                akar_parser::ast::AlterAction::DropColumn { name } => {
                    let mut table = tc
                        .get_node_table_by_name_mut(&a.table_name)
                        .ok_or_else(|| format!("Table '{}' not found", a.table_name))?;
                    let pos = table
                        .columns
                        .iter()
                        .position(|c| c.name == *name)
                        .ok_or_else(|| format!("Column '{}' not found in '{}'", name, a.table_name))?;
                    if table.columns[pos].is_primary_key {
                        return Err(format!("Cannot drop primary key column '{}'", name).into());
                    }
                    table.columns.remove(pos);
                    tracing::info!("Pipeline: Dropped column '{}' from '{}'", name, a.table_name);
                    Ok(ddl_success_chunk(&format!(
                        "Column '{}' dropped from table '{}'",
                        name, a.table_name
                    )))
                }
                akar_parser::ast::AlterAction::RenameColumn { old_name, new_name } => {
                    {
                        let table = tc
                            .get_node_table_by_name(&a.table_name)
                            .ok_or_else(|| format!("Table '{}' not found", a.table_name))?;
                        if !table.columns.iter().any(|c| c.name == *old_name) {
                            return Err(format!("Column '{}' not found in '{}'", old_name, a.table_name).into());
                        }
                        if table.columns.iter().any(|c| c.name == *new_name) {
                            return Err(format!("Column '{}' already exists in '{}'", new_name, a.table_name).into());
                        }
                    }
                    let mut table = tc.get_node_table_by_name_mut(&a.table_name).unwrap();
                    let col = table.columns.iter_mut().find(|c| c.name == *old_name).unwrap();
                    col.name = new_name.clone();
                    tracing::info!(
                        "Pipeline: Renamed column '{}' to '{}' in '{}'",
                        old_name,
                        new_name,
                        a.table_name
                    );
                    Ok(ddl_success_chunk(&format!(
                        "Column '{}' renamed to '{}' in table '{}'",
                        old_name, new_name, a.table_name
                    )))
                }
                akar_parser::ast::AlterAction::RenameTable { new_name } => {
                    if tc.get_node_table_by_name(new_name).is_some() || tc.get_rel_table_by_name(new_name).is_some() {
                        return Err(format!("Table '{}' already exists", new_name).into());
                    }
                    if let Some(mut table) = tc.get_node_table_by_name_mut(&a.table_name) {
                        table.name = new_name.clone();
                    } else if let Some(mut table) = tc.get_rel_table_by_name_mut(&a.table_name) {
                        table.name = new_name.clone();
                    } else {
                        return Err(format!("Table '{}' not found", a.table_name).into());
                    }
                    tracing::info!("Pipeline: Renamed table '{}' to '{}'", a.table_name, new_name);
                    Ok(ddl_success_chunk(&format!(
                        "Table '{}' renamed to '{}'",
                        a.table_name, new_name
                    )))
                }
            }
        }
        LogicalOperator::CreateIndex(idx) => {
            let tc = ctx
                .table_catalog
                .as_ref()
                .ok_or("CREATE INDEX requires a table catalog")?;
            tc.create_art_index(&idx.table_name, &idx.index_name)?;
            tracing::info!(
                "Pipeline: Created ART index '{}' on '{}'",
                idx.index_name,
                idx.table_name
            );
            Ok(ddl_success_chunk(&format!(
                "ART index '{}' created on table '{}'",
                idx.index_name, idx.table_name
            )))
        }
        LogicalOperator::DropIndex(idx) => {
            let tc = ctx
                .table_catalog
                .as_ref()
                .ok_or("DROP INDEX requires a table catalog")?;
            tc.drop_art_index(&idx.table_name)?;
            tracing::info!("Pipeline: Dropped index '{}' from '{}'", idx.index_name, idx.table_name);
            Ok(ddl_success_chunk(&format!(
                "Index '{}' dropped from table '{}'",
                idx.index_name, idx.table_name
            )))
        }
        LogicalOperator::CreateVectorIndex(vi) => {
            let tc = ctx
                .table_catalog
                .as_ref()
                .ok_or("CREATE VECTOR INDEX requires a table catalog")?;

            // Map string metric to typed enum
            let metric = match vi.metric.to_lowercase().as_str() {
                "cosine" => akar_vector::hnsw::DistanceMetric::Cosine,
                "euclidean" | "l2" => akar_vector::hnsw::DistanceMetric::L2Squared,
                "dot" => akar_vector::hnsw::DistanceMetric::DotProduct,
                other => return Err(format!("Unknown vector metric '{other}'").into()),
            };

            // Create the vector index in storage
            tc.create_vector_index(
                vi.index_name.clone(),
                vi.table_name.clone(),
                vi.column_name.clone(),
                metric,
                vi.dimensions as u32,
            );

            // Auto-populate from existing table data
            if let Some(table) = tc.get_node_table_by_name(&vi.table_name) {
                let col_idx = table.columns.iter().position(|c| c.name == vi.column_name);
                if let Some(col_idx) = col_idx {
                    for row_id in 0..table.num_rows as usize {
                        if let Some(val) = table.get_value(row_id, col_idx) {
                            if let Ok(vec) = akar_storage::extract_f64_list_from_value(val) {
                                if let Some(mut vib) = tc.get_vector_index_by_name_mut(&vi.index_name) {
                                    vib.hnsw_mut().insert(vec, row_id);
                                }
                            }
                        }
                    }
                }
            }

            tracing::info!(
                "Pipeline: Created vector index '{}' on '{}.{}'",
                vi.index_name,
                vi.table_name,
                vi.column_name
            );
            Ok(ddl_success_chunk(&format!(
                "Vector index '{}' created on '{}.{}'",
                vi.index_name, vi.table_name, vi.column_name
            )))
        }
        LogicalOperator::CreateSequence(s) => {
            if let Some(ref ddl_fn) = ctx.schema_ddl_fn {
                let result = ddl_fn(SchemaDdlOp::CreateSequence {
                    name: s.name.clone(),
                    if_not_exists: s.if_not_exists,
                    start_value: s.start_with,
                    increment: s.increment,
                    min_value: s.min_value,
                    max_value: s.max_value,
                    cycle: s.cycle,
                })?;
                Ok(ddl_success_chunk(&result))
            } else {
                Err("CREATE SEQUENCE requires schema catalog access".into())
            }
        }
        LogicalOperator::DropSequence(s) => {
            if let Some(ref ddl_fn) = ctx.schema_ddl_fn {
                let result = ddl_fn(SchemaDdlOp::DropSequence {
                    name: s.name.clone(),
                    if_exists: s.if_exists,
                })?;
                Ok(ddl_success_chunk(&result))
            } else {
                Err("DROP SEQUENCE requires schema catalog access".into())
            }
        }
        LogicalOperator::CreateDml(c) => {
            let tc = ctx
                .table_catalog
                .as_ref()
                .ok_or("CREATE DML requires a table catalog")?;
            let mut table = tc
                .get_node_table_by_name_mut(&c.table_name)
                .ok_or_else(|| format!("Table '{}' not found", c.table_name))?;

            // Build values from pattern properties, defaulting to Null
            let mut values: Vec<akar_common::types::Value> =
                table.columns.iter().map(|_| akar_common::types::Value::Null).collect();
            {
                let registry = ctx
                    .function_registry
                    .clone()
                    .ok_or("CREATE DML requires a function registry")?;
                let registry = registry.lock().map_err(|e| format!("Lock poisoned: {e}"))?;
                for (prop_name, expr) in &c.properties {
                    if let Some(col_idx) = table.columns.iter().position(|col| col.name == *prop_name) {
                        values[col_idx] =
                            crate::physical::write_ops::set::evaluate_constant_expr(expr, &registry);
                    }
                }
            }

            table.insert_row(values)?;
            tracing::info!("Pipeline: Created node in '{}'", c.table_name);
            Ok(ddl_success_chunk(&format!("Created node in '{}'", c.table_name)))
        }
        LogicalOperator::ExportDatabase(e) => {
            if let Some(ref ddl_fn) = ctx.schema_ddl_fn {
                let result = ddl_fn(SchemaDdlOp::ExportDatabase {
                    file_path: e.file_path.clone(),
                    file_type: e.file_type.clone(),
                    schema_only: e.schema_only,
                })?;
                Ok(ddl_success_chunk(&result))
            } else {
                Err("EXPORT DATABASE requires schema catalog access".into())
            }
        }
        LogicalOperator::ImportDatabase(i) => {
            if let Some(ref ddl_fn) = ctx.schema_ddl_fn {
                let result = ddl_fn(SchemaDdlOp::ImportDatabase {
                    file_path: i.file_path.clone(),
                    query: i.query.clone(),
                    index_query: i.index_query.clone(),
                })?;
                Ok(ddl_success_chunk(&result))
            } else {
                Err("IMPORT DATABASE requires schema catalog access".into())
            }
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
        _ => Err(format!("DDL operator not implemented in mapper: {:?}", op).into()),
    }
}

/// Build a success DataChunk with a single message column.
fn ddl_success_chunk(message: &str) -> Vec<DataChunk> {
    let mut v = akar_common::vector::ValueVector::new(akar_common::types::PhysicalTypeID::String, 1);
    v.resize(1);
    v.set_value(0, &akar_common::types::Value::String(message.to_string()))
        .unwrap();
    let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&v).array;
    let mut chunk = DataChunk::new(vec![arr], vec![akar_common::types::PhysicalTypeID::String]);
    chunk.size = 1;
    chunk.field_names = vec!["result".to_string()];
    vec![chunk]
}

/// Minimal type parser for ALTER TABLE ADD COLUMN (avoids Akar-binder dependency).
fn parse_type_simple(type_name: &str) -> Result<akar_common::types::LogicalTypeID, ProcessorError> {
    let upper = type_name.trim().to_uppercase();
    match upper.as_str() {
        "BOOL" | "BOOLEAN" => Ok(akar_common::types::LogicalTypeID::Bool),
        "INT64" => Ok(akar_common::types::LogicalTypeID::Int64),
        "INT32" => Ok(akar_common::types::LogicalTypeID::Int32),
        "INT16" => Ok(akar_common::types::LogicalTypeID::Int16),
        "INT8" => Ok(akar_common::types::LogicalTypeID::Int8),
        "UINT64" => Ok(akar_common::types::LogicalTypeID::UInt64),
        "UINT32" => Ok(akar_common::types::LogicalTypeID::UInt32),
        "UINT16" => Ok(akar_common::types::LogicalTypeID::UInt16),
        "UINT8" => Ok(akar_common::types::LogicalTypeID::UInt8),
        "DOUBLE" => Ok(akar_common::types::LogicalTypeID::Double),
        "FLOAT" => Ok(akar_common::types::LogicalTypeID::Float),
        "STRING" => Ok(akar_common::types::LogicalTypeID::String),
        "BLOB" => Ok(akar_common::types::LogicalTypeID::Blob),
        "DATE" => Ok(akar_common::types::LogicalTypeID::Date),
        "TIMESTAMP" => Ok(akar_common::types::LogicalTypeID::Timestamp),
        "INTERVAL" => Ok(akar_common::types::LogicalTypeID::Interval),
        "UUID" => Ok(akar_common::types::LogicalTypeID::Uuid),
        _ => Err(format!("Unknown type '{type_name}'").into()),
    }
}
