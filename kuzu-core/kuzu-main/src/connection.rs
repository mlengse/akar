//! Connection — used to execute queries against a Database.
//!
//! Manages the full query lifecycle: parse → bind → plan → optimize → execute.
//! DDL statements (CREATE/DROP TABLE) update the catalog directly and return
//! a message result. DML statements (MATCH/RETURN) produce DataChunk results.

use crate::database::Database;
use crate::query_result::QueryResult;
use kuzu_binder::Binder;
use kuzu_binder::bound_statement::BoundStatement;
use kuzu_optimizer::Optimizer;
use kuzu_parser::parse;
use kuzu_planner::QueryPlanner;
use kuzu_processor::QueryProcessor;
use std::sync::Arc;

/// A connection to the database for executing queries.
pub struct Connection {
    database: Arc<Database>,
}

impl Connection {
    pub fn new(database: &Arc<Database>) -> Self {
        Self {
            database: database.clone(),
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
        match &bound {
            BoundStatement::BoundCreateNodeTable(t) => {
                tracing::info!("Created node table '{}'", t.name);
                return Ok(QueryResult::success_message(format!(
                    "Node table '{}' created", t.name
                )));
            }
            BoundStatement::BoundCreateRelTable(t) => {
                tracing::info!("Created rel table '{}'", t.name);
                return Ok(QueryResult::success_message(format!(
                    "Rel table '{}' created", t.name
                )));
            }
            BoundStatement::BoundDropTable(t) => {
                tracing::info!("Dropped table '{}'", t.name);
                return Ok(QueryResult::success_message(format!(
                    "Table '{}' dropped", t.name
                )));
            }
            BoundStatement::BoundQuery(_) => {
                // Continue to planning + execution
            }
        }

        // 4. Plan
        let planner = QueryPlanner::new();
        let logical_plan = planner.plan(bound)
            .map_err(|e| format!("Plan error: {e}"))?;

        if logical_plan.is_empty() {
            return Ok(QueryResult::success_message("Query executed (no result)".into()));
        }

        // 5. Optimize
        let optimizer = Optimizer::new();
        let optimized_plan = optimizer.optimize(logical_plan);

        // 6. Execute
        let processor = QueryProcessor::new();
        let chunks = processor
            .execute(&optimized_plan)
            .map_err(|e| format!("Execute error: {e}"))?;

        Ok(QueryResult::new(chunks))
    }
}
