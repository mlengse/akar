use super::Connection;
use super::utils::{ast_constant_to_value, pk_value_to_string, value_to_csv_string};
use crate::query_result::QueryResult;
use akar_binder::bound_statement::BoundStatement;
use akar_common::types::Value;

impl Connection {
    /// Handle DDL statements by checking the bound statement type.
    /// Returns `Ok(Some(result))` if DDL, `Ok(None)` if DML (continue).
    pub(crate) fn handle_ddl(&self, bound: &BoundStatement) -> Result<Option<QueryResult>, String> {
        match bound {
            BoundStatement::BoundStandaloneCall(_) => {
                // StandaloneCall is executed via QueryProcessor pipeline
                Ok(None)
            }
            BoundStatement::BoundExplain(_) => {
                // EXPLAIN is handled by the query processor pipeline
                Ok(None)
            }
            BoundStatement::BoundTransaction(t) => match t.action {
                akar_parser::ast::TransactionAction::Begin => {
                    let txn = self.begin_write_txn()?;
                    Ok(Some(QueryResult::success_message(format!(
                        "Transaction started (txn#{})",
                        txn.transaction_id
                    ))))
                }
                akar_parser::ast::TransactionAction::Commit => {
                    let txn_ids: Vec<u64> = self
                        .txn_resources
                        .lock()
                        .map_err(|e| format!("Lock: {e}"))?
                        .keys()
                        .copied()
                        .collect();
                    if txn_ids.is_empty() {
                        return Err("No active transaction to commit".into());
                    }
                    let tm = &self.database.transaction_manager;
                    if let Ok(mut active) = tm.active_snapshot() {
                        for txn_id in &txn_ids {
                            if let Some(txn) = active.remove(txn_id) {
                                let mut t = txn;
                                self.commit_write_txn(&mut t)?;
                                return Ok(Some(QueryResult::success_message("Transaction committed".into())));
                            }
                        }
                    }
                    Err("No active transaction to commit".into())
                }
                akar_parser::ast::TransactionAction::Rollback => {
                    let txn_ids: Vec<u64> = self
                        .txn_resources
                        .lock()
                        .map_err(|e| format!("Lock: {e}"))?
                        .keys()
                        .copied()
                        .collect();
                    if txn_ids.is_empty() {
                        return Err("No active transaction to rollback".into());
                    }
                    let tm = &self.database.transaction_manager;
                    if let Ok(mut active) = tm.active_snapshot() {
                        for txn_id in &txn_ids {
                            if let Some(mut txn) = active.remove(txn_id) {
                                self.rollback_write_txn(&mut txn)?;
                                return Ok(Some(QueryResult::success_message("Transaction rolled back".into())));
                            }
                        }
                    }
                    Err("No active transaction to rollback".into())
                }
                akar_parser::ast::TransactionAction::Checkpoint => {
                    self.do_sync_checkpoint()?;
                    Ok(Some(QueryResult::success_message("Checkpoint completed".into())))
                }
            },
            BoundStatement::BoundExtension(e) => {
                let msg = match e.action {
                    akar_parser::ast::ExtensionAction::Load => {
                        format!(
                            "Extension '{}': Extensions are compile-time features in Akar Rust. \
                             Rebuild with --features {}-extension to enable.",
                            e.name,
                            e.name.to_lowercase()
                        )
                    }
                    akar_parser::ast::ExtensionAction::Install => {
                        format!(
                            "INSTALL EXTENSION '{}' is not yet supported in Akar Rust. \
                             Extensions are compile-time features; rebuild with --features {}-extension.",
                            e.name,
                            e.name.to_lowercase()
                        )
                    }
                    akar_parser::ast::ExtensionAction::Uninstall => {
                        format!(
                            "UNINSTALL EXTENSION '{}': Extensions are compile-time features in Akar Rust. \
                             Rebuild without the feature flag to disable.",
                            e.name
                        )
                    }
                };
                Ok(Some(QueryResult::success_message(msg)))
            }
            BoundStatement::BoundAttachDatabase(a) => {
                // Register a foreign table entry in the catalog
                let mut catalog = self
                    .database
                    .catalog
                    .lock()
                    .map_err(|e| format!("Lock poisoned: {e}"))?;
                let table_id = catalog.next_table_id();
                let entry = akar_catalog::ForeignTableEntry {
                    table_id,
                    name: a.alias.clone(),
                    columns: Vec::new(),
                    source_type: a
                        .options
                        .get("source_type")
                        .cloned()
                        .unwrap_or_else(|| "unknown".into()),
                };
                catalog.add_foreign_entry(entry);
                Ok(Some(QueryResult::success_message(format!(
                    "Database attached as '{}' from '{}'",
                    a.alias, a.path
                ))))
            }
            BoundStatement::BoundDetachDatabase(d) => {
                let mut catalog = self
                    .database
                    .catalog
                    .lock()
                    .map_err(|e| format!("Lock poisoned: {e}"))?;
                catalog.remove_foreign_entry(&d.alias).map_err(|e| e.to_string())?;
                Ok(Some(QueryResult::success_message(format!(
                    "Database '{}' detached",
                    d.alias
                ))))
            }
            BoundStatement::BoundUseDatabase(u) => {
                // USE DATABASE — switch default schema context
                tracing::info!("USE DATABASE '{}'", u.alias);
                Ok(Some(QueryResult::success_message(format!(
                    "Using database '{}' (schema switching will be available in a future release)",
                    u.alias
                ))))
            }
            BoundStatement::BoundLoadFrom(l) => {
                // LOAD FROM scans external file — delegate to COPY FROM path
                let format = l.options.get("format").cloned().unwrap_or_else(|| "csv".into());
                Ok(Some(QueryResult::success_message(format!(
                    "LOAD FROM '{}' (format={}): external scan will be available in a future release",
                    l.path, format
                ))))
            }
            BoundStatement::BoundCreateType(t) => {
                // Register the type alias in the catalog
                let _catalog = self.database.catalog.lock().map_err(|e| format!("Lock error: {e}"))?;
                // Types are stored as catalog entries — for now, just a placeholder
                tracing::info!("CREATE TYPE '{}' AS '{}'", t.name, t.type_name);
                Ok(Some(QueryResult::success_message(format!(
                    "Type '{}' created (type: {})",
                    t.name, t.type_name
                ))))
            }
            BoundStatement::BoundCommentOnTable(c) => {
                // Store the comment as metadata — for now, just acknowledge
                tracing::info!("COMMENT ON TABLE '{}' IS '{}'", c.table_name, c.comment);
                Ok(Some(QueryResult::success_message(format!(
                    "Comment set on table '{}'",
                    c.table_name
                ))))
            }
            BoundStatement::BoundCreateGraph(g) => {
                let mut cat = self.database.catalog.lock().map_err(|e| format!("Lock error: {e}"))?;
                let info = akar_catalog::ProjectedGraphInfo {
                    name: g.name.clone(),
                    entry_type: "NATIVE".into(),
                    cypher_query: None,
                };
                cat.create_projected_graph(info).map_err(|e| e.to_string())?;
                tracing::info!("CREATE GRAPH '{}' (any={})", g.name, g.is_any);
                Ok(Some(QueryResult::success_message(format!(
                    "Graph '{}' created",
                    g.name
                ))))
            }
            BoundStatement::BoundUseGraph(g) => {
                tracing::info!("USE GRAPH '{}'", g.name);
                Ok(Some(QueryResult::success_message(format!("Using graph '{}'", g.name))))
            }
            BoundStatement::BoundDropGraph(g) => {
                let mut cat = self.database.catalog.lock().map_err(|e| format!("Lock error: {e}"))?;
                cat.drop_projected_graph(&g.name).map_err(|e| e.to_string())?;
                tracing::info!("DROP GRAPH '{}'", g.name);
                Ok(Some(QueryResult::success_message(format!(
                    "Graph '{}' dropped",
                    g.name
                ))))
            }
            BoundStatement::BoundCopyTo(c) => {
                // Execute the inner query
                let inner_bound = BoundStatement::BoundQuery(c.query.clone());
                let result = self.execute_query_inner(&inner_bound, None, None)?;

                // Write results to file
                let path = std::path::Path::new(&c.file_path);
                match c.format {
                    akar_parser::ast::CopyToFormat::Csv => {
                        let mut w = csv::WriterBuilder::new()
                            .has_headers(c.header)
                            .from_path(path)
                            .map_err(|e| format!("Cannot create file '{}': {}", c.file_path, e))?;

                        // Write header
                        if c.header {
                            if let Some(first_chunk) = result.chunks.first() {
                                let header: Vec<String> = if first_chunk.field_names.is_empty() {
                                    (0..first_chunk.num_fields()).map(|i| format!("column_{}", i)).collect()
                                } else {
                                    first_chunk.field_names.clone()
                                };
                                if !header.is_empty() {
                                    w.write_record(&header).map_err(|e| format!("CSV write error: {e}"))?;
                                }
                            }
                        }

                        // Write rows
                        for chunk in &result.chunks {
                            for row in 0..chunk.size {
                                let row_values: Vec<String> = (0..chunk.fields.len())
                                    .map(|col_idx| {
                                        chunk
                                            .get_value(col_idx, row)
                                            .map(|v| value_to_csv_string(&v))
                                            .unwrap_or_default()
                                    })
                                    .collect();
                                w.write_record(&row_values)
                                    .map_err(|e| format!("CSV write error: {e}"))?;
                            }
                        }

                        w.flush().map_err(|e| format!("CSV flush error: {e}"))?;
                    }
                    akar_parser::ast::CopyToFormat::Parquet => {
                        #[cfg(feature = "parquet-export")]
                        {
                            self.write_parquet(&c.file_path, &result, c.header)
                                .map_err(|e| format!("Parquet export error: {e}"))?;
                        }
                        #[cfg(not(feature = "parquet-export"))]
                        {
                            return Err("Parquet export requires 'parquet-export' feature. \
                                 Build with: cargo build --features parquet-export"
                                .into());
                        }
                    }
                }
                Ok(Some(QueryResult::success_message(format!(
                    "COPY TO '{}' completed, {} rows exported",
                    c.file_path, result.num_rows
                ))))
            }
            BoundStatement::BoundCreateNodeTable(t) => {
                let columns: Vec<akar_catalog::CatalogColumn> = t
                    .columns
                    .iter()
                    .map(|c| akar_catalog::CatalogColumn {
                        name: c.name.clone(),
                        logical_type: c.logical_type,
                        is_primary_key: c.is_primary_key,
                        compression: akar_common::enums::CompressionType::Uncompressed,
                        default_value: None,
                    })
                    .collect();
                self.database.create_node_table(t.name.clone(), columns)?;
                Ok(Some(QueryResult::success_message(format!(
                    "Node table '{}' created",
                    t.name
                ))))
            }
            BoundStatement::BoundCreateRelTable(t) => {
                let columns: Vec<akar_catalog::CatalogColumn> = t
                    .columns
                    .iter()
                    .map(|c| akar_catalog::CatalogColumn {
                        name: c.name.clone(),
                        logical_type: c.logical_type,
                        is_primary_key: c.is_primary_key,
                        compression: akar_common::enums::CompressionType::Uncompressed,
                        default_value: None,
                    })
                    .collect();
                let src_id = t.src_table_id;
                let dst_id = t.dst_table_id;
                self.database
                    .create_rel_table(t.name.clone(), src_id, dst_id, columns)?;
                Ok(Some(QueryResult::success_message(format!(
                    "Rel table '{}' created",
                    t.name
                ))))
            }
            BoundStatement::BoundDropTable(t) => {
                self.database.drop_table(&t.name)?;
                Ok(Some(QueryResult::success_message(format!(
                    "Table '{}' dropped",
                    t.name
                ))))
            }
            #[cfg(feature = "vector-extension")]
            BoundStatement::BoundCreateVectorIndex(idx) => {
                let metric = match idx.metric.to_lowercase().as_str() {
                    "cosine" => akar_vector::hnsw::DistanceMetric::Cosine,
                    "euclidean" => akar_vector::hnsw::DistanceMetric::Euclidean,
                    "l2" => akar_vector::hnsw::DistanceMetric::L2Squared,
                    "dot" => akar_vector::hnsw::DistanceMetric::DotProduct,
                    other => return Err(format!("Unknown metric '{other}'")),
                };
                self.database.create_vector_index(
                    idx.index_name.clone(),
                    idx.table_name.clone(),
                    idx.column_name.clone(),
                    metric,
                    idx.dimensions as u32,
                )?;
                Ok(Some(QueryResult::success_message(format!(
                    "Vector index '{}' created",
                    idx.index_name
                ))))
            }
            #[cfg(not(feature = "vector-extension"))]
            BoundStatement::BoundCreateVectorIndex(idx) => Err(format!(
                "Vector extension not enabled. Enable the 'vector-extension' feature to use CREATE VECTOR INDEX. Index '{}' not created.",
                idx.index_name
            )),
            BoundStatement::BoundUnion(u) => {
                tracing::info!("UNION ALL query");
                let planner = akar_planner::QueryPlanner::new();
                let optimizer = akar_optimizer::Optimizer::with_stats(self.database.stats_store.clone());

                // Execute left side
                let left_plan = planner
                    .plan(BoundStatement::BoundQuery(*u.left.clone()))
                    .map_err(|e| format!("Plan left UNION: {e}"))?;
                let left_optimized = optimizer.optimize(left_plan);
                let processor = self.create_processor();
                let left_chunks = processor
                    .execute(&left_optimized)
                    .map_err(|e| format!("Execute left UNION: {e}"))?;

                // Execute right side
                let right_plan = planner
                    .plan(BoundStatement::BoundQuery(*u.right.clone()))
                    .map_err(|e| format!("Plan right UNION: {e}"))?;
                let right_optimized = optimizer.optimize(right_plan);
                let processor = self.create_processor();
                let right_chunks = processor
                    .execute(&right_optimized)
                    .map_err(|e| format!("Execute right UNION: {e}"))?;

                // Concatenate results
                let mut all_chunks = left_chunks;
                all_chunks.extend(right_chunks);
                Ok(Some(QueryResult::new(all_chunks)))
            }
            BoundStatement::BoundAlterTable(a) => {
                tracing::info!("ALTER TABLE '{}'", a.table_name);
                let mut catalog = self
                    .database
                    .catalog
                    .lock()
                    .map_err(|e| format!("Lock poisoned: {e}"))?;
                match &a.action {
                    akar_parser::ast::AlterAction::AddColumn { name, type_name } => {
                        let logical_type =
                            akar_binder::Binder::parse_type(type_name).map_err(|e| format!("ALTER ADD: {e}"))?;
                        catalog
                            .add_column(
                                &a.table_name,
                                akar_catalog::CatalogColumn {
                                    name: name.clone(),
                                    logical_type,
                                    is_primary_key: false,
                                    compression: akar_common::enums::CompressionType::Uncompressed,
                                    default_value: None,
                                },
                            )
                            .map_err(|e| format!("ALTER ADD: {e}"))?;
                        Ok(Some(QueryResult::success_message(format!(
                            "Column '{}' added to table '{}'",
                            name, a.table_name
                        ))))
                    }
                    akar_parser::ast::AlterAction::DropColumn { name } => {
                        catalog
                            .drop_column(&a.table_name, name)
                            .map_err(|e| format!("ALTER DROP: {e}"))?;
                        Ok(Some(QueryResult::success_message(format!(
                            "Column '{}' dropped from table '{}'",
                            name, a.table_name
                        ))))
                    }
                    akar_parser::ast::AlterAction::RenameColumn { old_name, new_name } => {
                        catalog
                            .rename_column(&a.table_name, old_name, new_name)
                            .map_err(|e| format!("ALTER RENAME COLUMN: {e}"))?;
                        Ok(Some(QueryResult::success_message(format!(
                            "Column '{}' renamed to '{}' in table '{}'",
                            old_name, new_name, a.table_name
                        ))))
                    }
                    akar_parser::ast::AlterAction::RenameTable { new_name } => {
                        catalog
                            .rename_table(&a.table_name, new_name)
                            .map_err(|e| format!("ALTER RENAME TABLE: {e}"))?;
                        Ok(Some(QueryResult::success_message(format!(
                            "Table '{}' renamed to '{}'",
                            a.table_name, new_name
                        ))))
                    }
                }
            }
            BoundStatement::BoundCopyFrom(c) => {
                tracing::info!("COPY FROM '{}' from '{}'", c.table_name, c.file_path);
                // Fall through to the pipeline (plan → optimize → execute)
                Ok(None)
            }
            BoundStatement::BoundCreateDml(c) => {
                tracing::info!("CREATE DML into '{}'", c.table_name);

                let catalog = self.database.table_catalog();
                let mut table = catalog
                    .get_node_table_by_name_mut(&c.table_name)
                    .ok_or_else(|| format!("Table '{}' not found in storage", c.table_name))?;

                // Build values from pattern properties, defaulting to Null
                let mut values: Vec<Value> = table.columns.iter().map(|_| Value::Null).collect();
                {
                    let registry = self
                        .database
                        .function_registry
                        .lock()
                        .map_err(|e| format!("Lock poisoned: {e}"))?;
                    for (prop_name, expr) in &c.properties {
                        if let Some(col_idx) = table.columns.iter().position(|c| c.name == *prop_name) {
                            values[col_idx] =
                                akar_processor::physical::write_ops::set::evaluate_constant_expr(expr, &registry);
                        }
                    }
                }

                // Auto-generate SERIAL column values for null entries
                {
                    let mut sys_catalog = self
                        .database
                        .catalog
                        .lock()
                        .map_err(|e| format!("Lock poisoned: {e}"))?;
                    for (col_idx, col) in table.columns.iter().enumerate() {
                        if col.logical_type == akar_common::types::LogicalTypeID::Serial
                            && matches!(values[col_idx], Value::Null)
                        {
                            let seq_name = akar_catalog::SequenceEntry::get_serial_name(&c.table_name, &col.name);
                            if let Some(seq) = sys_catalog.get_sequence_mut(&seq_name) {
                                let next_val = seq.next_k_val(1);
                                values[col_idx] = Value::Int64(next_val);
                            }
                        }
                    }
                }

                let row_id = table.num_rows as usize;
                table.insert_row(values)?;

                // Auto-populate vector indexes on this table
                let vec_indexes_on_table: Vec<(String, String)> = catalog
                    .all_vector_indexes()
                    .iter()
                    .filter(|entry| entry.table_name == c.table_name)
                    .map(|entry| (entry.name.clone(), entry.column_name.clone()))
                    .collect();

                for (index_name, col_name) in &vec_indexes_on_table {
                    if let Some(col_idx) = table.columns.iter().position(|c| c.name == *col_name)
                        && let Some(val) = table.get_value(row_id, col_idx)
                        && let Ok(vec) = akar_storage::extract_f64_list_from_value(val)
                        && let Some(mut vi) = catalog.get_vector_index_by_name_mut(index_name)
                    {
                        vi.hnsw_mut().insert(vec, row_id);
                        tracing::debug!("Auto-populated vector index '{}' with row {}", index_name, row_id);
                    }
                }

                Ok(Some(QueryResult::success_message(format!(
                    "Created node in '{}'",
                    c.table_name
                ))))
            }
            BoundStatement::BoundMerge(m) => {
                tracing::info!("MERGE into '{}'", m.table_name);

                // Get the table — DashMap handles locking internally
                let catalog = self.database.table_catalog();
                let mut table = catalog
                    .get_node_table_by_name_mut(&m.table_name)
                    .ok_or_else(|| format!("Table '{}' not found in storage", m.table_name))?;

                // Evaluate the PK properties from the MERGE pattern
                let pk_col_idx = table.primary_key_column;
                let pk_prop = m
                    .properties
                    .iter()
                    .find(|(name, _)| table.columns.get(pk_col_idx).map(|c| c.name == *name).unwrap_or(false));

                // Check if the node already exists by PK
                let pk_val = if let Some((_, expr)) = pk_prop {
                    if let akar_parser::ast::Expression::Constant(c) = expr {
                        Some(ast_constant_to_value(c))
                    } else {
                        None
                    }
                } else {
                    None
                };
                let exists = pk_val
                    .as_ref()
                    .map(|pv| table.hash_index.lookup(&pk_value_to_string(pv)).is_some())
                    .unwrap_or(false);

                if exists {
                    // Apply ON MATCH SET — lookup the row by the pattern's PK
                    // value, then update the SET target cell with the SET value.
                    let row = pk_val
                        .as_ref()
                        .and_then(|pv| table.hash_index.lookup(&pk_value_to_string(pv)));
                    for item in &m.on_match {
                        if let Some(row) = row
                            && let akar_parser::ast::Expression::Constant(c) = &item.value
                        {
                            let val = ast_constant_to_value(c);
                            let _ = table.update_cell(row, item.column_idx, val);
                        }
                    }
                    Ok(Some(QueryResult::success_message(format!(
                        "Matched existing node in '{}'",
                        m.table_name
                    ))))
                } else {
                    // Create new node with pattern properties + ON CREATE SET
                    let mut values: Vec<Value> = table.columns.iter().map(|_| Value::Null).collect();
                    for (prop_name, expr) in &m.properties {
                        if let Some(col_idx) = table.columns.iter().position(|c| c.name == *prop_name)
                            && let akar_parser::ast::Expression::Constant(c) = expr
                        {
                            values[col_idx] = ast_constant_to_value(c);
                        }
                    }
                    for item in &m.on_create {
                        if let akar_parser::ast::Expression::Constant(c) = &item.value {
                            let val = ast_constant_to_value(c);
                            if item.column_idx < values.len() {
                                values[item.column_idx] = val;
                            }
                        }
                    }
                    table.insert_row(values)?;
                    Ok(Some(QueryResult::success_message(format!(
                        "Created new node in '{}'",
                        m.table_name
                    ))))
                }
            }
            BoundStatement::BoundCreateIndex(idx) => {
                self.database.create_art_index(&idx.table_name, &idx.index_name)?;
                tracing::info!(
                    "Created '{}' index '{}' on '{}'",
                    idx.index_type.as_str(),
                    idx.index_name,
                    idx.table_name
                );
                Ok(Some(QueryResult::success_message(format!(
                    "{} index '{}' created on table '{}'",
                    idx.index_type.as_str(),
                    idx.index_name,
                    idx.table_name
                ))))
            }
            BoundStatement::BoundDropIndex(idx) => {
                self.database.drop_art_index(&idx.table_name, &idx.index_name)?;
                tracing::info!("Dropped index '{}' from '{}'", idx.index_name, idx.table_name);
                Ok(Some(QueryResult::success_message(format!(
                    "Index '{}' dropped from table '{}'",
                    idx.index_name, idx.table_name
                ))))
            }
            BoundStatement::BoundCreateSequence(s) => {
                let mut catalog = self
                    .database
                    .catalog
                    .lock()
                    .map_err(|e| format!("Lock poisoned: {e}"))?;
                let result = catalog.create_sequence(
                    s.name.clone(),
                    s.start_with,
                    s.increment,
                    s.min_value,
                    s.max_value,
                    s.cycle,
                );
                match result {
                    akar_catalog::CatalogResult::Created { .. } => {
                        tracing::info!("Created sequence '{}'", s.name);
                        Ok(Some(QueryResult::success_message(format!(
                            "Sequence '{}' created",
                            s.name
                        ))))
                    }
                    akar_catalog::CatalogResult::AlreadyExists => {
                        if s.if_not_exists {
                            Ok(Some(QueryResult::success_message(format!(
                                "Sequence '{}' already exists",
                                s.name
                            ))))
                        } else {
                            Err(format!("Sequence '{}' already exists", s.name))
                        }
                    }
                    _ => Err("Failed to create sequence".into()),
                }
            }
            BoundStatement::BoundDropSequence(s) => {
                let mut catalog = self
                    .database
                    .catalog
                    .lock()
                    .map_err(|e| format!("Lock poisoned: {e}"))?;
                let result = catalog.drop_sequence(&s.name);
                match result {
                    akar_catalog::CatalogResult::Dropped { .. } => {
                        tracing::info!("Dropped sequence '{}'", s.name);
                        Ok(Some(QueryResult::success_message(format!(
                            "Sequence '{}' dropped",
                            s.name
                        ))))
                    }
                    akar_catalog::CatalogResult::NotFound => {
                        if s.if_exists {
                            Ok(Some(QueryResult::success_message(format!(
                                "Sequence '{}' not found",
                                s.name
                            ))))
                        } else {
                            Err(format!("Sequence '{}' not found", s.name))
                        }
                    }
                    _ => Err("Failed to drop sequence".into()),
                }
            }
            BoundStatement::BoundCreateMacro(m) => {
                let mut catalog = self
                    .database
                    .catalog
                    .lock()
                    .map_err(|e| format!("Lock poisoned: {e}"))?;
                match catalog.create_macro(
                    m.name.clone(),
                    m.positional_args.clone(),
                    m.default_args.clone(),
                    m.expression.clone(),
                ) {
                    akar_catalog::CatalogResult::Created { .. } => {
                        tracing::info!("Created macro '{}'", m.name);
                        Ok(Some(QueryResult::success_message(format!(
                            "Macro '{}' created",
                            m.name
                        ))))
                    }
                    akar_catalog::CatalogResult::AlreadyExists => Err(format!("Macro '{}' already exists", m.name)),
                    _ => Err("Failed to create macro".into()),
                }
            }
            BoundStatement::BoundExportDatabase(e) => self.execute_export_database(e),
            BoundStatement::BoundImportDatabase(i) => self.execute_import_database(i),
            BoundStatement::BoundQuery(q) => {
                // Check if this is a FOREACH-only query — handle it directly
                if q.clauses.len() == 1
                    && let Some(akar_binder::bound_statement::BoundClause::BoundForeach(fc)) = q.clauses.first()
                {
                    return self.handle_foreach(fc);
                }
                Ok(None)
            }
            BoundStatement::BoundCreateFtsIndex(_) => Ok(None),
            BoundStatement::BoundAnalyze(a) => {
                tracing::info!("ANALYZE {} tables", a.table_ids.len());
                let mut stats = self
                    .database
                    .stats_store
                    .lock()
                    .map_err(|e| format!("Lock poisoned: {e}"))?;
                let catalog = self.database.table_catalog();

                for &table_id in &a.table_ids {
                    // Try node table first, then rel table
                    let node_table = catalog.get_node_table(table_id);
                    if let Some(table) = node_table {
                        let row_count = table.num_rows;
                        let mut columns = std::collections::HashMap::new();
                        for col_idx in 0..table.columns.len() {
                            let mut null_count: u64 = 0;
                            let mut distinct_set = std::collections::HashSet::new();
                            for row_idx in 0..row_count as usize {
                                if let Some(val) = table.get_value(row_idx, col_idx) {
                                    if matches!(val, Value::Null) {
                                        null_count += 1;
                                    } else {
                                        distinct_set.insert(format!("{:?}", val));
                                    }
                                } else {
                                    null_count += 1;
                                }
                            }
                            columns.insert(
                                col_idx as u32,
                                akar_storage::stats::ColumnStats {
                                    table_id,
                                    column_id: col_idx as u32,
                                    num_distinct_values: distinct_set.len() as u64,
                                    num_null_values: null_count,
                                    min_value: None,
                                    max_value: None,
                                },
                            );
                        }
                        stats.update_table_stats(
                            table_id,
                            akar_storage::stats::TableStats {
                                num_rows: row_count,
                                columns,
                            },
                        );
                    } else {
                        // Try rel table
                        let rel_table = catalog.get_rel_table(table_id);
                        if let Some(table) = rel_table {
                            let row_count = table.num_rows;
                            let mut columns = std::collections::HashMap::new();
                            for col_idx in 0..table.columns.len() {
                                let mut null_count: u64 = 0;
                                let mut distinct_set = std::collections::HashSet::new();
                                if let Some(col_data) = table.get_column(col_idx) {
                                    for val in col_data {
                                        if matches!(val, Value::Null) {
                                            null_count += 1;
                                        } else {
                                            distinct_set.insert(format!("{:?}", val));
                                        }
                                    }
                                }
                                columns.insert(
                                    col_idx as u32,
                                    akar_storage::stats::ColumnStats {
                                        table_id,
                                        column_id: col_idx as u32,
                                        num_distinct_values: distinct_set.len() as u64,
                                        num_null_values: null_count,
                                        min_value: None,
                                        max_value: None,
                                    },
                                );
                            }
                            stats.update_table_stats(
                                table_id,
                                akar_storage::stats::TableStats {
                                    num_rows: row_count,
                                    columns,
                                },
                            );
                        }
                    }
                }

                let table_desc = a.table_name.as_deref().unwrap_or("all tables");
                Ok(Some(QueryResult::success_message(format!(
                    "Statistics collected for {}",
                    table_desc
                ))))
            }
        }
    }

    /// Write query results to a Parquet file.
    #[cfg(feature = "parquet-export")]
    pub(crate) fn write_parquet(&self, path: &str, result: &QueryResult, _header: bool) -> Result<(), String> {
        write_parquet_to_file(path, result)
    }
}

/// Standalone function to write query results to a Parquet file.
/// Used by both `COPY TO` and `CALL export_parquet()`.
#[cfg(feature = "parquet-export")]
pub fn write_parquet_to_file(path: &str, result: &QueryResult) -> Result<(), String> {
    let mut rows: Vec<Vec<Value>> = Vec::new();
    for chunk in &result.chunks {
        for row_idx in 0..chunk.size {
            let mut row = Vec::with_capacity(chunk.fields.len());
            for col_idx in 0..chunk.fields.len() {
                row.push(chunk.get_value(col_idx, row_idx).unwrap_or(Value::Null));
            }
            rows.push(row);
        }
    }

    let column_names: Vec<String> = result
        .chunks
        .first()
        .map(|chunk| {
            if chunk.field_names.is_empty() {
                (0..chunk.fields.len()).map(|i| format!("column_{}", i)).collect()
            } else {
                chunk
                    .field_names
                    .iter()
                    .map(|n| {
                        n.rsplit_once('.')
                            .map(|(_, base)| base.to_string())
                            .unwrap_or_else(|| n.clone())
                    })
                    .collect()
            }
        })
        .unwrap_or_default();

    Ok(akar_storage::parquet_writer::write_parquet(path, &rows, &column_names)?)
}
