//! PostgreSQL extension for Kuzu.
//!
//! Provides integration with PostgreSQL databases using the native
//! `tokio-postgres` crate with a synchronous wrapper via `block_on`.
//!
//! ## Native Rust approach
//! Uses `tokio-postgres` (v0.7) with `futures::executor::block_on()` to bridge
//! the async PostgreSQL client with Kuzu's synchronous runtime.
//! Most complex extension due to catalog binding + table enumeration + type mapping.

use kuzu_extension::{Extension, ExtensionContext};
use std::sync::Arc;
use kuzu_common::types::Value;

/// The PostgreSQL extension enables querying PostgreSQL databases from Kuzu.
pub struct PostgresExtension;

impl Default for PostgresExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl PostgresExtension {
    pub fn new() -> Self {
        Self
    }
}

impl Extension for PostgresExtension {
    fn name(&self) -> &'static str {
        "POSTGRES"
    }

    fn load(&self, context: &ExtensionContext) -> Result<(), String> {
        use kuzu_function::registry::ScalarFunction;

        #[cfg(feature = "native")]
        {
            // sql_query(conn_str: String, sql: String) → executes SQL against PostgreSQL
            let query_fn: Arc<dyn Fn(&[Value]) -> Result<Value, String> + Send + Sync> = Arc::new(|args| {
                if args.len() < 2 {
                    return Err("sql_query requires (connection_string, sql) arguments".into());
                }
                let conn_str = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err("sql_query: first argument must be a connection string".into()),
                };
                let sql = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => return Err("sql_query: second argument must be a SQL string".into()),
                };

                // Create a one-shot tokio runtime for this query
                let rt = tokio::runtime::Runtime::new().map_err(|e| format!("Failed to create tokio runtime: {e}"))?;

                let (client, connection) = rt
                    .block_on(async { tokio_postgres::connect(&conn_str, tokio_postgres::NoTls).await })
                    .map_err(|e| format!("PostgreSQL connect error: {e}"))?;

                // Spawn connection handler
                rt.spawn(async move {
                    if let Err(e) = connection.await {
                        tracing::warn!("PostgreSQL connection error: {e}");
                    }
                });

                let rows = rt
                    .block_on(async { client.query(&sql, &[]).await })
                    .map_err(|e| format!("PostgreSQL query error: {e}"))?;

                // Collect first row as string result
                if let Some(row) = rows.first() {
                    let mut parts = Vec::new();
                    for i in 0..row.len() {
                        let val: Option<&str> = row.try_get::<_, &str>(i).ok();
                        parts.push(val.unwrap_or("NULL").to_string());
                    }
                    Ok(Value::String(parts.join(",")))
                } else {
                    Ok(Value::String("(empty)".into()))
                }
            });

            context.register_scalar_function(
                "sql_query",
                ScalarFunction::CustomScalar {
                    name: "sql_query".into(),
                    execute: query_fn,
                },
            );

            tracing::info!("PostgreSQL extension loaded: 1 function registered (tokio-postgres native)");
        }

        #[cfg(not(feature = "native"))]
        {
            context.register_scalar_function(
                "sql_query",
                ScalarFunction::CustomScalar {
                    name: "sql_query".into(),
                    execute: Arc::new(|_| Err("PostgreSQL not available (feature 'native' disabled)".into())),
                },
            );
            tracing::info!("PostgreSQL extension loaded: 1 function registered (placeholder)");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_postgres_extension_name() {
        let ext = PostgresExtension::new();
        assert_eq!(ext.name(), "POSTGRES");
    }
}
