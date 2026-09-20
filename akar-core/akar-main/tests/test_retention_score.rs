//! P119 — `retention_score` reachable from Cypher.
//!
//! The formula's own properties are pinned in
//! `akar-function/src/scalar/retention.rs`. These tests pin the pipeline: that
//! the function is registered under its public name, resolves through the
//! planner/evaluator, and returns a usable double per row.

mod common;
use common::*;

fn scalar(conn: &Connection, sql: &str) -> f64 {
    let rows = query_rows(conn, sql);
    assert_eq!(rows.len(), 1, "expected one row from {sql}: {rows:?}");
    assert_eq!(rows[0].len(), 1, "expected one column from {sql}: {rows:?}");
    let raw = &rows[0][0];
    let inner = raw
        .strip_prefix("Double(")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or_else(|| panic!("expected a Double, got {raw}"));
    inner
        .parse::<f64>()
        .unwrap_or_else(|e| panic!("unparsable double {raw}: {e}"))
}

#[test]
fn retention_score_is_callable_from_cypher() {
    let (_db, conn) = setup_db();

    // Just recalled → fully retained.
    let fresh = scalar(&conn, "RETURN retention_score(0, 0, 0, 1.0, 0) AS r");
    assert!(
        (fresh - 1.0).abs() < 1e-12,
        "fresh memory must be fully retained: {fresh}"
    );

    // 30 days idle with no history at max salience: plain one-day Ebbinghaus.
    let idle = scalar(&conn, "RETURN retention_score(0, 0, 30, 1.0, 0) AS r");
    assert!(
        (idle - (-30.0f64).exp()).abs() < 1e-9,
        "one-day Ebbinghaus after 30 days: {idle}"
    );
}

#[test]
fn retention_score_reads_memory_columns_per_row() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Mem(id INT64, days INT64, salience DOUBLE, PRIMARY KEY (id))",
    );
    // Same idleness, different salience: the salient memory must survive better.
    exec(&conn, "CREATE (:Mem {id: 1, days: 14, salience: 0.1})");
    exec(&conn, "CREATE (:Mem {id: 2, days: 14, salience: 1.0})");

    let rows = query_rows(
        &conn,
        "MATCH (m:Mem) RETURN m.id AS id, retention_score(m.days, 0, m.days, m.salience, 0) AS r ORDER BY m.id",
    );
    assert_eq!(rows.len(), 2, "both memories must be scored: {rows:?}");
    let low = parse_double(&rows[0][1]);
    let high = parse_double(&rows[1][1]);
    assert!(low < high, "the salient memory must retain more: {low} vs {high}");
    assert!((0.0..=1.0).contains(&low) && (0.0..=1.0).contains(&high));
}

#[test]
fn retention_score_can_order_a_decay_report() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Decay(id INT64, idle INT64, PRIMARY KEY (id))");
    for idle in [0, 5, 20] {
        exec(&conn, &format!("CREATE (:Decay {{id: {idle}, idle: {idle}}})"));
    }

    // The whole point of the primitive: `ORDER BY retention_score(...)` ranks
    // the most-retained memories first without a separate computed column.
    let rows = query_rows(
        &conn,
        "MATCH (d:Decay) RETURN d.id AS id ORDER BY retention_score(0, 0, d.idle, 1.0, 0) DESC",
    );
    assert_eq!(
        rows,
        vec![
            vec!["Int64(0)".to_string()],
            vec!["Int64(5)".to_string()],
            vec!["Int64(20)".to_string()],
        ],
        "least idle must rank first: {rows:?}"
    );
}

#[test]
fn bad_arguments_are_reported_not_silently_null() {
    let (_db, conn) = setup_db();
    let err = exec_err(&conn, "RETURN retention_score(1, 2, 3) AS r");
    assert!(err.contains("5 or 6 arguments"), "arity must be validated, got: {err}");
}

fn parse_double(raw: &str) -> f64 {
    raw.strip_prefix("Double(")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or_else(|| panic!("expected a Double, got {raw}"))
        .parse()
        .unwrap_or_else(|e| panic!("unparsable double {raw}: {e}"))
}
