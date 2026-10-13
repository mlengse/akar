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
                "sslmode=require requested but TLS support is not compiled into akar-postgres                  (use sslmode=disable or sslmode=prefer)"
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

/// Validate a PostgreSQL query string.
///
/// Ensures the query is non-empty, starts with a read-only query keyword
/// (`SELECT`, `EXPLAIN`, `SHOW`, `WITH`, `TABLE`, `VALUES`), and does not
/// contain stacked/multiple SQL statements separated by semicolons.
pub fn validate_query(sql: &str) -> Result<(), String> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err("Empty query".into());
    }

    let uppercase_trimmed = trimmed.to_uppercase();
    let valid_keywords = ["SELECT", "EXPLAIN", "SHOW", "WITH", "TABLE", "VALUES"];
    if !valid_keywords.iter().any(|kw| uppercase_trimmed.starts_with(kw)) {
        let first_word = trimmed.split_whitespace().next().unwrap_or("");
        return Err(format!(
            "Query must start with SELECT, EXPLAIN, SHOW, WITH, TABLE, or VALUES. Got: {first_word}"
        ));
    }

    if has_stacked_statements(trimmed) {
        return Err("Multiple SQL statements are not allowed".into());
    }

    Ok(())
}

fn has_stacked_statements(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut statement_count = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut dollar_tag: Option<String> = None;
    let mut has_unquoted_content = false;

    while i < len {
        if in_line_comment {
            if bytes[i] == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }

        if in_block_comment {
            if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        if let Some(ref tag) = dollar_tag {
            if sql[i..].starts_with(tag) {
                i += tag.len();
                dollar_tag = None;
            } else {
                i += 1;
            }
            continue;
        }

        if in_single_quote {
            if bytes[i] == b'\'' {
                if i + 1 < len && bytes[i + 1] == b'\'' {
                    i += 2;
                } else {
                    in_single_quote = false;
                    i += 1;
                }
            } else {
                i += 1;
            }
            continue;
        }

        if in_double_quote {
            if bytes[i] == b'"' {
                if i + 1 < len && bytes[i + 1] == b'"' {
                    i += 2;
                } else {
                    in_double_quote = false;
                    i += 1;
                }
            } else {
                i += 1;
            }
            continue;
        }

        if i + 1 < len && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            in_line_comment = true;
            i += 2;
            continue;
        }

        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            in_block_comment = true;
            i += 2;
            continue;
        }

        if bytes[i] == b'\'' {
            in_single_quote = true;
            has_unquoted_content = true;
            i += 1;
            continue;
        }

        if bytes[i] == b'"' {
            in_double_quote = true;
            has_unquoted_content = true;
            i += 1;
            continue;
        }

        if bytes[i] == b'$' {
            if let Some(end_dollar) = sql[i + 1..].find('$') {
                let tag_candidate = &sql[i..=i + 1 + end_dollar];
                let inner = &tag_candidate[1..tag_candidate.len() - 1];
                if inner.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    dollar_tag = Some(tag_candidate.to_string());
                    has_unquoted_content = true;
                    i += tag_candidate.len();
                    continue;
                }
            }
        }

        if bytes[i] == b';' {
            if has_unquoted_content {
                statement_count += 1;
                has_unquoted_content = false;
            }
            i += 1;
            continue;
        }

        if !bytes[i].is_ascii_whitespace() {
            has_unquoted_content = true;
        }

        i += 1;
    }

    if has_unquoted_content {
        statement_count += 1;
    }

    statement_count > 1
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

                validate_query(&sql)?;

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

    #[test]
    fn test_validate_query_valid() {
        assert!(validate_query("SELECT * FROM users").is_ok());
        assert!(validate_query("  select id, name from table  ").is_ok());
        assert!(validate_query("EXPLAIN SELECT 1").is_ok());
        assert!(validate_query("SHOW tables").is_ok());
        assert!(validate_query("WITH cte AS (SELECT 1) SELECT * FROM cte").is_ok());
        assert!(validate_query("TABLE users").is_ok());
        assert!(validate_query("VALUES (1, 'a'), (2, 'b')").is_ok());
        assert!(validate_query("SELECT 'hello; world' FROM t").is_ok());
        assert!(validate_query("SELECT $tag$hello; world$tag$ FROM t").is_ok());
        assert!(validate_query("SELECT * FROM t;").is_ok());
    }

    #[test]
    fn test_validate_query_invalid() {
        assert!(validate_query("").is_err());
        assert!(validate_query("   ").is_err());
        assert!(validate_query("INSERT INTO t VALUES (1)").is_err());
        assert!(validate_query("DELETE FROM t").is_err());
        assert!(validate_query("DROP TABLE t").is_err());
        assert!(validate_query("UPDATE t SET x = 1").is_err());
        assert!(validate_query("CREATE TABLE t (id INT)").is_err());
        assert!(validate_query("SELECT * FROM t; DROP TABLE t").is_err());
        assert!(validate_query("SELECT 1; SELECT 2").is_err());
    }
}
