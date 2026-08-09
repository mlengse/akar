//! SQLite extension for Akar.
//!
//! Provides integration with SQLite databases using the native `rusqlite` crate.
//! Supports attaching SQLite databases and executing queries against them.
//!
//! ## Native Rust approach
//! Uses `rusqlite` (v0.40) with bundled feature for a self-contained build.
//! No DuckDB dependency needed.

use akar_extension::{Extension, ExtensionContext};
use std::sync::Arc;

/// Convert a single SQLite value to its string representation.
///
/// Unlike the previous `row.get::<_, String>` which failed on any non-text
/// column (yielding a misleading "NULL"), this handles every SQLite type.
#[cfg(feature = "bundled")]
fn sqlite_value_to_string(val: &rusqlite::types::Value) -> String {
    match val {
        rusqlite::types::Value::Null => "NULL".into(),
        rusqlite::types::Value::Integer(i) => i.to_string(),
        rusqlite::types::Value::Real(f) => f.to_string(),
        rusqlite::types::Value::Text(s) => s.clone(),
        rusqlite::types::Value::Blob(b) => format!("<blob:{} bytes>", b.len()),
    }
}

/// The SQLite extension enables querying SQLite databases from Akar.
pub struct SqliteExtension;

impl Default for SqliteExtension {
    fn default() -> Self {
        Self::new()
    }
}

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
        #[allow(unused_imports)]
        use akar_function::Value;
        use akar_function::registry::{ScalarFunction, TableFunction};

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

                let conn =
                    rusqlite::Connection::open(&path).map_err(|e| format!("Failed to open SQLite DB '{path}': {e}"))?;

                let mut stmt = conn.prepare(&sql).map_err(|e| format!("SQLite prepare error: {e}"))?;

                let col_count = stmt.column_count();
                let mut rows = stmt.query([]).map_err(|e| format!("SQLite query error: {e}"))?;

                // Collect every row (all columns) as strings, not just the first.
                let mut parts = Vec::new();
                while let Some(row) = rows.next().map_err(|e| format!("SQLite row error: {e}"))? {
                    for i in 0..col_count {
                        let val: rusqlite::types::Value = row
                            .get::<_, rusqlite::types::Value>(i)
                            .unwrap_or(rusqlite::types::Value::Null);
                        parts.push(sqlite_value_to_string(&val));
                    }
                }
                if parts.is_empty() {
                    Ok(Value::String("(empty)".into()))
                } else {
                    Ok(Value::String(parts.join(",")))
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
            let scan_fn: Arc<dyn Fn(&[Value], &mut akar_function::DataChunk) -> Result<(), String> + Send + Sync> =
                Arc::new(|args, chunk| {
                    if args.len() < 2 {
                        return Err("sqlite_scan requires (path, table) arguments".into());
                    }
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err("sqlite_scan: first argument must be a path string".into()),
                    };
                    let table = match &args[1] {
                        Value::String(s) => s.clone(),
                        _ => return Err("sqlite_scan: second argument must be a table name".into()),
                    };

                    if chunk.size > 0 {
                        return Ok(());
                    }

                    let conn = rusqlite::Connection::open(&path)
                        .map_err(|e| format!("Failed to open SQLite DB '{path}': {e}"))?;

                    let sql = format!("SELECT * FROM {}", akar_common::extension_utils::quote_sql_table_name(&table));
                    let mut stmt = conn.prepare(&sql).map_err(|e| format!("SQLite prepare error: {e}"))?;
                    let col_count = stmt.column_count();
                    let names: Vec<String> = (0..col_count)
                        .map(|i| stmt.column_name(i).map(str::to_string).unwrap_or_default())
                        .collect();
                    let mut rows = stmt.query([]).map_err(|e| format!("SQLite query error: {e}"))?;

                    let mut columns: Vec<Vec<Option<String>>> = vec![Vec::new(); col_count];
                    while let Some(row) = rows.next().map_err(|e| format!("SQLite row error: {e}"))? {
                        for i in 0..col_count {
                            let val: rusqlite::types::Value = row
                                .get::<_, rusqlite::types::Value>(i)
                                .unwrap_or(rusqlite::types::Value::Null);
                            columns[i].push(Some(sqlite_value_to_string(&val)));
                        }
                    }

                    chunk.fields.clear();
                    chunk.field_types.clear();
                    chunk.field_names.clear();
                    for (col, name) in columns.into_iter().zip(names) {
                        chunk.fields.push(
                            std::sync::Arc::new(arrow::array::StringArray::from_iter(
                                col.iter().map(|o| o.as_deref()),
                            )) as arrow::array::ArrayRef,
                        );
                        chunk.field_types.push(akar_common::types::PhysicalTypeID::String);
                        chunk.field_names.push(name);
                    }
                    chunk.size = chunk.fields.first().map(|f| f.len()).unwrap_or(0);
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
