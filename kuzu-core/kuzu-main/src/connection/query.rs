use super::Connection;
use crate::prepared_statement::PreparedStatement;
use crate::query_result::QueryResult;
use kuzu_binder::Binder;
use kuzu_binder::bound_statement::BoundStatement;
use kuzu_common::types::Value;
use kuzu_optimizer::Optimizer;
use kuzu_parser::parse;
use kuzu_planner::QueryPlanner;
use kuzu_processor::QueryProcessor;
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

        // Handle BEGIN TRANSACTION / COMMIT / ROLLBACK explicitly
        let upper = trimmed.to_uppercase();
        if upper == "BEGIN" || upper == "BEGIN TRANSACTION" || upper == "BEGIN WORK" {
            let txn = self.begin_write_txn()?;
            return Ok(QueryResult::success_message(format!(
                "Transaction started (txn#{})",
                txn.transaction_id
            )));
        }
        if upper == "COMMIT" || upper == "COMMIT TRANSACTION" || upper == "COMMIT WORK" {
            // Find the active write txn — in this simplified model, we look for
            // the most recently started write transaction.
            let tm = &self.database.transaction_manager;
            // We need to know which txn to commit. For now, find it from resources.
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
            // Get the first active write transaction from TM
            if let Ok(mut active) = tm.active_snapshot() {
                for txn_id in &txn_ids {
                    if let Some(txn) = active.remove(txn_id) {
                        let mut t = txn;
                        self.commit_write_txn(&mut t)?;
                        return Ok(QueryResult::success_message("Transaction committed".into()));
                    }
                }
            }
            return Err("No active transaction to commit".into());
        }
        if upper == "ROLLBACK" || upper == "ROLLBACK TRANSACTION" || upper == "ROLLBACK WORK" {
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
                        return Ok(QueryResult::success_message("Transaction rolled back".into()));
                    }
                }
            }
            return Err("No active transaction to rollback".into());
        }

        // Handle CHECKPOINT explicitly
        if upper == "CHECKPOINT" {
            self.do_sync_checkpoint()?;
            return Ok(QueryResult::success_message("Checkpoint completed".into()));
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

        // 1. Parse
        let statement = parse(trimmed).map_err(|e| format!("Parse error: {e}"))?;

        // 2. Bind (using shared catalog Arc — DDL mutations persist)
        let binder = Binder::new(self.database.catalog.clone());
        let bound = binder.bind(statement).map_err(|e| format!("Bind error: {e}"))?;

        // 3. Determine if this is a write operation and begin a transaction if so.
        //    Only wrap in a transaction when concurrent_writes is enabled.
        //    In single-writer mode, the old direct-write path is kept for
        //    backward compatibility (no WAL commit records for pure DDL).
        let concurrent_mode = self.database.transaction_manager.allow_concurrent_writes();
        let is_write = concurrent_mode && Connection::is_write_statement(&bound);
        let mut txn_opt: Option<kuzu_transaction::Transaction> =
            if is_write { Some(self.begin_write_txn()?) } else { None };

        // 4. Execute the query within a transaction scope
        let query_result = self.execute_query_inner(&bound, txn_opt.as_mut());

        // 5. Commit or rollback based on result
        match (is_write, &query_result) {
            (true, Ok(_)) => {
                if let Some(ref mut txn) = txn_opt {
                    self.commit_write_txn(txn)?;
                }
            }
            (true, Err(e)) => {
                if let Some(ref mut txn) = txn_opt {
                    let _records = self.rollback_write_txn(txn);
                    tracing::warn!("Transaction rolled back due to error: {e}");
                }
            }
            _ => {}
        }

        query_result
    }

    /// Inner query execution (after parsing and binding, before commit/rollback).
    pub(crate) fn execute_query_inner(
        &self,
        bound: &BoundStatement,
        txn_opt: Option<&mut kuzu_transaction::Transaction>,
    ) -> Result<QueryResult, String> {
        // Route: DDL vs DML (handle_ddl returns Some for DDL, None for DML)
        if let Some(result) = self.handle_ddl(bound)? {
            // DDL may have modified the catalog; checkpoint if needed.
            self.maybe_auto_checkpoint()?;
            return Ok(result);
        }

        // Lock all tables required by this query if in a write transaction
        if let Some(ref txn) = txn_opt {
            let write_tables = Connection::extract_write_tables(bound);
            for tid in write_tables {
                self.database.transaction_manager.lock_table(txn.transaction_id, tid)?;
            }
        }

        // Plan
        let planner = QueryPlanner::new();
        let logical_plan = planner.plan(bound.clone()).map_err(|e| format!("Plan error: {e}"))?;

        if logical_plan.is_empty() {
            return Ok(QueryResult::success_message("Query executed (no result)".into()));
        }

        // Optimize
        let optimizer = Optimizer::with_stats(self.database.stats_store.clone());
        let optimized_plan = optimizer.optimize(logical_plan);

        // Execute
        let processor = self.create_processor();
        let chunks = processor
            .execute(&optimized_plan)
            .map_err(|e| format!("Execute error: {e}"))?;

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
            let cache = self.statement_cache.lock().unwrap();
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
            let mut cache = self.statement_cache.lock().unwrap();
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

        // Execute
        let processor = self.create_processor();
        let chunks = processor
            .execute(&optimized_plan)
            .map_err(|e| format!("Execute error: {e}"))?;

        // Auto-checkpoint after DML execution
        self.maybe_auto_checkpoint()?;

        Ok(QueryResult::new(chunks))
    }

    /// Create a QueryProcessor configured with the sequence callback.
    pub(crate) fn create_processor(&self) -> QueryProcessor {
        let catalog = self.database.catalog.clone();
        let seq_fn: Arc<dyn Fn(&str, bool) -> Result<Value, String> + Send + Sync> =
            Arc::new(move |seq_name: &str, is_nextval: bool| -> Result<Value, String> {
                let mut catalog = catalog.lock().map_err(|e| format!("Catalog lock error: {e}"))?;
                if is_nextval {
                    match catalog.get_sequence_mut(seq_name) {
                        Some(entry) => {
                            let val = entry.next_k_val(1);
                            Ok(Value::Int64(val))
                        }
                        None => Err(format!("Sequence '{}' not found", seq_name)),
                    }
                } else {
                    match catalog.get_sequence(seq_name) {
                        Some(entry) => Ok(Value::Int64(entry.curr_val())),
                        None => Err(format!("Sequence '{}' not found", seq_name)),
                    }
                }
            });

        let db = self.database.clone();
        let subquery_fn: Arc<
            dyn Fn(&kuzu_parser::ast::Query) -> Result<Vec<kuzu_common::vector::DataChunk>, String> + Send + Sync,
        > = Arc::new(
            move |query: &kuzu_parser::ast::Query| -> Result<Vec<kuzu_common::vector::DataChunk>, String> {
                let stmt = kuzu_parser::ast::Statement::Query(query.clone());
                let binder = Binder::new(db.catalog.clone());
                let bound = binder.bind(stmt).map_err(|e| format!("Bind error: {e}"))?;
                let planner = QueryPlanner::new();
                let logical_plan = planner.plan(bound).map_err(|e| format!("Plan error: {e}"))?;
                let optimizer = Optimizer::with_stats(db.stats_store.clone());
                let optimized_plan = optimizer.optimize(logical_plan);

                let catalog_inner = db.catalog.clone();
                let seq_fn_inner: Arc<dyn Fn(&str, bool) -> Result<Value, String> + Send + Sync> =
                    Arc::new(move |seq_name: &str, is_nextval: bool| -> Result<Value, String> {
                        let mut cat = catalog_inner.lock().map_err(|e| format!("Catalog lock error: {e}"))?;
                        if is_nextval {
                            match cat.get_sequence_mut(seq_name) {
                                Some(entry) => Ok(Value::Int64(entry.next_k_val(1))),
                                None => Err(format!("Sequence '{}' not found", seq_name)),
                            }
                        } else {
                            match cat.get_sequence(seq_name) {
                                Some(entry) => Ok(Value::Int64(entry.curr_val())),
                                None => Err(format!("Sequence '{}' not found", seq_name)),
                            }
                        }
                    });

                let processor = QueryProcessor::with_catalog(
                    db.function_registry.clone(),
                    db.storage_manager.table_catalog(),
                    db.vfs.clone(),
                )
                .with_sequence_fn(seq_fn_inner);
                // Note: Not attaching subquery_fn recursively to avoid complex ARC dependencies for now.

                processor
                    .execute(&optimized_plan)
                    .map_err(|e| format!("Execute error: {e}"))
            },
        );

        QueryProcessor::with_catalog(
            self.database.function_registry.clone(),
            self.database.storage_manager.table_catalog(),
            self.database.vfs.clone(),
        )
        .with_sequence_fn(seq_fn)
        .with_subquery_fn(subquery_fn)
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
    fn do_sync_checkpoint(&self) -> Result<(), String> {
        self.database
            .storage_manager
            .checkpoint_with_drain()
            .map_err(|e| format!("Checkpoint failed: {e}"))?;
        tracing::debug!("Sync checkpoint completed");
        Ok(())
    }
}
