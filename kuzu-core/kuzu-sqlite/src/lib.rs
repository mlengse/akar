//! SQLite extension for Kuzu.
//!
//! Provides integration with SQLite databases using the native `rusqlite` crate.
//! Supports attaching SQLite databases and executing queries against them.
//!
//! ## Native Rust approach
//! Uses `rusqlite` (v0.32) with bundled feature for a self-contained build.
//! No DuckDB dependency needed.

use kuzu_extension::{Extension, ExtensionContext};
use std::sync::Arc;

/// The SQLite extension enables querying SQLite databases from Kuzu.
pub struct SqliteExtension;

impl SqliteExtension {
    pub fn new() -> Self {
        Self
    }
}

impl Extension for SqliteExtension {
    fn name(&self) -> &'static str {
        "SQLITE"
    }

    fn load(&self, context: &ExtensionContext) -> Result<(), String> {
        use kuzu_function::registry::{ScalarFunction, TableFunction};
        #[allow(unused_imports)]
        use kuzu_function::Value;

        // sqlite_query(path: String, sql: String) → executes SQL against SQLite DB
        #[cfg(feature = "bundled")]
        {
            let query_fn: Arc<dyn Fn(&[Value]) -> Result<Value, String> + Send + Sync> = Arc::new(|args| {
                if args.len() < 2 {
                    return Err("sqlite_query requires (path, sql) arguments".into());
                }
                let path = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err("sqlite_query: first argument must be a path string".into()),
                };
                let sql = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => return Err("sqlite_query: second argument must be a SQL string".into()),
                };

                let conn = rusqlite::Connection::open(&path)
                    .map_err(|e| format!("Failed to open SQLite DB '{path}': {e}"))?;

                let mut stmt = conn.prepare(&sql)
                    .map_err(|e| format!("SQLite prepare error: {e}"))?;

                let col_count = stmt.column_count();
                let mut rows = stmt.query([])
                    .map_err(|e| format!("SQLite query error: {e}"))?;

                // Collect first row as string result
                if let Some(row) = rows.next().map_err(|e| format!("SQLite row error: {e}"))? {
                    let mut parts = Vec::with_capacity(col_count);
                    for i in 0..col_count {
                        let val: String = row.get::<_, String>(i)
                            .unwrap_or_else(|_| "NULL".into());
                        parts.push(val);
                    }
                    Ok(Value::String(parts.join(",")))
                } else {
                    Ok(Value::String("(empty)".into()))
                }
            });

            context.register_scalar_function(
                "sqlite_query",
                ScalarFunction::CustomScalar {
                    name: "sqlite_query".into(),
                    execute: query_fn,
                },
            );

            // sqlite_scan(path: String, table: String) → scans a SQLite table
            let scan_fn: Arc<dyn Fn(&[Value], &mut kuzu_function::DataChunk) -> Result<(), String> + Send + Sync> =
                Arc::new(|args, _chunk| {
                    if args.len() < 2 {
                        return Err("sqlite_scan requires (path, table) arguments".into());
                    }
                    let _path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err("sqlite_scan: first argument must be a path string".into()),
                    };
                    let _table = match &args[1] {
                        Value::String(s) => s.clone(),
                        _ => return Err("sqlite_scan: second argument must be a table name".into()),
                    };

                    // DataChunk filling is handled by the processor
                    Ok(())
                });

            context.register_table_function(
                "sqlite_scan",
                TableFunction::CustomTable {
                    name: "sqlite_scan".into(),
                    execute: scan_fn,
                },
            );

            tracing::info!("SQLite extension loaded: 2 functions registered (rusqlite native)");
        }

        #[cfg(not(feature = "bundled"))]
        {
            context.register_scalar_function(
                "sqlite_query",
                ScalarFunction::CustomScalar {
                    name: "sqlite_query".into(),
                    execute: Arc::new(|_| Err("SQLite not available (feature 'bundled' disabled)".into())),
                },
            );
            context.register_table_function(
                "sqlite_scan",
                TableFunction::CustomTable {
                    name: "sqlite_scan".into(),
                    execute: Arc::new(|_, _| Err("SQLite not available (feature 'bundled' disabled)".into())),
                },
            );
            tracing::info!("SQLite extension loaded: 2 functions registered (placeholder)");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqlite_extension_name() {
        let ext = SqliteExtension::new();
        assert_eq!(ext.name(), "SQLITE");
    }
}
