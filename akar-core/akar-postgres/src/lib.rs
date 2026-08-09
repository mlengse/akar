//! PostgreSQL extension for Akar.
//!
//! Provides integration with PostgreSQL databases using the native
//! `tokio-postgres` crate with a synchronous wrapper via `block_on`.
//!
//! ## Native Rust approach
//! Uses `tokio-postgres` (v0.7) with `futures::executor::block_on()` to bridge
//! the async PostgreSQL client with Akar's synchronous runtime.
//! Most complex extension due to catalog binding + table enumeration + type mapping.

use akar_extension::{Extension, ExtensionContext};
use std::sync::Arc;

/// Render a single cell of a PostgreSQL row as text.
///
/// `try_get` needs a concrete type at compile time, so we cascade through the
/// common representations. A NULL cell yields `Ok(None)` for every `Option<T>`
/// probe and is reported as "NULL"; cells whose type matches none of the probes
/// (e.g. bytea, timestamps) fall back to their column type name.
#[cfg(feature = "native")]
fn postgres_value_to_string(row: &tokio_postgres::Row, i: usize) -> String {
    let type_name = row.columns().get(i).map(|c| c.type_().name()).unwrap_or("unknown").to_string();
    if let Ok(Some(v)) = row.try_get::<_, Option<String>>(i) {
        return v;
    }
    if let Ok(Some(v)) = row.try_get::<_, Option<i64>>(i) {
        return v.to_string();
    }
    if let Ok(Some(v)) = row.try_get::<_, Option<i32>>(i) {
        return v.to_string();
    }
    if let Ok(Some(v)) = row.try_get::<_, Option<f64>>(i) {
        return v.to_string();
    }
    if let Ok(Some(v)) = row.try_get::<_, Option<bool>>(i) {
        return v.to_string();
    }
    // A NULL cell succeeds with None for any Option<T> probe.
    if row.try_get::<_, Option<i64>>(i).is_ok() {
        "NULL".to_string()
    } else {
        format!("<{type_name}>")
    }
}

/// The PostgreSQL extension enables querying PostgreSQL databases from Akar.
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
        use akar_function::registry::ScalarFunction;

        #[cfg(feature = "native")]
        {
            use akar_function::Value;

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

                // Collect every row (all columns) as strings, not just the first.
                let mut parts = Vec::new();
                for row in &rows {
                    for i in 0..row.len() {
                        parts.push(postgres_value_to_string(row, i));
                    }
                }
                if parts.is_empty() {
                    Ok(Value::String("(empty)".into()))
                } else {
                    Ok(Value::String(parts.join(",")))
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
