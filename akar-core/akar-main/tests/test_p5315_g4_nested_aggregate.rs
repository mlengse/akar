//! P53.15 — nested aggregate functions inside expressions (G4).
//!
//! `COALESCE(MAX(x), 0)`, `COALESCE(MAX(x), 0) + 1` and `COALESCE(SUM(x), 0)`
//! must bind and execute. Aggregate functions are recognized not only at the
//! top level of a RETURN/WITH projection but anywhere inside the expression
//! tree (function-call arguments, arithmetic operands, CASE branches).
//!
//! The optimizer's `AggregateDetection` pass previously only matched a
//! projection expression that WAS a single aggregate call. For a nested call
//! (`COALESCE(MAX(id), 0)`) it saw no aggregate and left the projection as-is;
//! the evaluator then looked up `MAX` as a scalar function →
//! `Unknown function: 'MAX'`.
//!
//! Fix: aggregate calls are detected recursively, extracted into an Aggregate
//! operator, and replaced in the projection by a field-name reference to the
//! aggregate output column; a Projection is kept above the Aggregate to
//! evaluate the outer expression over the collapsed result.

mod common;

use common::*;

/// Insert Memory rows with distinct ids and a numeric `val`.
fn setup(conn: &Connection) {
    exec(conn, "CREATE NODE TABLE Memory (id INT64, val INT64, grp STRING, PRIMARY KEY (id))");
    exec(conn, "CREATE (m:Memory {id: 1, val: 10, grp: 'a'})");
    exec(conn, "CREATE (m:Memory {id: 2, val: 20, grp: 'a'})");
    exec(conn, "CREATE (m:Memory {id: 5, val: 30, grp: 'b'})");
}

/// Read all rows of a single-column result as Int64.
fn int_column(conn: &Connection, sql: &str) -> Vec<i64> {
    query_column(conn, sql)
        .into_iter()
        .map(|v| match v {
            Value::Int64(n) => n,
            other => panic!("expected Int64, got {other:?}"),
        })
        .collect()
}

#[test]
fn nested_coalesce_max_returns_max() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // G4 probe: MAX nested inside COALESCE must resolve to the table max.
    let got = int_column(&conn, "MATCH (m:Memory) RETURN COALESCE(MAX(m.id), 0)");
    assert_eq!(got, vec![5], "COALESCE(MAX(id), 0) = max id");
}

#[test]
fn nested_coalesce_max_empty_table_returns_fallback() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Memory (id INT64, PRIMARY KEY (id))");

    // Empty input → MAX = NULL → COALESCE falls back to 0 (kairos `_next_id`).
    let got = int_column(&conn, "MATCH (m:Memory) RETURN COALESCE(MAX(m.id), 0)");
    assert_eq!(got, vec![0], "COALESCE(MAX(id), 0) = 0 on empty table");
}

#[test]
fn nested_coalesce_max_plus_one() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // Exact kairos pattern: `COALESCE(MAX(id), 0) + 1 AS nid`.
    let got = int_column(&conn, "MATCH (m:Memory) RETURN COALESCE(MAX(m.id), 0) + 1");
    assert_eq!(got, vec![6], "COALESCE(MAX(id), 0) + 1 = next id");
}

#[test]
fn nested_coalesce_sum() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // Exact kairos stats pattern: `COALESCE(SUM(x), 0)`.
    let got = int_column(&conn, "MATCH (m:Memory) RETURN COALESCE(SUM(m.val), 0)");
    assert_eq!(got, vec![60], "COALESCE(SUM(val), 0) = total");
}

#[test]
fn nested_coalesce_sum_empty_table_returns_fallback() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Memory (id INT64, val INT64, PRIMARY KEY (id))");

    let got = int_column(&conn, "MATCH (m:Memory) RETURN COALESCE(SUM(m.val), 0)");
    assert_eq!(got, vec![0], "COALESCE(SUM(val), 0) = 0 on empty table");
}

#[test]
fn group_by_with_nested_aggregate() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // Non-aggregate column becomes a GROUP BY key; the nested aggregate is
    // computed per group, then COALESCE is applied per row.
    let result = query(&conn, "MATCH (m:Memory) RETURN m.grp, COALESCE(MAX(m.val), 0) ORDER BY m.grp");
    let mut rows = Vec::new();
    for chunk in &result.chunks {
        for row in chunk.iter_rows() {
            let grp = match chunk.get_value(0, row).unwrap() {
                Value::String(s) => s,
                other => panic!("expected grp String, got {other:?}"),
            };
            let mx = match chunk.get_value(1, row).unwrap() {
                Value::Int64(n) => n,
                other => panic!("expected COALESCE(MAX(val), 0) Int64, got {other:?}"),
            };
            rows.push((grp, mx));
        }
    }
    assert_eq!(rows, vec![("a".to_string(), 20), ("b".to_string(), 30)], "per-group nested aggregate");
}

#[test]
fn top_level_aggregates_still_work() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // Regression: pure top-level aggregates must keep the previous behavior.
    let result = query(&conn, "MATCH (m:Memory) RETURN COUNT(*), SUM(m.val), MAX(m.id)");
    let chunk = result.chunks.first().expect("one result chunk");
    let row = chunk.iter_rows().next().expect("one result row");
    assert_eq!(chunk.get_value(0, row).unwrap(), Value::Int64(3));
    assert_eq!(chunk.get_value(1, row).unwrap(), Value::Int64(60));
    assert_eq!(chunk.get_value(2, row).unwrap(), Value::Int64(5));
}

#[test]
fn mixed_top_level_and_nested_aggregates() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // One top-level aggregate + one nested aggregate in the same projection.
    let result = query(&conn, "MATCH (m:Memory) RETURN COUNT(*), COALESCE(MAX(m.id), 0)");
    let chunk = result.chunks.first().expect("one result chunk");
    let row = chunk.iter_rows().next().expect("one result row");
    assert_eq!(chunk.get_value(0, row).unwrap(), Value::Int64(3));
    assert_eq!(chunk.get_value(1, row).unwrap(), Value::Int64(5));
}
