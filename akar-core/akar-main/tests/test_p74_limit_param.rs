//! P74 — Parser/binder support `LIMIT $limit` / `SKIP $skip` parameterized.
//!
//! Prior to P74 the `limit`/`offset` grammar rules only accepted a literal
//! integer, so kairos had to inline `LIMIT {int(limit)}`. These integration
//! tests exercise the parameterized form end-to-end: literal-and-parameter
//! LIMIT/SKIP combinations, plus the error paths (missing / negative /
//! non-integer parameter).

mod common;
use common::*;

fn setup_table(conn: &Connection) {
    exec(conn, "CREATE NODE TABLE T(id INT64, PRIMARY KEY (id))");
    exec(conn, "CREATE (n:T {id: 1})");
    exec(conn, "CREATE (n:T {id: 2})");
    exec(conn, "CREATE (n:T {id: 3})");
    exec(conn, "CREATE (n:T {id: 4})");
    exec(conn, "CREATE (n:T {id: 5})");
}

/// Execute a prepared query with params and return the projected `id` column
/// as a Vec<String> (nulls become "null"), for easy equality assertions.
fn run_ids(conn: &Connection, sql: &str, params: Vec<(&str, Value)>) -> Vec<String> {
    let stmt = conn.prepare(sql).unwrap();
    let result = conn.execute(&stmt, params).unwrap();
    let mut ids = Vec::new();
    for chunk in &result.chunks {
        for row in chunk.iter_rows() {
            for col in 0..chunk.fields.len() {
                if chunk.fields[col].is_null(row) {
                    ids.push("null".to_string());
                } else if let Some(v) = chunk.get_value(col, row) {
                    ids.push(format!("{v:?}"));
                }
            }
        }
    }
    ids
}

#[test]
fn limit_literal_still_works() {
    // Control: literal LIMIT path is unchanged.
    let (_db, conn) = setup_db();
    setup_table(&conn);
    assert_eq!(
        run_ids(&conn, "MATCH (n:T) RETURN n.id ORDER BY n.id LIMIT 2", vec![]),
        vec!["Int64(1)", "Int64(2)"]
    );
    assert_eq!(
        run_ids(&conn, "MATCH (n:T) RETURN n.id ORDER BY n.id LIMIT 2 SKIP 2", vec![]),
        vec!["Int64(3)", "Int64(4)"]
    );
}

#[test]
fn limit_param_basic() {
    // `LIMIT $limit` substitutes the bound integer value.
    let (_db, conn) = setup_db();
    setup_table(&conn);
    assert_eq!(
        run_ids(
            &conn,
            "MATCH (n:T) RETURN n.id ORDER BY n.id LIMIT $limit",
            vec![("limit", Value::UInt64(2))]
        ),
        vec!["Int64(1)", "Int64(2)"]
    );
}

#[test]
fn limit_param_with_skip_param() {
    // Both LIMIT and SKIP may be parameterized together.
    let (_db, conn) = setup_db();
    setup_table(&conn);
    assert_eq!(
        run_ids(
            &conn,
            "MATCH (n:T) RETURN n.id ORDER BY n.id LIMIT $limit SKIP $skip",
            vec![("limit", Value::UInt64(2)), ("skip", Value::UInt64(2))]
        ),
        vec!["Int64(3)", "Int64(4)"]
    );
}

#[test]
fn limit_param_int64_positive() {
    // An Int64 (signed) positive value is accepted too.
    let (_db, conn) = setup_db();
    setup_table(&conn);
    assert_eq!(
        run_ids(
            &conn,
            "MATCH (n:T) RETURN n.id ORDER BY n.id LIMIT $limit",
            vec![("limit", Value::Int64(3))]
        ),
        vec!["Int64(1)", "Int64(2)", "Int64(3)"]
    );
}

#[test]
fn limit_param_missing_value() {
    // Executing without the limit parameter must error (Missing parameter).
    let (_db, conn) = setup_db();
    setup_table(&conn);
    let stmt = conn
        .prepare("MATCH (n:T) RETURN n.id ORDER BY n.id LIMIT $limit")
        .unwrap();
    let err = conn.execute(&stmt, vec![]).unwrap_err().to_string();
    assert!(
        err.contains("Missing parameter"),
        "expected Missing parameter, got: {err}"
    );
}

#[test]
fn limit_param_negative_rejected() {
    // A negative integer cannot be a row limit.
    let (_db, conn) = setup_db();
    setup_table(&conn);
    let stmt = conn
        .prepare("MATCH (n:T) RETURN n.id ORDER BY n.id LIMIT $limit")
        .unwrap();
    let err = conn
        .execute(&stmt, vec![("limit", Value::Int64(-1))])
        .unwrap_err()
        .to_string();
    assert!(err.contains("non-negative"), "expected non-negative error, got: {err}");
}

#[test]
fn limit_param_non_integer_rejected() {
    // A string cannot be a row limit.
    let (_db, conn) = setup_db();
    setup_table(&conn);
    let stmt = conn
        .prepare("MATCH (n:T) RETURN n.id ORDER BY n.id LIMIT $limit")
        .unwrap();
    let err = conn
        .execute(&stmt, vec![("limit", Value::String("x".into()))])
        .unwrap_err()
        .to_string();
    assert!(err.contains("non-negative"), "expected non-negative error, got: {err}");
}
