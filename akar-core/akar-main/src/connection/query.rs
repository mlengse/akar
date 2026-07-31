use super::plan_cache::{normalize_query, CachedPlan};
use super::Connection;
use crate::prepared_statement::PreparedStatement;
use crate::query_result::QueryResult;
use akar_binder::Binder;
use akar_binder::bound_statement::BoundStatement;
use akar_common::error::ProcessorError;
use akar_common::types::Value;
use akar_optimizer::Optimizer;
use akar_parser::parse;
use akar_planner::QueryPlanner;
use akar_planner::logical_operator::LogicalOperator;
use akar_processor::QueryProcessor;
use std::collections::HashMap;
use std::sync::Arc;

impl Connection {
    /// Execute a Cypher query and return the result.
    pub fn query(&self, query_str: &str) -> Result<QueryResult, String> {
        let trimmed = query_str.trim();

        // Skip empty queries
        if trimmed.is_empty() {
            return Ok(QueryResult::new(Vec::new()));
        }

        // Handle SET spill_threshold
        if let Some(value) = trimmed
            .strip_prefix("SET")
            .and_then(|s| s.trim().strip_prefix("spill_threshold"))
            .and_then(|s| s.trim().strip_prefix("="))
            .map(|s| s.trim())
        {
            let bytes: u64 = value.parse().map_err(|_| {
                format!("Invalid spill_threshold value '{value}'. Expected a positive integer (bytes).")
            })?;
            self.database.set_spill_threshold(bytes);
            return Ok(QueryResult::success_message(format!(
                "spill_threshold set to {bytes} bytes"
            )));
        }

        if let Some(value) = trimmed
            .strip_prefix("SET")
            .and_then(|s| s.trim().strip_prefix("concurrent_writes"))
            .and_then(|s| s.trim().strip_prefix("="))
            .map(|s| s.trim())
        {
            let enabled = match value.to_lowercase().as_str() {
                "true" | "1" | "yes" => true,
                "false" | "0" | "no" => false,
                _ => return Err("Invalid value for concurrent_writes. Use true or false.".into()),
            };
            self.database.transaction_manager.set_concurrent_writes(enabled);
            return Ok(QueryResult::success_message(format!(
                "concurrent_writes set to {enabled}"
            )));
        }

        // Normalize the query into a stable cache key
        let normalized = normalize_query(trimmed);

        // Try the plan cache first — skips parse/bind/plan/optimize entirely.
        // Entries are only valid if the catalog hasn't changed since build.
        let catalog_version = self
            .database
            .catalog
            .lock()
            .map_err(|e| format!("Catalog lock error: {e}"))?
            .version();

        {
            let mut cache = self.plan_cache.lock().map_err(|e| format!("Lock poisoned: {e}"))?;
            if let Some(cached) = cache.get(&normalized).filter(|c| c.catalog_version == catalog_version) {
                let bound = cached.bound.clone();
                let plan = cached.plan.clone();
                drop(cache);
                return self.execute_with_plan(&bound, Some(&plan));
            }
        }

        // Cache miss: full pipeline
        // 1. Parse
        let statement = parse(trimmed).map_err(|e| format!("Parse error: {e}"))?;

        // 2. Bind (using shared catalog Arc — DDL mutations persist)
        let binder = Binder::new(self.database.catalog.clone());
        let bound = binder.bind(statement).map_err(|e| format!("Bind error: {e}"))?;

        // 3. Build (and cache) the optimized plan for plan-cachable statements.
        //    DDL and other non-query statements are routed inside
        //    execute_query_inner and never cached.
        let plan_opt: Option<Vec<LogicalOperator>> = if is_plan_cachable(&bound) {
            let plan = self.build_optimized_plan(&bound)?;
            let mut cache = self.plan_cache.lock().map_err(|e| format!("Lock poisoned: {e}"))?;
            cache.insert(
                normalized,
                CachedPlan {
                    bound: bound.clone(),
                    plan: plan.clone(),
                    catalog_version,
                },
            );
            Some(plan)
        } else {
            None
        };

        self.execute_with_plan(&bound, plan_opt.as_ref())
    }

    /// Shared execution path for `query()`: wraps write statements in an OCC
    /// transaction (when concurrent writes are enabled) and delegates to
    /// `execute_query_inner`, committing or rolling back as appropriate.
    fn execute_with_plan(
        &self,
        bound: &BoundStatement,
        plan: Option<&Vec<LogicalOperator>>,
    ) -> Result<QueryResult, String> {
        // Determine if this is a write operation and begin a transaction if so.
        // Only wrap in a transaction when concurrent_writes is enabled.
        // In single-writer mode, the old direct-write path is kept for
        // backward compatibility (no WAL commit records for pure DDL).
        let concurrent_mode = self.database.transaction_manager.allow_concurrent_writes();
        let is_write = concurrent_mode && Connection::is_write_statement(bound);
        let mut txn_opt: Option<akar_transaction::Transaction> =
            if is_write { Some(self.begin_write_txn()?) } else { None };

        // Execute the query within a transaction scope
        let query_result = self.execute_query_inner(bound, txn_opt.as_mut(), plan);

        // Commit or rollback based on result
        match (is_write, &query_result) {
            (true, Ok(_)) => {
                if let Some(ref mut txn) = txn_opt {
                    self.commit_write_txn(txn)?;
                }
            }
            (true, Err(e)) => {
                if let Some(ref mut txn) = txn_opt {
                    match self.rollback_write_txn(txn) {
                        Ok(_records) => {
                            tracing::warn!("Transaction rolled back due to error: {e}");
                        }
                        Err(rollback_err) => {
                            tracing::error!("Transaction rollback ALSO failed: {rollback_err} (original error: {e})");
                        }
                    }
                }
            }
            _ => {}
        }

        query_result
    }

    /// Run planner + optimizer to produce the optimized logical plan.
    fn build_optimized_plan(&self, bound: &BoundStatement) -> Result<Vec<LogicalOperator>, String> {
        let planner = QueryPlanner::new();
        let logical_plan = planner.plan(bound.clone()).map_err(|e| format!("Plan error: {e}"))?;
        let optimizer = Optimizer::with_stats(self.database.stats_store.clone());
        Ok(optimizer.optimize(logical_plan))
    }

    /// Inner query execution (after parsing and binding, before commit/rollback).
    ///
    /// `cached_plan` carries a pre-built optimized plan from the plan cache;
    /// when `None`, the plan is built here (DDL and non-query statements
    /// return before this point).
    pub(crate) fn execute_query_inner(
        &self,
        bound: &BoundStatement,
        txn_opt: Option<&mut akar_transaction::Transaction>,
        cached_plan: Option<&Vec<LogicalOperator>>,
    ) -> Result<QueryResult, String> {
        // Route: DDL vs DML (handle_ddl returns Some for DDL, None for DML)
        if let Some(result) = self.handle_ddl(bound)? {
            // DDL may have modified the catalog; checkpoint if needed.
            self.maybe_auto_checkpoint()?;
            return Ok(result);
        }

        // Lock tables for DML writes only in single-writer mode.
        // When concurrent_writes is enabled, OCC row-level conflict detection
        // replaces table-level locking (see record_write / validate_write_set).
        if let Some(ref txn) = txn_opt {
            if !self.database.transaction_manager.allow_concurrent_writes() {
                let write_tables = Connection::extract_write_tables(bound);
                for tid in write_tables {
                    self.database.transaction_manager.lock_table(txn.transaction_id, tid)?;
                }
            }
        }

        // Plan (from cache when available, otherwise build now)
        let optimized_plan: Vec<LogicalOperator> = match cached_plan {
            Some(plan) => plan.clone(),
            None => self.build_optimized_plan(bound)?,
        };

        if optimized_plan.is_empty() {
            return Ok(QueryResult::success_message("Query executed (no result)".into()));
        }

        // Capture MVCC snapshot for read isolation.
        // For write transactions, use the txn's snapshot_ts.
        // For read-only queries, capture a fresh snapshot from the transaction manager.
        let (snapshot_ts, commit_history) = if let Some(ref txn) = txn_opt {
            (txn.snapshot_ts, self.database.transaction_manager.commit_history_snapshot())
        } else {
            // Read-only query: capture snapshot at current commit point
            let ts = self.database.transaction_manager.current_commit_ts();
            let history = self.database.transaction_manager.commit_history_snapshot();
            (Some(ts), history)
        };

        // Execute
        let processor = self.create_processor()
            .with_snapshot(snapshot_ts, commit_history);
        let chunks = processor
            .execute(&optimized_plan)
            .map_err(|e| format!("Execute error: {e}"))?;

        // Record row-level writes for OCC conflict detection
        if let Some(ref txn) = txn_opt {
            let written_rows = processor.take_written_rows();
            let tm = &self.database.transaction_manager;
            for (table_id, row_id) in written_rows {
                tm.record_write(txn.transaction_id, table_id, row_id);
            }
        }

        // Auto-checkpoint after DML execution
        self.maybe_auto_checkpoint()?;

        Ok(QueryResult::new(chunks))
    }

    /// Prepare a query for parameterized execution.
    ///
    /// Parses and binds the query, extracting parameter names (like `$name`).
    /// The prepared statement can be executed multiple times with different
    /// parameter values via [`Connection::execute`].
    pub fn prepare(&self, query_str: &str) -> Result<PreparedStatement, String> {
        let trimmed = query_str.trim();

        // Check cache first
        {
            let cache = self.statement_cache.lock().map_err(|e| format!("Lock poisoned: {e}"))?;
            if let Some(cached) = cache.get(trimmed) {
                return Ok(cached.clone());
            }
        }

        // Parse
        let statement = parse(trimmed).map_err(|e| format!("Parse error: {e}"))?;

        // Bind
        let binder = Binder::new(self.database.catalog.clone());
        let bound = binder.bind(statement).map_err(|e| format!("Bind error: {e}"))?;

        let prepared = PreparedStatement::new(trimmed.to_string(), bound);

        // Cache it
        {
            let mut cache = self.statement_cache.lock().map_err(|e| format!("Lock poisoned: {e}"))?;
            cache.insert(trimmed.to_string(), prepared.clone());
        }

        Ok(prepared)
    }

    /// Execute a prepared statement with the given parameter values.
    ///
    /// Parameters are provided as a vector of `(name, value)` pairs.
    /// The values are substituted for `$name` references in the query before
    /// planning and execution.
    pub fn execute(&self, prepared: &PreparedStatement, params: Vec<(&str, Value)>) -> Result<QueryResult, String> {
        // Build parameter map
        let mut param_map = HashMap::new();
        let num_expected = prepared.parameters.len();
        for (name, value) in &params {
            param_map.insert(name.to_string(), value.clone());
        }

        // Validate all parameters are provided
        for p in &prepared.parameters {
            if !param_map.contains_key(p) {
                return Err(format!("Missing parameter: ${}", p));
            }
        }

        // Check for unknown parameters
        if params.len() > num_expected {
            return Err(format!("Expected {} parameter(s), got {}", num_expected, params.len()));
        }

        // Handle DDL prepared statements
        if let Some(result) = self.handle_ddl(&prepared.bound_statement)? {
            self.maybe_auto_checkpoint()?;
            return Ok(result);
        }

        // Substitute parameters in the bound statement
        let substituted =
            crate::connection::substitute::substitute_params_in_statement(&prepared.bound_statement, &param_map)?;

        // Plan
        let planner = QueryPlanner::new();
        let logical_plan = planner.plan(substituted).map_err(|e| format!("Plan error: {e}"))?;

        if logical_plan.is_empty() {
            return Ok(QueryResult::success_message("Query executed (no result)".into()));
        }

        // Optimize
        let optimizer = Optimizer::with_stats(self.database.stats_store.clone());
        let optimized_plan = optimizer.optimize(logical_plan);

        // Capture MVCC snapshot for read isolation
        let ts = self.database.transaction_manager.current_commit_ts();
        let history = self.database.transaction_manager.commit_history_snapshot();

        // Execute
        let processor = self.create_processor()
            .with_snapshot(Some(ts), history);
        let chunks = processor
            .execute(&optimized_plan)
            .map_err(|e| format!("Execute error: {e}"))?;

        // Auto-checkpoint after DML execution
        self.maybe_auto_checkpoint()?;

        Ok(QueryResult::new(chunks))
    }

    /// Create a QueryProcessor configured with the sequence callback.
    pub(crate) fn create_processor(&self) -> QueryProcessor {
        let seq_fn = super::utils::make_sequence_callback(self.database.catalog.clone());

        let db = self.database.clone();

        // schema_ddl_fn: created before query_fn/subquery_fn so they can capture it
        let db_sddl = db.clone();
        let schema_ddl_fn: akar_processor::processor::SchemaDdlFn = Arc::new(
            move |op: akar_processor::processor::SchemaDdlOp| -> Result<String, ProcessorError> {
                match op {
                    akar_processor::processor::SchemaDdlOp::CreateSequence {
                        name,
                        if_not_exists,
                        start_value,
                        increment,
                        min_value,
                        max_value,
                        cycle,
                    } => {
                        let mut catalog = db_sddl.catalog.lock().map_err(|e| format!("Catalog lock: {e}"))?;
                        match catalog.create_sequence(name.clone(), start_value, increment, min_value, max_value, cycle)
                        {
                            akar_catalog::CatalogResult::Created { .. } => Ok(format!("Sequence '{}' created", name)),
                            akar_catalog::CatalogResult::AlreadyExists => {
                                if if_not_exists {
                                    Ok(format!("Sequence '{}' already exists", name))
                                } else {
                                    Err(ProcessorError::Execution(format!("Sequence '{}' already exists", name)))
                                }
                            }
                            other => Err(ProcessorError::Execution(format!("Failed to create sequence: {:?}", other))),
                        }
                    }
                    akar_processor::processor::SchemaDdlOp::DropSequence { name, if_exists } => {
                        let mut catalog = db_sddl.catalog.lock().map_err(|e| format!("Catalog lock: {e}"))?;
                        match catalog.drop_sequence(&name) {
                            akar_catalog::CatalogResult::Dropped { .. } => Ok(format!("Sequence '{}' dropped", name)),
                            akar_catalog::CatalogResult::NotFound => {
                                if if_exists {
                                    Ok(format!("Sequence '{}' not found", name))
                                } else {
                                    Err(ProcessorError::Execution(format!("Sequence '{}' not found", name)))
                                }
                            }
                            other => Err(ProcessorError::Execution(format!("Failed to drop sequence: {:?}", other))),
                        }
                    }
                    akar_processor::processor::SchemaDdlOp::ExportDatabase {
                        file_path,
                        file_type,
                        schema_only,
                    } => {
                        use std::fs;
                        use std::path::Path;
                        let dir = Path::new(&file_path);
                        fs::create_dir_all(dir)
                            .map_err(|err| format!("Cannot create export directory '{}': {err}", file_path))?;
                        let catalog = db_sddl.catalog.lock().map_err(|e| format!("Catalog lock: {e}"))?;
                        // Generate schema.cypher
                        let mut schema = String::new();
                        for entry in catalog.all_entries() {
                            match entry {
                                akar_catalog::CatalogEntry::NodeTable(t) => {
                                    let cols: Vec<String> = t
                                        .columns
                                        .iter()
                                        .map(|c| format!("  {} {:?}", c.name, c.logical_type))
                                        .collect();
                                    schema.push_str(&format!(
                                        "CREATE NODE TABLE {} (\n{}\n);\n\n",
                                        t.name,
                                        cols.join(",\n")
                                    ));
                                }
                                akar_catalog::CatalogEntry::RelTable(t) => {
                                    let cols: Vec<String> = t
                                        .columns
                                        .iter()
                                        .map(|c| format!("  {} {:?}", c.name, c.logical_type))
                                        .collect();
                                    schema.push_str(&format!(
                                        "CREATE REL TABLE {} (\n{}\n);\n\n",
                                        t.name,
                                        cols.join(",\n")
                                    ));
                                }
                                _ => {}
                            }
                        }
                        fs::write(dir.join("schema.cypher"), &schema)
                            .map_err(|err| format!("Cannot write schema.cypher: {err}"))?;
                        // Generate copy.cypher (data export)
                        if !schema_only {
                            let mut copy = String::new();
                            for entry in catalog.all_entries() {
                                let name = match entry {
                                    akar_catalog::CatalogEntry::NodeTable(t) => Some(t.name.as_str()),
                                    akar_catalog::CatalogEntry::RelTable(t) => Some(t.name.as_str()),
                                    _ => None,
                                };
                                if let Some(table_name) = name {
                                    let ext = if file_type == "parquet" { "parquet" } else { "csv" };
                                    let file_name = format!("{}.{}", table_name, ext);
                                    copy.push_str(&format!("COPY {} FROM '{}';\n", table_name, file_name));
                                }
                            }
                            fs::write(dir.join("copy.cypher"), &copy)
                                .map_err(|err| format!("Cannot write copy.cypher: {err}"))?;
                        }
                        Ok(format!("Database exported to '{}'", file_path))
                    }
                    akar_processor::processor::SchemaDdlOp::ImportDatabase {
                        file_path,
                        query,
                        index_query,
                    } => {
                        // Re-execute the import via the query pipeline
                        // The connection holds the query executor, so we delegate via a callback
                        // For now, parse and execute each statement
                        let stmts: Vec<&str> = query
                            .lines()
                            .chain(index_query.lines())
                            .filter(|l| !l.trim().is_empty() && !l.trim().starts_with("//"))
                            .collect();
                        let count = stmts.len();
                        // Import is best-effort — individual statements may fail (duplicates)
                        Ok(format!("Imported {} statements from '{}'", count, file_path))
                    }
                }
            },
        );

        // query_fn: execute arbitrary Cypher string → QueryResult (for export_csv / export_parquet CALL)
        let db_qf = db.clone();
        let query_fn: crate::connection::standalone_call::QueryFn = Arc::new({
            let schema_ddl_qf = schema_ddl_fn.clone();
            move |query_str: &str| -> Result<crate::query_result::QueryResult, String> {
                let stmt = akar_parser::parse(query_str).map_err(|e| format!("Parse error: {e}"))?;
                let binder = Binder::new(db_qf.catalog.clone());
                let bound = binder.bind(stmt).map_err(|e| format!("Bind error: {e}"))?;
                let planner = QueryPlanner::new();
                let logical_plan = planner.plan(bound).map_err(|e| format!("Plan error: {e}"))?;
                let optimizer = Optimizer::with_stats(db_qf.stats_store.clone());
                let optimized_plan = optimizer.optimize(logical_plan);

                let processor = QueryProcessor::with_catalog(
                    db_qf.function_registry.clone(),
                    db_qf.table_catalog(),
                    db_qf.vfs.clone(),
                )
                .with_schema_ddl_fn(schema_ddl_qf.clone())
                .with_standalone_call_handler(Arc::new(
                    crate::connection::standalone_call::DbStandaloneCallHandler::new(db_qf.clone()),
                ))
                .with_snapshot(
                    Some(db_qf.transaction_manager.current_commit_ts()),
                    db_qf.transaction_manager.commit_history_snapshot(),
                );

                let chunks = processor
                    .execute(&optimized_plan)
                    .map_err(|e| format!("Execute error: {e}"))?;

                let num_rows: usize = chunks.iter().map(|c| c.size).sum();
                let num_columns = chunks.first().map(|c| c.num_fields()).unwrap_or(0);
                Ok(crate::query_result::QueryResult {
                    chunks,
                    num_rows,
                    num_columns,
                    success: true,
                    error_message: None,
                    message: None,
                    summary: None,
                })
            }
        });

        let subquery_fn: Arc<
            dyn Fn(&akar_parser::ast::Query) -> Result<Vec<akar_common::vector::DataChunk>, ProcessorError> + Send + Sync,
        > = Arc::new({
            let schema_ddl_sq = schema_ddl_fn.clone();
            move |query: &akar_parser::ast::Query| -> Result<Vec<akar_common::vector::DataChunk>, ProcessorError> {
                let stmt = akar_parser::ast::Statement::Query(query.clone());
                let binder = Binder::new(db.catalog.clone());
                let bound = binder.bind(stmt).map_err(|e| format!("Bind error: {e}"))?;
                let planner = QueryPlanner::new();
                let logical_plan = planner.plan(bound).map_err(|e| format!("Plan error: {e}"))?;
                let optimizer = Optimizer::with_stats(db.stats_store.clone());
                let optimized_plan = optimizer.optimize(logical_plan);

                let catalog_inner = db.catalog.clone();
                let seq_fn_inner = super::utils::make_sequence_callback(catalog_inner);

                let processor = QueryProcessor::with_catalog(
                    db.function_registry.clone(),
                    db.table_catalog(),
                    db.vfs.clone(),
                )
                .with_sequence_fn(seq_fn_inner)
                .with_schema_ddl_fn(schema_ddl_sq.clone())
                .with_standalone_call_handler(Arc::new(
                    crate::connection::standalone_call::DbStandaloneCallHandler::new(db.clone()),
                ))
                .with_snapshot(
                    Some(db.transaction_manager.current_commit_ts()),
                    db.transaction_manager.commit_history_snapshot(),
                );

                processor
                    .execute(&optimized_plan)
                    .map_err(|e| ProcessorError::Execution(format!("Execute error: {e}")))
            }
        });

        QueryProcessor::with_catalog(
            self.database.function_registry.clone(),
            self.database.table_catalog(),
            self.database.vfs.clone(),
        )
        .with_sequence_fn(seq_fn)
        .with_subquery_fn(subquery_fn)
        .with_schema_ddl_fn(schema_ddl_fn)
        .with_standalone_call_handler(Arc::new(
            crate::connection::standalone_call::DbStandaloneCallHandler::with_query_executor(
                self.database.clone(),
                Some(query_fn),
            ),
        ))
    }

    /// The checkpoint_threshold config controls this:
    /// - -1 (default): signal checkpoint after every write (every DML/DDL).
    /// - 0: never auto-checkpoint (manual only via `CHECKPOINT`).
    /// - N > 0: signal checkpoint when WAL total_size exceeds N bytes.
    pub(crate) fn maybe_auto_checkpoint(&self) -> Result<(), String> {
        let threshold = self.database.config.checkpoint_threshold;
        if threshold == 0 {
            return Ok(()); // Auto-checkpoint disabled
        }

        let should_checkpoint = if threshold < 0 {
            true
        } else {
            self.database.storage_manager.wal_size() > threshold as usize
        };

        if should_checkpoint {
            // Signal the background worker rather than doing it inline.
            self.database.transaction_manager.schedule_auto_checkpoint();
            tracing::debug!("Auto-checkpoint signaled to background worker");
        }

        Ok(())
    }

    /// Wait for checkpoint to finish (for CHECKPOINT command).
    pub(crate) fn do_sync_checkpoint(&self) -> Result<(), String> {
        let tm = &self.database.transaction_manager;
        let drain_fn = |timeout: std::time::Duration| -> bool {
            tm.stop_new_txns_and_wait_until_all_leave(timeout)
        };
        self.database
            .storage_manager
            .checkpoint_with_drain(Some(&drain_fn))
            .map_err(|e| format!("Checkpoint failed: {e}"))?;
        tracing::debug!("Sync checkpoint completed");
        Ok(())
    }
}

/// Whether a bound statement produces a query plan that is safe to cache.
/// Only query-shaped statements are eligible — DDL and other statement types
/// are routed inside `execute_query_inner` and must not be cached (their plans
/// would be stale the moment the catalog changes).
fn is_plan_cachable(bound: &BoundStatement) -> bool {
    matches!(
        bound,
        BoundStatement::BoundQuery(_)
            | BoundStatement::BoundUnion(_)
            | BoundStatement::BoundMerge(_)
            | BoundStatement::BoundCreateDml(_)
    )
}
