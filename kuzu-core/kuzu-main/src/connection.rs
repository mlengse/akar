//! Connection — used to execute queries against a Database.
//!
//! Manages the full query lifecycle: parse → bind → plan → optimize → execute.
//! DDL statements (CREATE/DROP TABLE) update the catalog directly and return
//! a message result. DML statements (MATCH/RETURN) produce DataChunk results.
//!
//! Supports prepared statements via `prepare()` and `execute()` for
//! parameterized queries.

use crate::database::Database;
use crate::prepared_statement::{PreparedStatement, substitute_params};
use crate::query_result::QueryResult;
use kuzu_binder::Binder;
use kuzu_binder::bound_statement::{
    BoundStatement, BoundClause, BoundExpression,
    BoundQuery, BoundReturnClause, BoundWhereClause,
};
use kuzu_common::types::Value;
use kuzu_optimizer::Optimizer;
use kuzu_parser::parse;
use kuzu_planner::QueryPlanner;
use kuzu_processor::QueryProcessor;
use kuzu_storage::table::ColumnDefinition;
use std::collections::HashMap;
use std::sync::Arc;

/// A connection to the database for executing queries.
pub struct Connection {
    database: Arc<Database>,
    /// Cache of prepared statements (query → PreparedStatement).
    statement_cache: std::sync::Mutex<HashMap<String, PreparedStatement>>,
}

impl Connection {
    pub fn new(database: &Arc<Database>) -> Self {
        Self {
            database: database.clone(),
            statement_cache: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Execute a Cypher query and return the result.
    pub fn query(&self, query_str: &str) -> Result<QueryResult, String> {
        let trimmed = query_str.trim();

        // Skip empty queries
        if trimmed.is_empty() {
            return Ok(QueryResult::new(Vec::new()));
        }

        // 1. Parse
        let statement = parse(trimmed)
            .map_err(|e| format!("Parse error: {e}"))?;

        // 2. Bind (using shared catalog Arc — DDL mutations persist)
        let binder = Binder::new(self.database.catalog.clone());
        let bound = binder.bind(statement)
            .map_err(|e| format!("Bind error: {e}"))?;

        // 3. Route: DDL vs DML
        if let Some(result) = self.handle_ddl(&bound)? {
            return Ok(result);
        }

        // 4. Plan
        let planner = QueryPlanner::new();
        let logical_plan = planner.plan(bound)
            .map_err(|e| format!("Plan error: {e}"))?;

        if logical_plan.is_empty() {
            return Ok(QueryResult::success_message("Query executed (no result)".into()));
        }

        // 5. Optimize
        let optimizer = Optimizer::with_stats(self.database.stats_store.clone());
        let optimized_plan = optimizer.optimize(logical_plan);

        // 6. Execute
        let processor = QueryProcessor::with_catalog(
            self.database.function_registry.clone(),
            self.database.storage_manager.table_catalog(),
        );
        let chunks = processor
            .execute(&optimized_plan)
            .map_err(|e| format!("Execute error: {e}"))?;

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
        let statement = parse(trimmed)
            .map_err(|e| format!("Parse error: {e}"))?;

        // Bind
        let binder = Binder::new(self.database.catalog.clone());
        let bound = binder.bind(statement)
            .map_err(|e| format!("Bind error: {e}"))?;

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
    pub fn execute(
        &self,
        prepared: &PreparedStatement,
        params: Vec<(&str, Value)>,
    ) -> Result<QueryResult, String> {
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
            return Err(format!(
                "Expected {} parameter(s), got {}",
                num_expected,
                params.len()
            ));
        }

        // Handle DDL prepared statements
        if let Some(result) = self.handle_ddl(&prepared.bound_statement)? {
            return Ok(result);
        }

        // Substitute parameters in the bound statement
        let substituted = substitute_params_in_statement(
            &prepared.bound_statement,
            &param_map,
        )?;

        // Plan
        let planner = QueryPlanner::new();
        let logical_plan = planner.plan(substituted)
            .map_err(|e| format!("Plan error: {e}"))?;

        if logical_plan.is_empty() {
            return Ok(QueryResult::success_message("Query executed (no result)".into()));
        }

        // Optimize
        let optimizer = Optimizer::with_stats(self.database.stats_store.clone());
        let optimized_plan = optimizer.optimize(logical_plan);

        // Execute
        let processor = QueryProcessor::with_catalog(
            self.database.function_registry.clone(),
            self.database.storage_manager.table_catalog(),
        );
        let chunks = processor
            .execute(&optimized_plan)
            .map_err(|e| format!("Execute error: {e}"))?;

        Ok(QueryResult::new(chunks))
    }

    /// Handle DDL statements by checking the bound statement type.
    /// Returns `Ok(Some(result))` if DDL, `Ok(None)` if DML (continue).
    fn handle_ddl(&self, bound: &BoundStatement) -> Result<Option<QueryResult>, String> {
        match bound {
            BoundStatement::BoundCreateNodeTable(t) => {
                // Also create a storage table with in-memory data capacity
                let columns: Vec<ColumnDefinition> = t.columns.iter().map(|c| {
                    ColumnDefinition {
                        name: c.name.clone(),
                        logical_type: c.logical_type,
                        is_primary_key: c.is_primary_key,
                    }
                }).collect();
                self.database.storage_manager.create_node_table(t.name.clone(), columns);
                tracing::info!("Created node table '{}'", t.name);
                Ok(Some(QueryResult::success_message(format!(
                    "Node table '{}' created", t.name
                ))))
            }
            BoundStatement::BoundCreateRelTable(t) => {
                // Create a storage rel table
                let columns: Vec<ColumnDefinition> = t.columns.iter().map(|c| {
                    ColumnDefinition {
                        name: c.name.clone(),
                        logical_type: c.logical_type,
                        is_primary_key: c.is_primary_key,
                    }
                }).collect();
                let src_id = 0; // src table ID resolved during binding
                let dst_id = 0;
                self.database.storage_manager.create_rel_table(t.name.clone(), src_id, dst_id, columns);
                tracing::info!("Created rel table '{}'", t.name);
                Ok(Some(QueryResult::success_message(format!(
                    "Rel table '{}' created", t.name
                ))))
            }
            BoundStatement::BoundDropTable(t) => {
                tracing::info!("Dropped table '{}'", t.name);
                Ok(Some(QueryResult::success_message(format!(
                    "Table '{}' dropped", t.name
                ))))
            }
            BoundStatement::BoundCopyFrom(c) => {
                tracing::info!("COPY FROM '{}' from '{}'", c.table_name, c.file_path);
                Ok(Some(QueryResult::success_message(format!(
                    "Copy from '{}' into table '{}'", c.file_path, c.table_name
                ))))
            }
            BoundStatement::BoundQuery(_) => Ok(None),
        }
    }

    /// Clear the prepared statement cache.
    pub fn clear_cache(&self) {
        if let Ok(mut cache) = self.statement_cache.lock() {
            cache.clear();
        }
    }

    /// Number of cached prepared statements.
    pub fn cache_size(&self) -> usize {
        self.statement_cache.lock().map(|c| c.len()).unwrap_or(0)
    }
}

/// Substitute parameter references in a BoundStatement with concrete values.
fn substitute_params_in_statement(
    bound: &BoundStatement,
    params: &HashMap<String, Value>,
) -> Result<BoundStatement, String> {
    match bound {
        BoundStatement::BoundQuery(q) => {
            let mut new_clauses = Vec::new();
            for clause in &q.clauses {
                let new_clause = match clause {
                    BoundClause::BoundReturn(r) => {
                        let new_exprs: Result<Vec<_>, _> = r.expressions.iter()
                            .map(|e| substitute_in_bound_expr(e, params))
                            .collect();
                        BoundClause::BoundReturn(
                            BoundReturnClause { expressions: new_exprs? }
                        )
                    }
                    BoundClause::BoundWhere(w) => {
                        let new_expr = substitute_in_bound_expr(&w.expression, params)?;
                        BoundClause::BoundWhere(
                            BoundWhereClause { expression: new_expr }
                        )
                    }
                    other => other.clone(),
                };
                new_clauses.push(new_clause);
            }
            Ok(BoundStatement::BoundQuery(
                BoundQuery { variables: q.variables.clone(), clauses: new_clauses }
            ))
        }
        other => Ok(other.clone()),
    }
}

fn substitute_in_bound_expr(
    expr: &BoundExpression,
    params: &HashMap<String, Value>,
) -> Result<BoundExpression, String> {
    let new_expr = substitute_params(&expr.expression, params)?;
    Ok(BoundExpression {
        expression: new_expr,
        resolved_type: expr.resolved_type,
        is_constant: expr.is_constant,
    })
}
