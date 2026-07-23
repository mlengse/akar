use kuzu_main::{Connection, Database, SystemConfig};
use std::sync::Arc;

/// Create a temporary database for testing.
pub fn setup_db() -> (Arc<Database>, Connection) {
    let db = Arc::new(Database::new(":memory:", SystemConfig::default()).unwrap());
    let conn = Connection::new(&db);
    (db, conn)
}

/// Execute a query and return its summary string (useful for DDL/mutations).
pub fn exec(conn: &Connection, query: &str) -> String {
    let result = conn.query(query).unwrap();
    assert!(
        result.is_success(),
        "Query failed: {query} → {:?}",
        result.error_message
    );
    result.result_summary()
}

/// Execute a query and assert it returns an error. Returns the error message.
#[allow(dead_code)]
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

/// Execute a query and return actual values as a single formatted string.
/// Useful for checking actual data.
#[allow(dead_code)]
pub fn query_values(conn: &Connection, query: &str) -> String {
    let result = conn.query(query).unwrap();
    assert!(
        result.is_success(),
        "Query failed: {query} → {:?}",
        result.error_message
    );
    let mut out = String::new();
    for chunk in &result.chunks {
        for row in chunk.iter_rows() {
            for (col_idx, field) in chunk.fields.iter().enumerate() {
                if field.is_null(row) {
                    out.push_str("null ");
                } else if let Some(v) = chunk.get_value(col_idx, row) {
                    out.push_str(&format!("{:?} ", v));
                }
            }
            out.push('\n');
        }
    }
    out
}

/// Execute a query and return actual values as a vector of vectors of strings.
/// Useful for exact assertions on rows.
#[allow(dead_code)]
pub fn query_rows(conn: &Connection, query: &str) -> Vec<Vec<String>> {
    let result = conn.query(query).unwrap();
    assert!(
        result.is_success(),
        "Query failed: {query} → {:?}",
        result.error_message
    );
    let mut rows = Vec::new();
    for chunk in &result.chunks {
        for row in chunk.iter_rows() {
            let mut current_row = Vec::new();
            for (col_idx, field) in chunk.fields.iter().enumerate() {
                if field.is_null(row) {
                    current_row.push("null".to_string());
                } else if let Some(v) = chunk.get_value(col_idx, row) {
                    current_row.push(format!("{:?}", v));
                } else {
                    current_row.push("null".to_string());
                }
            }
            rows.push(current_row);
        }
    }
    rows
}
