//! Re-export shared test helpers from the crate's `test_helpers` module.
//!
//! Integration tests use `common::setup_db()` etc. via `mod common;`.

pub use akar_main::test_helpers::*;
#[allow(unused_imports)]
pub use akar_main::{Connection, Database, SystemConfig};

#[allow(dead_code)]
pub fn query_rows(conn: &Connection, sql: &str) -> Vec<Vec<String>> {
    let result = conn.query(sql).unwrap();
    let mut rows = Vec::new();
    for chunk in &result.chunks {
        for row in chunk.iter_rows() {
            let mut vals = Vec::new();
            for col_idx in 0..chunk.fields.len() {
                if chunk.fields[col_idx].is_null(row) {
                    vals.push("null".to_string());
                } else if let Some(v) = chunk.get_value(col_idx, row) {
                    vals.push(format!("{v:?}"));
                }
            }
            rows.push(vals);
        }
    }
    rows
}
