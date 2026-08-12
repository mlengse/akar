//! DuckDB extension for Akar.
//!
//! Provides integration with DuckDB for executing SQL queries
//! and reading DuckDB tables from within Akar.
//!
//! Uses the `duckdb` Rust crate (v~1.105) with bundled DuckDB.
//! Gate behind `#[cfg(feature = "bundled")]` for wasm32 compatibility.

pub mod attach_helper;
pub mod connection;
pub mod result_converter;
pub mod type_converter;

use akar_extension::{Extension, ExtensionContext};
use std::sync::Arc;

/// The DuckDB extension enables querying DuckDB from Akar.
pub struct DuckDbExtension;

impl Default for DuckDbExtension {
    fn default() -> Self {
        Self::new()
    }
}

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
        #[allow(unused_imports)]
        use akar_function::Value;
        use akar_function::registry::{ScalarFunction, TableFunction};

        // duckdb_query(sql: String) → executes SQL via DuckDB and returns JSON result
        #[cfg(feature = "bundled")]
        {
            let query_fn: Arc<dyn Fn(&[Value]) -> Result<Value, String> + Send + Sync> = Arc::new(|args| {
                if args.is_empty() {
                    return Err("duckdb_query requires a SQL string argument".into());
                }
                let sql = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err("duckdb_query expects a string argument".into()),
                };

                let manager = match connection::DuckDbManager::shared_in_memory() {
                    Ok(m) => m,
                    Err(e) => return Err(format!("Failed to open DuckDB: {e}")),
                };

                match manager.query_rows(&sql) {
                    Ok(rows) => {
                        // Collect every row (all columns) as strings, not just the first.
                        let mut parts = Vec::new();
                        for row in &rows {
                            for val in row {
                                parts.push(result_converter::duckdb_value_to_string(val));
                            }
                        }
                        if parts.is_empty() {
                            Ok(Value::String("(empty)".into()))
                        } else {
                            Ok(Value::String(parts.join(",")))
                        }
                    }
                    Err(e) => Err(format!("DuckDB query error: {e}")),
                }
            });

            context.register_scalar_function(
                "duckdb_query",
                ScalarFunction::CustomScalar {
                    name: "duckdb_query".into(),
                    execute: query_fn,
                },
            );

            // duckdb_scan(sql: String) → executes SQL and returns table result
            let scan_fn: Arc<dyn Fn(&[Value], &mut akar_function::DataChunk) -> Result<(), String> + Send + Sync> =
                Arc::new(|args, chunk| {
                    if args.is_empty() {
                        return Err("duckdb_scan requires a SQL string argument".into());
                    }
                    let sql = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err("duckdb_scan expects a string argument".into()),
                    };

                    if chunk.size > 0 {
                        return Ok(());
                    }

                    let manager = match connection::DuckDbManager::shared_in_memory() {
                        Ok(m) => m,
                        Err(e) => return Err(format!("Failed to open DuckDB: {e}")),
                    };

                    match manager.query_rows(&sql) {
                        Ok(rows) => {
                            let converted = result_converter::duckdb_results_to_akar(rows)?;
                            if let Some(out) = converted.into_iter().next() {
                                *chunk = out;
                            }
                            Ok(())
                        }
                        Err(e) => Err(format!("DuckDB scan error: {e}")),
                    }
                });

            context.register_table_function(
                "duckdb_scan",
                TableFunction::CustomTable {
                    name: "duckdb_scan".into(),
                    execute: scan_fn,
                },
            );

            tracing::info!("DuckDB extension loaded: 2 functions registered (real callbacks)");
        }

        #[cfg(not(feature = "bundled"))]
        {
            // Placeholder registration when DuckDB crate is unavailable (e.g., wasm32)
            context.register_scalar_function(
                "duckdb_query",
                ScalarFunction::CustomScalar {
                    name: "duckdb_query".into(),
                    execute: Arc::new(|_| Err("DuckDB not available (feature 'bundled' disabled)".into())),
                },
            );
            context.register_table_function(
                "duckdb_scan",
                TableFunction::CustomTable {
                    name: "duckdb_scan".into(),
                    execute: Arc::new(|_, _| Err("DuckDB not available (feature 'bundled' disabled)".into())),
                },
            );
            tracing::info!("DuckDB extension loaded: 2 functions registered (placeholder)");
        }

        Ok(())
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
        return Err(format!(
            "Query must start with SELECT, EXPLAIN, DESCRIBE, SHOW, PRAGMA, CALL, or WITH. Got: {}",
            trimmed.split_whitespace().next().unwrap_or("")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_config_defaults() {
        let cfg = connection::DuckDbConfig::default();
        assert_eq!(cfg.database_path, ":memory:");
    }
}
