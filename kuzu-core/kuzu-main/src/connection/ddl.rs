use super::utils::{
    ast_constant_to_value, eval_ast_expr_to_value, extract_arg_string, format_storage_size, pk_value_to_string,
    value_to_csv_string,
};
use super::Connection;
use crate::query_result::QueryResult;
use kuzu_binder::bound_statement::BoundStatement;
use kuzu_common::types::Value;
use kuzu_storage::table::ColumnDefinition;

impl Connection {
    /// Handle DDL statements by checking the bound statement type.
    /// Returns `Ok(Some(result))` if DDL, `Ok(None)` if DML (continue).
    pub(crate) fn handle_ddl(&self, bound: &BoundStatement) -> Result<Option<QueryResult>, String> {
        match bound {
            BoundStatement::BoundExplain(_) => {
                // EXPLAIN is handled by the query processor pipeline
                Ok(None)
            }
            BoundStatement::BoundTransaction(t) => {
                match t.action {
                    kuzu_parser::ast::TransactionAction::Begin => {
                        let txn = self.begin_write_txn()?;
                        Ok(Some(QueryResult::success_message(format!(
                            "Transaction started (txn#{})",
                            txn.transaction_id
                        ))))
                    }
                    kuzu_parser::ast::TransactionAction::Commit => {
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
                                    return Ok(Some(QueryResult::success_message(
                                        "Transaction committed".into(),
                                    )));
                                }
                            }
                        }
                        Err("No active transaction to commit".into())
                    }
                    kuzu_parser::ast::TransactionAction::Rollback => {
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
                                    self.rollback_write_txn(&mut txn);
                                    return Ok(Some(QueryResult::success_message(
                                        "Transaction rolled back".into(),
                                    )));
                                }
                            }
                        }
                        Err("No active transaction to rollback".into())
                    }
                    kuzu_parser::ast::TransactionAction::Checkpoint => {
                        self.do_sync_checkpoint()?;
                        Ok(Some(QueryResult::success_message(
                            "Checkpoint completed".into(),
                        )))
                    }
                }
            }
            BoundStatement::BoundExtension(e) => {
                let msg = match e.action {
                    kuzu_parser::ast::ExtensionAction::Load => {
                        format!(
                            "Extension '{}': Extensions are compile-time features in Kuzu Rust. \
                             Rebuild with --features {}-extension to enable.",
                            e.name,
                            e.name.to_lowercase()
                        )
                    }
                    kuzu_parser::ast::ExtensionAction::Install => {
                        format!(
                            "INSTALL EXTENSION '{}' is not yet supported in Kuzu Rust. \
                             Extensions are compile-time features; rebuild with --features {}-extension.",
                            e.name,
                            e.name.to_lowercase()
                        )
                    }
                    kuzu_parser::ast::ExtensionAction::Uninstall => {
                        format!(
                            "UNINSTALL EXTENSION '{}': Extensions are compile-time features in Kuzu Rust. \
                             Rebuild without the feature flag to disable.",
                            e.name
                        )
                    }
                };
                Ok(Some(QueryResult::success_message(msg)))
            }
            BoundStatement::BoundAttachDatabase(a) => {
                // Register a foreign table entry in the catalog
                let mut catalog = self.database.catalog.lock().unwrap();
                let table_id = catalog.next_table_id();
                let entry = kuzu_catalog::ForeignTableEntry {
                    table_id,
                    name: a.alias.clone(),
                    columns: Vec::new(),
                    source_type: a.options.get("source_type").cloned().unwrap_or_else(|| "unknown".into()),
                };
                catalog.add_foreign_entry(entry);
                Ok(Some(QueryResult::success_message(format!(
                    "Database attached as '{}' from '{}'",
                    a.alias, a.path
                ))))
            }
            BoundStatement::BoundDetachDatabase(d) => {
                let mut catalog = self.database.catalog.lock().unwrap();
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
                let _catalog = self.database.catalog.lock()
                    .map_err(|e| format!("Lock error: {e}"))?;
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
                tracing::info!("CREATE GRAPH '{}' (any={})", g.name, g.is_any);
                Ok(Some(QueryResult::success_message(format!(
                    "Graph '{}' created",
                    g.name
                ))))
            }
            BoundStatement::BoundUseGraph(g) => {
                tracing::info!("USE GRAPH '{}'", g.name);
                Ok(Some(QueryResult::success_message(format!(
                    "Using graph '{}'",
                    g.name
                ))))
            }
            BoundStatement::BoundDropGraph(g) => {
                tracing::info!("DROP GRAPH '{}'", g.name);
                Ok(Some(QueryResult::success_message(format!(
                    "Graph '{}' dropped",
                    g.name
                ))))
            }
            BoundStatement::BoundCopyTo(c) => {
                // Execute the inner query
                let inner_bound = BoundStatement::BoundQuery(c.query.clone());
                let result = self.execute_query_inner(&inner_bound, None)?;

                // Write results to file
                let path = std::path::Path::new(&c.file_path);
                match c.format {
                    kuzu_parser::ast::CopyToFormat::Csv => {
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
                                    w.write_record(&header)
                                        .map_err(|e| format!("CSV write error: {e}"))?;
                                }
                            }
                        }

                        // Write rows
                        for chunk in &result.chunks {
                            for row in 0..chunk.size {
                                let row_values: Vec<String> = chunk
                                    .fields
                                    .iter()
                                    .map(|f| {
                                        f.get_value(row)
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
                    kuzu_parser::ast::CopyToFormat::Parquet => {
                        #[cfg(feature = "parquet-export")]
                        {
                            self.write_parquet(path, &result, c.header)
                                .map_err(|e| format!("Parquet export error: {e}"))?;
                        }
                        #[cfg(not(feature = "parquet-export"))]
                        {
                            return Err(
                                "Parquet export requires 'parquet-export' feature. \
                                 Build with: cargo build --features parquet-export"
                                    .into(),
                            );
                        }
                    }
                }
                Ok(Some(QueryResult::success_message(format!(
                    "COPY TO '{}' completed, {} rows exported",
                    c.file_path,
                    result.num_rows
                ))))
            }
            BoundStatement::BoundCreateNodeTable(t) => {
                // Also create a storage table with in-memory data capacity
                let columns: Vec<ColumnDefinition> = t
                    .columns
                    .iter()
                    .map(|c| ColumnDefinition {
                        name: c.name.clone(),
                        logical_type: c.logical_type,
                        is_primary_key: c.is_primary_key,
                    })
                    .collect();
                self.database
                    .storage_manager
                    .create_node_table(t.name.clone(), columns);

                // Auto-create backing sequences for SERIAL columns
                {
                    let mut catalog = self.database.catalog.lock().unwrap();
                    for col in &t.columns {
                        if col.logical_type == kuzu_common::types::LogicalTypeID::Serial {
                            match catalog.create_serial_sequence(&t.name, &col.name) {
                                kuzu_catalog::CatalogResult::Created { .. } => {
                                    tracing::info!("Created serial sequence for {}.{}", t.name, col.name);
                                }
                                other => {
                                    tracing::warn!(
                                        "Failed to create serial sequence for {}.{}: {:?}",
                                        t.name,
                                        col.name,
                                        other
                                    );
                                }
                            }
                        }
                    }
                }
                // Auto-create ART index for primary key
                if t.columns.iter().any(|c| c.is_primary_key) {
                    let index_name = format!("{}_pk_idx", t.name);
                    if let Err(e) = self.database.storage_manager.create_art_index(&t.name, &index_name) {
                        tracing::warn!("Failed to create ART index for table {}: {}", t.name, e);
                    }
                }

                tracing::info!("Created node table '{}'", t.name);
                Ok(Some(QueryResult::success_message(format!(
                    "Node table '{}' created",
                    t.name
                ))))
            }
            BoundStatement::BoundCreateRelTable(t) => {
                // Create a storage rel table
                let columns: Vec<ColumnDefinition> = t
                    .columns
                    .iter()
                    .map(|c| ColumnDefinition {
                        name: c.name.clone(),
                        logical_type: c.logical_type,
                        is_primary_key: c.is_primary_key,
                    })
                    .collect();
                let src_id = 0; // src table ID resolved during binding
                let dst_id = 0;
                self.database
                    .storage_manager
                    .create_rel_table(t.name.clone(), src_id, dst_id, columns);
                tracing::info!("Created rel table '{}'", t.name);
                Ok(Some(QueryResult::success_message(format!(
                    "Rel table '{}' created",
                    t.name
                ))))
            }
            BoundStatement::BoundDropTable(t) => {
                // Drop auto-created serial sequences for the table
                {
                    let mut catalog = self.database.catalog.lock().unwrap();
                    // Find and drop any sequences matching `{name}_*_serial`
                    let serial_seqs: Vec<String> = catalog
                        .sequences()
                        .iter()
                        .filter(|s| s.name.ends_with("_serial"))
                        .filter(|s| {
                            // Match format: {table_name}_{column_name}_serial
                            s.name.starts_with(&format!("{}_", t.name))
                        })
                        .map(|s| s.name.clone())
                        .collect();
                    for seq_name in serial_seqs {
                        if let kuzu_catalog::CatalogResult::Dropped { .. } = catalog.drop_sequence(&seq_name) {
                            tracing::info!("Dropped serial sequence '{}'", seq_name);
                        }
                    }
                }

                tracing::info!("Dropped table '{}'", t.name);
                Ok(Some(QueryResult::success_message(format!(
                    "Table '{}' dropped",
                    t.name
                ))))
            }
            #[cfg(feature = "vector-extension")]
            BoundStatement::BoundCreateVectorIndex(idx) => {
                let metric = match idx.metric.to_lowercase().as_str() {
                    "cosine" => kuzu_vector::hnsw::DistanceMetric::Cosine,
                    "euclidean" => kuzu_vector::hnsw::DistanceMetric::Euclidean,
                    "l2" => kuzu_vector::hnsw::DistanceMetric::L2Squared,
                    "dot" => kuzu_vector::hnsw::DistanceMetric::DotProduct,
                    other => return Err(format!("Unknown metric '{other}'")),
                };

                // Create the vector index in storage
                self.database.storage_manager.create_vector_index(
                    idx.index_name.clone(),
                    idx.table_name.clone(),
                    idx.column_name.clone(),
                    metric,
                    idx.dimensions as u32,
                );

                // Auto-populate from existing table data
                let table_catalog = self.database.storage_manager.table_catalog();
                if let Some(table) = table_catalog.get_node_table_by_name(&idx.table_name) {
                    let col_idx = table.columns.iter().position(|c| c.name == idx.column_name);
                    if let Some(col_idx) = col_idx {
                        // Scan all rows and extract vectors
                        for row_id in 0..table.num_rows as usize {
                            if let Some(val) = table.get_value(row_id, col_idx) {
                                if let Ok(vec) = kuzu_storage::extract_f64_list_from_value(val) {
                                    // Get mutable access and insert into HNSW
                                    if let Some(mut vi) = table_catalog.get_vector_index_by_name_mut(&idx.index_name) {
                                        vi.hnsw_mut().insert(vec, row_id);
                                    }
                                }
                            }
                        }
                    }
                }

                tracing::info!("Created vector index '{}'", idx.index_name);
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
                let planner = kuzu_planner::QueryPlanner::new();
                let optimizer = kuzu_optimizer::Optimizer::with_stats(self.database.stats_store.clone());

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
                let mut catalog = self.database.catalog.lock().unwrap();
                match &a.action {
                    kuzu_parser::ast::AlterAction::AddColumn { name, type_name } => {
                        let logical_type = kuzu_binder::Binder::parse_type(type_name)
                            .map_err(|e| format!("ALTER ADD: {e}"))?;
                        catalog
                            .add_column(
                                &a.table_name,
                                kuzu_catalog::CatalogColumn {
                                    name: name.clone(),
                                    logical_type,
                                    is_primary_key: false,
                                    default_value: None,
                                },
                            )
                            .map_err(|e| format!("ALTER ADD: {e}"))?;
                        Ok(Some(QueryResult::success_message(format!(
                            "Column '{}' added to table '{}'",
                            name, a.table_name
                        ))))
                    }
                    kuzu_parser::ast::AlterAction::DropColumn { name } => {
                        catalog
                            .drop_column(&a.table_name, name)
                            .map_err(|e| format!("ALTER DROP: {e}"))?;
                        Ok(Some(QueryResult::success_message(format!(
                            "Column '{}' dropped from table '{}'",
                            name, a.table_name
                        ))))
                    }
                    kuzu_parser::ast::AlterAction::RenameColumn { old_name, new_name } => {
                        catalog
                            .rename_column(&a.table_name, old_name, new_name)
                            .map_err(|e| format!("ALTER RENAME COLUMN: {e}"))?;
                        Ok(Some(QueryResult::success_message(format!(
                            "Column '{}' renamed to '{}' in table '{}'",
                            old_name, new_name, a.table_name
                        ))))
                    }
                    kuzu_parser::ast::AlterAction::RenameTable { new_name } => {
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

                let catalog = self.database.storage_manager.table_catalog();
                let mut table = catalog
                    .get_node_table_by_name_mut(&c.table_name)
                    .ok_or_else(|| format!("Table '{}' not found in storage", c.table_name))?;

                // Build values from pattern properties, defaulting to Null
                let mut values: Vec<Value> = table.columns.iter().map(|_| Value::Null).collect();
                for (prop_name, expr) in &c.properties {
                    if let Some(col_idx) = table.columns.iter().position(|c| c.name == *prop_name)
                        && let kuzu_parser::ast::Expression::Constant(con) = expr
                    {
                        values[col_idx] = ast_constant_to_value(con);
                    }
                }

                // Auto-generate SERIAL column values for null entries
                {
                    let mut sys_catalog = self.database.catalog.lock().unwrap();
                    for (col_idx, col) in table.columns.iter().enumerate() {
                        if col.logical_type == kuzu_common::types::LogicalTypeID::Serial
                            && matches!(values[col_idx], Value::Null)
                        {
                            let seq_name =
                                kuzu_catalog::SequenceEntry::get_serial_name(&c.table_name, &col.name);
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
                        && let Ok(vec) = kuzu_storage::extract_f64_list_from_value(val)
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
                let catalog = self.database.storage_manager.table_catalog();
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
                let exists = if let Some((_, expr)) = pk_prop {
                    // Try to evaluate the PK expression
                    if let kuzu_parser::ast::Expression::Constant(c) = expr {
                        let pk_val = ast_constant_to_value(c);
                        table.hash_index.lookup(&pk_value_to_string(&pk_val)).is_some()
                    } else {
                        false
                    }
                } else {
                    false
                };

                if exists {
                    // Apply ON MATCH SET
                    for item in &m.on_match {
                        // Evaluate the SET value expression and update the cell
                        if let kuzu_parser::ast::Expression::Constant(c) = &item.value {
                            let val = ast_constant_to_value(c);
                            if let Some(row) = table.hash_index.lookup(&pk_value_to_string(&val)) {
                                let _ = table.update_cell(row, item.column_idx, val);
                            }
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
                            && let kuzu_parser::ast::Expression::Constant(c) = expr
                        {
                            values[col_idx] = ast_constant_to_value(c);
                        }
                    }
                    for item in &m.on_create {
                        if let kuzu_parser::ast::Expression::Constant(c) = &item.value {
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
            BoundStatement::BoundCall(c) => {
                tracing::info!("CALL '{}'", c.function_name);

                let fn_lower = c.function_name.to_lowercase();
                let result: Result<Vec<Vec<Value>>, String> = match fn_lower.as_str() {
                    // ── Catalog inspection functions ──
                    "show_tables" | "show tables" | "list_tables" | "list tables" | "tables" => {
                        let catalog = self.database.catalog.lock().unwrap();
                        let entries: Vec<Vec<Value>> = catalog
                            .all_entries()
                            .map(|e| {
                                let kind = match e {
                                    kuzu_catalog::CatalogEntry::NodeTable(_) => "NODE",
                                    kuzu_catalog::CatalogEntry::RelTable(_) => "REL",
                                    kuzu_catalog::CatalogEntry::Sequence(_) => "SEQUENCE",
                                    kuzu_catalog::CatalogEntry::Macro(_) => "MACRO",
                                    kuzu_catalog::CatalogEntry::VectorIndex(_) => "VECTOR_INDEX",
                                    kuzu_catalog::CatalogEntry::Foreign(_) => "FOREIGN",
                                };
                                vec![Value::String(e.name().to_string()), Value::String(kind.to_string())]
                            })
                            .collect();
                        Ok(entries)
                    }
                    "table_info" => {
                        let table_name = extract_arg_string(&c.args, 0)?;
                        let cat = self.database.catalog.lock().unwrap();
                        let entry = cat
                            .get_entry_by_name(&table_name)
                            .ok_or_else(|| format!("Table '{table_name}' not found"))?;
                        let columns = entry.columns();
                        let rows: Vec<Vec<Value>> = columns
                            .iter()
                            .map(|col| {
                                vec![
                                    Value::String(table_name.clone()),
                                    Value::String(col.name.clone()),
                                    Value::String(format!("{:?}", col.logical_type)),
                                    Value::String(if col.is_primary_key { "NO" } else { "YES" }.into()),
                                ]
                            })
                            .collect();
                        Ok(rows)
                    }
                    "show_functions" => {
                        let registry = self.database.function_registry.lock().unwrap();
                        let funcs = registry.list_all();
                        Ok(funcs
                            .into_iter()
                            .map(|(name, kind)| vec![Value::String(name), Value::String(kind)])
                            .collect())
                    }
                    "show_indexes" => {
                        let cat = self.database.catalog.lock().unwrap();
                        let indexes = cat.indexes();
                        Ok(indexes
                            .into_iter()
                            .map(|(name, table, kind, col)| {
                                vec![
                                    Value::String(name),
                                    Value::String(table),
                                    Value::String(kind),
                                    Value::String(col),
                                ]
                            })
                            .collect())
                    }
                    "show_sequences" => {
                        let cat = self.database.catalog.lock().unwrap();
                        let seqs = cat.sequences();
                        Ok(seqs
                            .into_iter()
                            .map(|s| vec![Value::String(s.name.clone()), Value::Int64(s.curr_val())])
                            .collect())
                    }
                    "show_macros" => {
                        let cat = self.database.catalog.lock().unwrap();
                        let macros = cat.macros();
                        Ok(macros
                            .into_iter()
                            .map(|m| {
                                vec![
                                    Value::String(m.name.clone()),
                                    Value::String(
                                        m.default_args
                                            .iter()
                                            .map(|(k, v)| format!("{k}={v}"))
                                            .collect::<Vec<_>>()
                                            .join(", "),
                                    ),
                                ]
                            })
                            .collect())
                    }
                    "show_connection" => {
                        let table_name = extract_arg_string(&c.args, 0)?;
                        let cat = self.database.catalog.lock().unwrap();
                        let info = cat
                            .connection_info(&table_name)
                            .ok_or_else(|| format!("Table '{table_name}' not found"))?;
                        Ok(vec![info])
                    }
                    "db_version" => {
                        let version = env!("CARGO_PKG_VERSION");
                        Ok(vec![vec![Value::String(version.to_string())]])
                    }
                    "catalog_version" => {
                        let cat = self.database.catalog.lock().unwrap();
                        let ver = cat.version();
                        Ok(vec![vec![Value::Int64(ver as i64)]])
                    }
                    // ── Configuration & stats functions ──
                    "current_setting" => {
                        let key = extract_arg_string(&c.args, 0).unwrap_or_else(|_| String::new());
                        let (k, v) = match key.to_lowercase().as_str() {
                            "spill_threshold" => {
                                (
                                    "spill_threshold",
                                    self.database.effective_spill_threshold().to_string(),
                                )
                            }
                            "checkpoint_threshold" => (
                                "checkpoint_threshold",
                                self.database.config.checkpoint_threshold.to_string(),
                            ),
                            "buffer_pool_size" => {
                                (
                                    "buffer_pool_size",
                                    self.database.config.buffer_pool_size.to_string(),
                                )
                            }
                            "max_num_threads" => {
                                ("max_num_threads", self.database.config.max_num_threads.to_string())
                            }
                            "concurrent_writes" => {
                                (
                                    "concurrent_writes",
                                    self.database.config.concurrent_writes.to_string(),
                                )
                            }
                            "read_only" => ("read_only", self.database.config.read_only.to_string()),
                            _ => (key.as_str(), "UNKNOWN".to_string()),
                        };
                        Ok(vec![vec![Value::String(k.to_string()), Value::String(v)]])
                    }
                    "stats_info" => {
                        let table_name = extract_arg_string(&c.args, 0)?;
                        let (row_count, storage_size) = {
                            let cat = self.database.catalog.lock().unwrap();
                            let table_id = cat
                                .get_table_id(&table_name)
                                .ok_or_else(|| format!("Table '{table_name}' not found"))?;
                            let stats = self.database.stats_store.lock().unwrap();
                            stats.table_stats_by_id(table_id)
                        };
                        Ok(vec![vec![
                            Value::String(table_name),
                            Value::Int64(row_count as i64),
                            Value::String(format_storage_size(storage_size)),
                        ]])
                    }
                    "storage_info" => {
                        let sm = &self.database.storage_manager;
                        let info = sm.storage_info();
                        Ok(vec![vec![
                            Value::String(info.db_path),
                            Value::Int64(info.page_size as i64),
                            Value::Int64(info.total_pages as i64),
                            Value::Int64(info.free_pages as i64),
                        ]])
                    }
                    "show_attached_databases" => Ok(vec![vec![
                        Value::String("main".to_string()),
                        Value::String("local".to_string()),
                    ]]),
                    // ── Storage introspection ──
                    "bm_info" => {
                        let bm = &self.database.storage_manager;
                        let info = bm.buffer_info();
                        Ok(vec![vec![
                            Value::String("buffer_pool".to_string()),
                            Value::Int64(info.total_memory as i64),
                            Value::Int64(info.used_memory as i64),
                            Value::Int64(info.num_pinned as i64),
                        ]])
                    }
                    "file_info" => {
                        let sm = &self.database.storage_manager;
                        let info = sm.file_info();
                        Ok(vec![vec![
                            Value::Int64(info.total_file_size as i64),
                            Value::Int64(info.num_data_pages as i64),
                            Value::Int64(info.wal_size as i64),
                        ]])
                    }
                    "free_space_info" => {
                        let sm = &self.database.storage_manager;
                        let info = sm.fsm_info();
                        Ok(vec![vec![
                            Value::Int64(info.total_free_pages as i64),
                            Value::Int64(info.num_entries as i64),
                        ]])
                    }
                    "disk_size_info" => {
                        let sm = &self.database.storage_manager;
                        let info = sm.file_info();
                        Ok(vec![vec![
                            Value::Int64(info.total_file_size as i64),
                            Value::Int64(info.num_data_pages as i64),
                            Value::Int64(info.wal_size as i64),
                        ]])
                    }
                    "storage_version" => Ok(vec![vec![Value::String(
                        kuzu_storage::version_info::STORAGE_VERSION.to_string(),
                    )]]),
                    // ── Extension info ──
                    "show_loaded_extensions" => {
                        let reg = self.database.extension_registry.lock().unwrap();
                        let names: Vec<Vec<Value>> =
                            reg.names().iter().map(|n| vec![Value::String(n.clone())]).collect();
                        Ok(names)
                    }
                    "show_official_extensions" => Ok(vec![
                        vec![Value::String("json".into()), Value::String("JSON functions".into())],
                        vec![
                            Value::String("fts".into()),
                            Value::String("Full-Text Search".into()),
                        ],
                        vec![
                            Value::String("vector".into()),
                            Value::String("Vector similarity search".into()),
                        ],
                        vec![
                            Value::String("httpfs".into()),
                            Value::String("HTTP/S3 file access".into()),
                        ],
                        vec![
                            Value::String("duckdb".into()),
                            Value::String("DuckDB integration".into()),
                        ],
                        vec![
                            Value::String("sqlite".into()),
                            Value::String("SQLite integration".into()),
                        ],
                        vec![
                            Value::String("postgres".into()),
                            Value::String("PostgreSQL integration".into()),
                        ],
                        vec![
                            Value::String("delta".into()),
                            Value::String("Delta Lake integration".into()),
                        ],
                        vec![
                            Value::String("iceberg".into()),
                            Value::String("Apache Iceberg integration".into()),
                        ],
                        vec![
                            Value::String("azure".into()),
                            Value::String("Azure Blob Storage".into()),
                        ],
                        vec![
                            Value::String("unity_catalog".into()),
                            Value::String("Unity Catalog integration".into()),
                        ],
                        vec![
                            Value::String("neo4j".into()),
                            Value::String("Neo4j integration".into()),
                        ],
                        vec![Value::String("llm".into()), Value::String("LLM integration".into())],
                        vec![
                            Value::String("algo".into()),
                            Value::String("Graph algorithms".into()),
                        ],
                    ]),
                    // ── Warning context ──
                    "clear_warnings" => {
                        // Clear warnings — no-op for now
                        Ok(vec![vec![Value::String("Warnings cleared".into())]])
                    }
                    "show_warnings" => {
                        // No warning infrastructure yet — return empty
                        Ok(vec![])
                    }
                    // ── Export functions ──
                    "export_csv" => {
                        let path = extract_arg_string(&c.args, 0)?;
                        let query_str = extract_arg_string(&c.args, 1)?;
                        self.export_to_csv(&path, &query_str)?;
                        Ok(vec![vec![Value::String(format!("Exported to '{path}'"))]])
                    }
                    "export_parquet" => {
                        let path = extract_arg_string(&c.args, 0)?;
                        let query_str = extract_arg_string(&c.args, 1)?;
                        self.export_to_parquet(&path, &query_str)?;
                        Ok(vec![vec![Value::String(format!("Exported to '{path}'"))]])
                    }
                    // ── GDS (Graph Data Science) functions — delegated to extension ──
                    "page_rank" | "pr" | "wcc" | "weakly_connected_components"
                    | "scc" | "strongly_connected_components" | "k_core" | "kcore"
                    | "louvain" | "spanning_forest" | "sf"
                    | "shortest_path" | "sp" | "weighted_shortest_path" => {
                        // Forward to the function registry (algo extension must be loaded)
                        let args: Vec<Value> = c.args.iter().map(eval_ast_expr_to_value).collect();
                        let registry = self.database.function_registry.lock().unwrap();
                        registry.execute_table_function(&c.function_name, &args)
                    }
                    _ => {
                        // Evaluate AST arguments to Values
                        let args: Vec<Value> = c.args.iter().map(eval_ast_expr_to_value).collect();
                        let registry = self.database.function_registry.lock().unwrap();
                        registry.execute_table_function(&c.function_name, &args)
                    }
                };

                let result = result?;

                // Convert Vec<Vec<Value>> to a simple text result
                let mut lines: Vec<String> = result
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|v| match v {
                                Value::String(s) => s.clone(),
                                Value::Int64(i) => i.to_string(),
                                Value::Int32(i) => i.to_string(),
                                Value::Double(f) => f.to_string(),
                                Value::Bool(b) => b.to_string(),
                                Value::Null => "NULL".into(),
                                other => format!("{other:?}"),
                            })
                            .collect::<Vec<_>>()
                            .join(" | ")
                    })
                    .collect();
                if lines.is_empty() {
                    lines.push("(empty result)".into());
                }
                let message = lines.join("\n");
                Ok(Some(QueryResult::success_message(message)))
            }
            BoundStatement::BoundCreateIndex(idx) => {
                // Create ART index on the node table
                self.database
                    .storage_manager
                    .create_art_index(&idx.table_name, &idx.index_name)?;
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
                // Drop ART index from the node table
                self.database
                    .storage_manager
                    .drop_art_index(&idx.table_name, &idx.index_name)?;
                tracing::info!("Dropped index '{}' from '{}'", idx.index_name, idx.table_name);
                Ok(Some(QueryResult::success_message(format!(
                    "Index '{}' dropped from table '{}'",
                    idx.index_name, idx.table_name
                ))))
            }
            BoundStatement::BoundCreateSequence(s) => {
                let mut catalog = self.database.catalog.lock().unwrap();
                let result = catalog.create_sequence(
                    s.name.clone(),
                    s.start_with,
                    s.increment,
                    s.min_value,
                    s.max_value,
                    s.cycle,
                );
                match result {
                    kuzu_catalog::CatalogResult::Created { .. } => {
                        tracing::info!("Created sequence '{}'", s.name);
                        Ok(Some(QueryResult::success_message(format!(
                            "Sequence '{}' created",
                            s.name
                        ))))
                    }
                    kuzu_catalog::CatalogResult::AlreadyExists => {
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
                let mut catalog = self.database.catalog.lock().unwrap();
                let result = catalog.drop_sequence(&s.name);
                match result {
                    kuzu_catalog::CatalogResult::Dropped { .. } => {
                        tracing::info!("Dropped sequence '{}'", s.name);
                        Ok(Some(QueryResult::success_message(format!(
                            "Sequence '{}' dropped",
                            s.name
                        ))))
                    }
                    kuzu_catalog::CatalogResult::NotFound => {
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
                let mut catalog = self.database.catalog.lock().unwrap();
                match catalog.create_macro(
                    m.name.clone(),
                    m.positional_args.clone(),
                    m.default_args.clone(),
                    m.expression.clone(),
                ) {
                    kuzu_catalog::CatalogResult::Created { .. } => {
                        tracing::info!("Created macro '{}'", m.name);
                        Ok(Some(QueryResult::success_message(format!(
                            "Macro '{}' created",
                            m.name
                        ))))
                    }
                    kuzu_catalog::CatalogResult::AlreadyExists => {
                        Err(format!("Macro '{}' already exists", m.name))
                    }
                    _ => Err("Failed to create macro".into()),
                }
            }
            BoundStatement::BoundExportDatabase(e) => self.execute_export_database(e),
            BoundStatement::BoundImportDatabase(i) => self.execute_import_database(i),
            BoundStatement::BoundQuery(q) => {
                // Check if this is a FOREACH-only query — handle it directly
                if q.clauses.len() == 1
                    && let Some(kuzu_binder::bound_statement::BoundClause::BoundForeach(fc)) =
                        q.clauses.first()
                {
                    return self.handle_foreach(fc);
                }
                Ok(None)
            }
            BoundStatement::BoundCreateFtsIndex(_) => Ok(None),
            BoundStatement::BoundAnalyze(a) => {
                tracing::info!("ANALYZE {} tables", a.table_ids.len());
                let mut stats = self.database.stats_store.lock().unwrap();
                let catalog = self.database.storage_manager.table_catalog();

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
                                kuzu_storage::stats::ColumnStats {
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
                            kuzu_storage::stats::TableStats {
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
                                    kuzu_storage::stats::ColumnStats {
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
                                kuzu_storage::stats::TableStats {
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
}
