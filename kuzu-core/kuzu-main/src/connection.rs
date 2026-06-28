//! Connection — used to execute queries against a Database.

use crate::database::Database;
use crate::query_result::QueryResult;
use kuzu_binder::Binder;
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
        // 1. Parse
        let statement = parse(query_str).map_err(|e| format!("Parse error: {e}"))?;

        // 2. Bind
        let catalog = self.database.catalog.lock().unwrap();
        let binder = Binder::new(catalog.clone());
        let bound = binder.bind(statement).map_err(|e| format!("Bind error: {e}"))?;
        drop(catalog);

        // 3. Plan
        let planner = QueryPlanner::new();
        let logical_plan = planner.plan(bound).map_err(|e| format!("Plan error: {e}"))?;

        // 4. Optimize
        let optimizer = Optimizer::new();
        let optimized_plan = optimizer.optimize(logical_plan);

        // 5. Execute
        let processor = QueryProcessor::new();
        let chunks = processor
            .execute(&optimized_plan)
            .map_err(|e| format!("Execute error: {e}"))?;

        Ok(QueryResult::new(chunks))
    }
}
