//! P53.23 — `ORDER BY` / `LIMIT` on a computed (non-column) expression must sort
//! by the evaluated expression, not by the i-th output column.
//!
//! Evidence (audit-latent-engine-orderby-computed-and-index.md): `resolve_sort_keys`
//! fell back to `col.unwrap_or(i)` whenever the sort key was not a plain
//! `PropertyAccess`/`Variable` (e.g. `n.a + n.b`), so `RETURN n.id, n.a, n.b
//! ORDER BY n.a + n.b` silently sorted by column index 1 (`n.id`). Fix (P53.23):
//! computed keys are evaluated per row via `ExpressionEvaluator` and appended as
//! synthetic sort columns; the trailing columns are stripped from the output.

mod common;

use common::*;

fn setup(conn: &Connection) {
    exec(
        conn,
        "CREATE NODE TABLE T (id INT64, a INT64, b INT64, PRIMARY KEY (id))",
    );
    // Anti-correlated: id order must NOT match a+b order, so a positional
    // (id-based) sort would produce a different result than the true sort.
    exec(conn, "CREATE (n:T {id: 1, a: 10, b: 1})"); // a+b = 11
    exec(conn, "CREATE (n:T {id: 2, a: 1, b: 100})"); // a+b = 101
    exec(conn, "CREATE (n:T {id: 3, a: 5, b: 2})"); // a+b = 7
    exec(conn, "CREATE (n:T {id: 4, a: 2, b: 8})"); // a+b = 10
}

fn read_rows(conn: &Connection, sql: &str) -> Vec<Vec<Value>> {
    let result = query(conn, sql);
    let mut rows = Vec::new();
    for chunk in &result.chunks {
        for row in chunk.iter_rows() {
            let mut vals = Vec::new();
            for col in 0..chunk.fields.len() {
                vals.push(chunk.get_value(col, row).unwrap_or(Value::Null));
            }
            rows.push(vals);
        }
    }
    rows
}

/// Sort by computed `n.a + n.b` DESC must follow the expression, not column 0.
#[test]
fn order_by_computed_expression_desc() {
    let (_db, conn) = setup_db();
    setup(&conn);

    let rows = read_rows(&conn, "MATCH (n:T) RETURN n.id, n.a, n.b ORDER BY n.a + n.b DESC");
    assert_eq!(
        rows,
        vec![
            vec![Value::Int64(2), Value::Int64(1), Value::Int64(100)], // 101
            vec![Value::Int64(1), Value::Int64(10), Value::Int64(1)],  // 11
            vec![Value::Int64(4), Value::Int64(2), Value::Int64(8)],   // 10
            vec![Value::Int64(3), Value::Int64(5), Value::Int64(2)],   // 7
        ],
        "ORDER BY n.a + n.b DESC sorted by the computed value"
    );
}

/// An ORDER BY key referencing a column that is not projected (`n.a` here) is
/// invalid Cypher. It must error — not silently sort by output column 0
/// (the pre-P53.23 behaviour / the evaluator's column-0 fallback).
#[test]
fn order_by_non_projected_column_errors() {
    let (_db, conn) = setup_db();
    setup(&conn);

    let err = exec_err(&conn, "MATCH (n:T) RETURN n.id ORDER BY n.a + n.b ASC");
    assert!(
        err.contains("not found") || err.contains("not in scope") || err.contains("field_names"),
        "expected a clear error, got: {err}"
    );
}

/// TopK path (`LIMIT`) must also sort by the computed expression.
#[test]
fn topk_computed_expression() {
    let (_db, conn) = setup_db();
    setup(&conn);

    let rows = read_rows(
        &conn,
        "MATCH (n:T) RETURN n.id, n.a, n.b ORDER BY n.a * n.b DESC LIMIT 2",
    );
    // n.a * n.b: 10, 100, 10, 16 → top2 = id2 (100), then id4 (16) beats id1 (10)
    assert_eq!(
        rows,
        vec![
            vec![Value::Int64(2), Value::Int64(1), Value::Int64(100)],
            vec![Value::Int64(4), Value::Int64(2), Value::Int64(8)],
        ],
        "ORDER BY n.a * n.b DESC LIMIT 2"
    );
}

/// Mixed keys with all referenced columns projected: computed key first, then a
/// plain column as tie-breaker.
#[test]
fn order_by_computed_then_column() {
    let (_db, conn) = setup_db();
    setup(&conn);

    let rows = read_rows(
        &conn,
        "MATCH (n:T) RETURN n.id, n.a, n.b ORDER BY n.a + n.b ASC, n.b DESC",
    );
    assert_eq!(
        rows,
        vec![
            vec![Value::Int64(3), Value::Int64(5), Value::Int64(2)],   // 7
            vec![Value::Int64(4), Value::Int64(2), Value::Int64(8)],   // 10
            vec![Value::Int64(1), Value::Int64(10), Value::Int64(1)],  // 11
            vec![Value::Int64(2), Value::Int64(1), Value::Int64(100)], // 101
        ]
    );
}
