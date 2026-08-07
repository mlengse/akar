//! Shared test helpers — single source of truth for all Akar tests.
//!
//! Provides `setup_db` (in-memory) and common assertion helpers.
//! Used by both `src/connection_test.rs` (via `crate::test_helpers::*`)
//! and integration tests in `tests/` (via `common::*`).

use crate::connection::Connection;
use crate::database::{Database, SystemConfig};
use crate::query_result::QueryResult;
pub use akar_common::types::Value;
use std::sync::Arc;

/// Create a database for testing backed by a temporary directory.
///
/// Uses a temp dir instead of `:memory:` because the WAL implementation
/// requires a real filesystem path.
pub fn setup_db() -> (Arc<Database>, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_db");
    let config = SystemConfig::default();
    let database = Arc::new(Database::new(db_path, config).unwrap());
    let conn = Connection::new(&database);
    // Leak the TempDir so it lives for the process lifetime.
    // This is fine for tests — the OS reclaims on exit.
    std::mem::forget(dir);
    (database, conn)
}

/// Create a filesystem-backed database in a temporary directory.
///
/// The returned `TempDir` must be held alive to keep the database on disk.
pub fn setup_db_on_disk() -> (tempfile::TempDir, Arc<Database>, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_db");
    let config = SystemConfig::default();
    let database = Arc::new(Database::new(db_path, config).unwrap());
    let conn = Connection::new(&database);
    (dir, database, conn)
}

/// Create a filesystem-backed database with a specific checkpoint threshold.
pub fn setup_db_with_checkpoint(threshold: i64) -> (Arc<Database>, Connection, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_db");
    let config = SystemConfig {
        checkpoint_threshold: threshold,
        ..SystemConfig::default()
    };
    let database = Arc::new(Database::new(db_path, config).unwrap());
    let conn = Connection::new(&database);
    (database, conn, dir)
}

/// Execute a query and return its summary string (useful for DDL/mutations).
pub fn exec(conn: &Connection, query: &str) -> String {
    let result = conn.query(query).unwrap();
    assert!(
        result.is_success(),
        "Query failed: {query} -> {:?}",
        result.error_message
    );
    result.result_summary()
}

/// Execute a query and return `Result<String, String>`.
pub fn exec_ok(conn: &Connection, sql: &str) -> Result<String, String> {
    conn.query(sql).map(|r| r.to_string())
}

/// Execute a query and assert it returns an error. Returns the error message.
pub fn exec_err(conn: &Connection, query: &str) -> String {
    let result = conn.query(query);
    match result {
        Err(e) => e,
        Ok(r) => {
            if r.is_success() {
                panic!("Expected error for query: {query}, got success: {}", r.result_summary());
            }
            r.error_message.unwrap_or_else(|| "Unknown error".into())
        }
    }
}

/// Execute a query and return the raw `QueryResult` (unwrapped).
pub fn query(conn: &Connection, sql: &str) -> QueryResult {
    conn.query(sql).unwrap()
}

/// Execute a query and return the raw `QueryResult`.
pub fn query_result(conn: &Connection, sql: &str) -> Result<QueryResult, String> {
    conn.query(sql)
}

/// Extract all values from the first column of a query result.
pub fn query_column(conn: &Connection, sql: &str) -> Vec<Value> {
    let result = conn.query(sql).unwrap();
    result
        .chunks
        .iter()
        .flat_map(|c| (0..c.size).filter_map(|i| c.get_value(0, i)))
        .collect()
}

/// Execute a query and return actual values as a single formatted string.
pub fn query_values(conn: &Connection, query: &str) -> String {
    let result = conn.query(query).unwrap();
    assert!(
        result.is_success(),
        "Query failed: {query} -> {:?}",
        result.error_message
    );
    let mut out = String::new();
    for chunk in &result.chunks {
        for row in chunk.iter_rows() {
            for (col_idx, field) in chunk.fields.iter().enumerate() {
                if field.is_null(row) {
                    out.push_str("null ");
                } else if let Some(v) = chunk.get_value(col_idx, row) {
                    out.push_str(&format!("{v:?} "));
                }
            }
            out.push('\n');
        }
    }
    out
}
