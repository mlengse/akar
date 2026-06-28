//! DuckDB extension for Kuzu.
//!
//! Provides integration with DuckDB for executing SQL queries
//! and reading DuckDB tables from within Kuzu.
//!
//! Note: This is a connector stub. Actual DuckDB integration
//! would require the `duckdb` crate as a dependency.

use kuzu_extension::{Extension, ExtensionContext};

/// The DuckDB extension enables querying DuckDB from Kuzu.
pub struct DuckDbExtension;

impl DuckDbExtension {
    pub fn new() -> Self {
        Self
    }
}

impl Extension for DuckDbExtension {
    fn name(&self) -> &'static str {
        "DUCKDB"
    }

    fn load(&self, context: &ExtensionContext) -> Result<(), String> {
        use kuzu_function::registry::{ScalarFunction, TableFunction};
        use kuzu_function::registry::UtilityOp;

        context.register_scalar_function(
            "duckdb_query",
            ScalarFunction::Utility { op: UtilityOp::Coalesce },
        );
        context.register_table_function(
            "duckdb_scan",
            TableFunction::ScanJson { path: "duckdb".into() },
        );

        tracing::info!("DuckDB extension loaded: 2 functions registered");
        Ok(())
    }
}

/// A simplified DuckDB connection configuration.
#[derive(Debug, Clone)]
pub struct DuckDbConfig {
    pub database_path: String,
    pub read_only: bool,
    pub thread_count: Option<u32>,
}

impl DuckDbConfig {
    pub fn new(path: &str) -> Self {
        Self {
            database_path: path.to_string(),
            read_only: true,
            thread_count: None,
        }
    }

    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    pub fn with_threads(mut self, threads: u32) -> Self {
        self.thread_count = Some(threads);
        self
    }
}

/// Validate a DuckDB SQL query string (basic).
pub fn validate_query(sql: &str) -> Result<(), String> {
    let trimmed = sql.trim().to_uppercase();
    if trimmed.is_empty() {
        return Err("Empty query".into());
    }
    let valid_keywords = ["SELECT", "EXPLAIN", "DESCRIBE", "SHOW", "PRAGMA", "CALL", "WITH"];
    if !valid_keywords.iter().any(|kw| trimmed.starts_with(kw)) {
        return Err(format!("Query must start with SELECT, EXPLAIN, DESCRIBE, SHOW, PRAGMA, CALL, or WITH. Got: {}", 
            trimmed.split_whitespace().next().unwrap_or("")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_duckdb_config_default() {
        let cfg = DuckDbConfig::new(":memory:");
        assert_eq!(cfg.database_path, ":memory:");
        assert!(cfg.read_only);
    }

    #[test]
    fn test_duckdb_config_custom() {
        let cfg = DuckDbConfig::new("/data/mydb.duckdb")
            .with_read_only(false)
            .with_threads(4);
        assert_eq!(cfg.database_path, "/data/mydb.duckdb");
        assert!(!cfg.read_only);
        assert_eq!(cfg.thread_count, Some(4));
    }

    #[test]
    fn test_validate_valid_query() {
        assert!(validate_query("SELECT * FROM table").is_ok());
        assert!(validate_query("EXPLAIN SELECT 1").is_ok());
        assert!(validate_query("WITH cte AS (SELECT 1) SELECT * FROM cte").is_ok());
    }

    #[test]
    fn test_validate_invalid_query() {
        assert!(validate_query("").is_err());
        assert!(validate_query("INSERT INTO t VALUES (1)").is_err());
        assert!(validate_query("DELETE FROM t").is_err());
    }

    #[test]
    fn test_duckdb_extension_name() {
        let ext = DuckDbExtension::new();
        assert_eq!(ext.name(), "DUCKDB");
    }
}
