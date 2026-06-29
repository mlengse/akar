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
            BoundStatement::BoundAlterTable(a) => {
                tracing::info!("ALTER TABLE '{}'", a.table_name);
                let mut catalog = self.database.catalog.lock().unwrap();
                match &a.action {
                    kuzu_parser::ast::AlterAction::AddColumn { name, type_name } => {
                        let logical_type = kuzu_binder::Binder::parse_type(type_name)
                            .map_err(|e| format!("ALTER ADD: {e}"))?;
                        catalog.add_column(&a.table_name, kuzu_catalog::CatalogColumn {
                            name: name.clone(),
                            logical_type,
                            is_primary_key: false,
                            default_value: None,
                        }).map_err(|e| format!("ALTER ADD: {e}"))?;
                        Ok(Some(QueryResult::success_message(format!(
                            "Column '{}' added to table '{}'", name, a.table_name
                        ))))
                    }
                    kuzu_parser::ast::AlterAction::DropColumn { name } => {
                        catalog.drop_column(&a.table_name, name)
                            .map_err(|e| format!("ALTER DROP: {e}"))?;
                        Ok(Some(QueryResult::success_message(format!(
                            "Column '{}' dropped from table '{}'", name, a.table_name
                        ))))
                    }
                    kuzu_parser::ast::AlterAction::RenameColumn { old_name, new_name } => {
                        catalog.rename_column(&a.table_name, old_name, new_name)
                            .map_err(|e| format!("ALTER RENAME COLUMN: {e}"))?;
                        Ok(Some(QueryResult::success_message(format!(
                            "Column '{}' renamed to '{}' in table '{}'",
                            old_name, new_name, a.table_name
                        ))))
                    }
                    kuzu_parser::ast::AlterAction::RenameTable { new_name } => {
                        catalog.rename_table(&a.table_name, new_name)
                            .map_err(|e| format!("ALTER RENAME TABLE: {e}"))?;
                        Ok(Some(QueryResult::success_message(format!(
                            "Table '{}' renamed to '{}'", a.table_name, new_name
                        ))))
                    }
                }
            }
            BoundStatement::BoundCopyFrom(c) => {
                tracing::info!("COPY FROM '{}' from '{}'", c.table_name, c.file_path);
                // Fall through to the pipeline (plan → optimize → execute)
                Ok(None)
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

// ─── Integration tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::database::SystemConfig;
    use kuzu_common::types::Value;

    /// Create a temporary Database and Connection for testing.
    fn setup_db() -> (tempfile::TempDir, Arc<Database>, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_db");
        let config = SystemConfig::default();
        let database = Arc::new(Database::new(db_path, config).unwrap());
        let conn = Connection::new(&database);
        (dir, database, conn)
    }

    /// Helper: execute a query and return whether it succeeded.
    fn exec_ok(conn: &Connection, sql: &str) -> Result<String, String> {
        conn.query(sql).map(|r| r.to_string())
    }

    /// Helper: extract all values from the first column of a query result.
    fn query_column(conn: &Connection, sql: &str) -> Vec<Value> {
        let result = conn.query(sql).unwrap();
        result.chunks.iter().flat_map(|c| {
            (0..c.size).filter_map(|i| {
                c.fields.first().and_then(|f| f.get_value(i))
            })
        }).collect()
    }

    #[test]
    fn test_copy_csv_with_header() {
        let (dir, _db, conn) = setup_db();
        let db_path = dir.path().join("test_db");
        let _ = &db_path; // keep db path alive

        exec_ok(&conn, "CREATE NODE TABLE Person(name STRING, age INT64, score DOUBLE, active BOOL, PRIMARY KEY (name))").unwrap();

        let csv_path = dir.path().join("people.csv");
        std::fs::write(&csv_path,
            "name,age,score,active\nAlice,30,95.5,true\nBob,25,87.3,false\nCharlie,35,91.2,true\n"
        ).unwrap();

        let file_path = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY Person FROM '{file_path}' (HEADER true)")).unwrap();

        let names = query_column(&conn, "MATCH (n:Person) RETURN n.name ORDER BY n.name");
        let extracted: Vec<String> = names.iter().filter_map(|v| {
            if let Value::String(s) = v { Some(s.clone()) } else { None }
        }).collect();
        assert_eq!(extracted, vec!["Alice", "Bob", "Charlie"]);
    }

    #[test]
    fn test_copy_csv_no_header() {
        let (_dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE Person(name STRING, age INT64, score DOUBLE, active BOOL, PRIMARY KEY (name))").unwrap();

        let csv_path = _dir.path().join("noheader.csv");
        std::fs::write(&csv_path, "Alice,30,95.5,true\nBob,25,87.3,false\n").unwrap();

        let file_path = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY Person FROM '{file_path}' (HEADER false)")).unwrap();

        // Verify: MATCH should return 2 rows (even if column projection is raw)
        let names = query_column(&conn, "MATCH (n:Person) RETURN n.name");
        assert_eq!(names.len(), 2, "Expected 2 rows in MATCH result");
    }

    #[test]
    fn test_copy_csv_custom_delimiter() {
        let (_dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE Item(name STRING, price DOUBLE, PRIMARY KEY (name))").unwrap();

        let csv_path = _dir.path().join("items.csv");
        std::fs::write(&csv_path, "name|price\nWidget|19.99\nGadget|29.99\n").unwrap();

        let file_path = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY Item FROM '{file_path}' (HEADER true, DELIM '|')")).unwrap();

        // Verify 2 rows were inserted
        let names = query_column(&conn, "MATCH (n:Item) RETURN n.name");
        assert_eq!(names.len(), 2, "Expected 2 rows after pipe-delimited COPY");
    }

    #[test]
    fn test_copy_csv_file_not_found() {
        let (dir, _db, conn) = setup_db();
        let _ = &dir;

        exec_ok(&conn, "CREATE NODE TABLE T(name STRING, PRIMARY KEY (name))").unwrap();

        let result = exec_ok(&conn, "COPY T FROM 'nonexistent.csv' (HEADER true)");
        assert!(result.is_err(), "Expected file not found error");
        let err = result.unwrap_err();
        assert!(err.contains("not found"), "Expected file not found error, got: {err}");
    }

    #[test]
    fn test_copy_csv_type_mismatch() {
        let (dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE T(name STRING, age INT64, PRIMARY KEY (name))").unwrap();

        let csv_path = dir.path().join("bad.csv");
        std::fs::write(&csv_path, "name,age\nAlice,not_a_number\n").unwrap();

        let file_path = csv_path.to_string_lossy().replace('\\', "/");
        let result = exec_ok(&conn, &format!("COPY T FROM '{file_path}' (HEADER true)"));
        assert!(result.is_err(), "Expected type coercion error");
        let err = result.unwrap_err();
        assert!(err.contains("INT64") || err.contains("parse"),
            "Expected type error, got: {err}");
    }

    #[test]
    fn test_copy_csv_column_count_mismatch() {
        let (dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE T(name STRING, age INT64, PRIMARY KEY (name))").unwrap();

        let csv_path = dir.path().join("bad_cols.csv");
        std::fs::write(&csv_path, "name,age,extra\nAlice,30,oops\n").unwrap();

        // Without DELIM option, the binder skips header validation.
        // The CSV reader will detect the mismatch and error at read time.
        let file_path = csv_path.to_string_lossy().replace('\\', "/");
        let result = exec_ok(&conn, &format!("COPY T FROM '{file_path}' (HEADER true)"));
        assert!(result.is_err(), "Expected column count error");
        let err = result.unwrap_err();
        assert!(err.contains("Column count mismatch") || err.contains("match"),
            "Expected column count error, got: {err}");
    }

    #[test]
    fn test_copy_parquet_roundtrip() {
        use arrow::array::*;
        use arrow::datatypes::{DataType, Field, Schema};
        use std::sync::Arc;

        let (dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE Person(name STRING, age INT64, score DOUBLE, active BOOL, PRIMARY KEY (name))").unwrap();

        let pq_path = dir.path().join("data.parquet");
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", DataType::Utf8, false),
            Field::new("age", DataType::Int64, false),
            Field::new("score", DataType::Float64, false),
            Field::new("active", DataType::Boolean, false),
        ]));

        let batch = arrow::record_batch::RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["Alice", "Bob"])),
                Arc::new(Int64Array::from(vec![30i64, 25])),
                Arc::new(Float64Array::from(vec![95.5, 87.3])),
                Arc::new(BooleanArray::from(vec![true, false])),
            ],
        ).unwrap();

        let file = std::fs::File::create(&pq_path).unwrap();
        let mut writer = parquet::arrow::ArrowWriter::try_new(file, batch.schema(), None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();

        let file_path = pq_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY Person FROM '{file_path}'")).unwrap();

        // Verify 2 rows were inserted via Parquet
        let names = query_column(&conn, "MATCH (n:Person) RETURN n.name");
        assert_eq!(names.len(), 2, "Expected 2 rows from Parquet COPY");
    }

    #[test]
    fn test_copy_csv_tab_delimiter() {
        let (dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE T(a STRING, b INT64, PRIMARY KEY (a))").unwrap();

        let csv_path = dir.path().join("data.tsv");
        std::fs::write(&csv_path, "a\tb\nx\t10\ny\t20\n").unwrap();

        // .tsv extension triggers tab delimiter in PhysicalCopyFrom.
        // Without a DELIM option, the binder skips header validation.
        let file_path = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY T FROM '{file_path}' (HEADER true)")).unwrap();

        // Verify 2 rows were inserted via TSV
        let vals = query_column(&conn, "MATCH (n:T) RETURN n.b");
        assert_eq!(vals.len(), 2, "Expected 2 rows from TSV COPY");
    }

    #[test]
    fn test_copy_empty_csv() {
        let (dir, _db, conn) = setup_db();

        exec_ok(&conn, "CREATE NODE TABLE T(name STRING, PRIMARY KEY (name))").unwrap();

        let csv_path = dir.path().join("empty.csv");
        std::fs::write(&csv_path, "name\n").unwrap(); // header only

        let file_path = csv_path.to_string_lossy().replace('\\', "/");
        exec_ok(&conn, &format!("COPY T FROM '{file_path}' (HEADER true)")).unwrap();

        let vals = query_column(&conn, "MATCH (n:T) RETURN n.name");
        assert!(vals.is_empty(), "Expected no rows in empty CSV");
    }
}
