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
    let type_name = row
        .columns()
        .get(i)
        .map(|c| c.type_().name())
        .unwrap_or("unknown")
        .to_string();
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

/// Shared PostgreSQL runtime, connection cache and query helper.
///
/// Reuses a single process-wide tokio runtime and caches one connection per
/// connection string, instead of building a fresh multi-thread runtime and a
/// fresh TCP connection on every `sql_query` call (P52.37). A `connect_timeout`
/// is enforced so an unreachable host cannot block the caller forever.
#[cfg(feature = "native")]
mod runtime {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::Duration;

    static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    static CONNECTIONS: OnceLock<Mutex<HashMap<String, Arc<tokio_postgres::Client>>>> = OnceLock::new();

    fn runtime() -> Result<&'static tokio::runtime::Runtime, String> {
        RUNTIME
            .get_or_init(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| format!("Failed to create tokio runtime: {e}"))
            })
            .as_ref()
            .map_err(|e| e.clone())
    }

    fn connections() -> &'static Mutex<HashMap<String, Arc<tokio_postgres::Client>>> {
        CONNECTIONS.get_or_init(Default::default)
    }

    pub fn query(conn_str: &str, sql: &str) -> Result<Vec<tokio_postgres::Row>, String> {
        let rt = runtime()?;

        let mut config = conn_str
            .parse::<tokio_postgres::Config>()
            .map_err(|e| format!("Invalid PostgreSQL connection string: {e}"))?;

        // Fail secure rather than silently sending credentials over plaintext
        // when the caller explicitly requires TLS (which we cannot provide yet).
        if config.get_ssl_mode() == tokio_postgres::config::SslMode::Require {
            return Err(
                "sslmode=require requested but TLS support is not compiled into akar-postgres \
                 (use sslmode=disable or sslmode=prefer)"
                    .into(),
            );
        }
        config.connect_timeout(Duration::from_secs(10));

        let mut cache = connections()
            .lock()
            .map_err(|_| "Connection cache lock poisoned".to_string())?;

        let client = match cache.get(conn_str) {
            Some(c) => Arc::clone(c),
            None => {
                let (client, connection) = rt
                    .block_on(async { config.connect(tokio_postgres::NoTls).await })
                    .map_err(|e| format!("PostgreSQL connect error: {e}"))?;
                rt.spawn(async move {
                    if let Err(e) = connection.await {
                        tracing::warn!("PostgreSQL connection error: {e}");
                    }
                });
                let client = Arc::new(client);
                cache.insert(conn_str.to_string(), Arc::clone(&client));
                client
            }
        };

        match rt.block_on(async { client.query(sql, &[]).await }) {
            Ok(rows) => Ok(rows),
            Err(e) => {
                // Drop a dead connection so the next call reconnects.
                if e.is_closed() {
                    cache.remove(conn_str);
                }
                Err(format!("PostgreSQL query error: {e}"))
            }
        }
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

                let rows = runtime::query(&conn_str, &sql)?;

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
