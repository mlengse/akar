//! P53.40 regression tests — kairos Finding #22: WHERE on a WITH-aggregate
//! alias must filter the aggregated pipeline (`HAVING` semantics), not fail.
//!
//! Repro shape from the kairos dream REM phase:
//!   MATCH (m:T) OPTIONAL MATCH (m)-[r:R]-(:T)
//!   WITH m, COUNT(r) AS cnt WHERE cnt < $n RETURN ...
//!
//! The binder now binds this (P53.18 resets scope to the WITH projection), but
//! execution failed with `Variable 'cnt' not found in field_names`.

mod common;
use common::*;

fn setup_graph() -> Connection {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, x INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE REL TABLE R(FROM T TO T)");
    exec(&conn, "CREATE (:T {id: 1, x: 10})");
    exec(&conn, "CREATE (:T {id: 2, x: 20})");
    exec(&conn, "MATCH (a:T {id: 1}), (b:T {id: 2}) CREATE (a)-[:R]->(b)");
    conn
}

#[test]
fn test_p5340_control_with_aggregate_return() {
    // Control: WITH aggregate straight to RETURN already works (P53.18 E2E).
    let conn = setup_graph();
    let rows = query_rows(
        &conn,
        "MATCH (a:T) OPTIONAL MATCH (a)-[r:R]-(:T) \
         WITH a, COUNT(r) AS cnt \
         RETURN a.id AS id, cnt AS connection_count ORDER BY id",
    );
    assert_eq!(
        rows,
        vec![
            vec!["Int64(1)".to_string(), "Int64(1)".to_string()],
            vec!["Int64(2)".to_string(), "Int64(1)".to_string()],
        ],
        "control WITH-aggregate RETURN must work, got: {rows:?}"
    );
}

#[test]
fn test_p5340_with_aggregate_where_alias_passes_all() {
    // The #22 case: WHERE on the aggregate alias, threshold high enough that
    // every row survives. Must NOT error; result equals the control.
    let conn = setup_graph();
    let rows = query_rows(
        &conn,
        "MATCH (a:T) OPTIONAL MATCH (a)-[r:R]-(:T) \
         WITH a, COUNT(r) AS cnt \
         WHERE cnt < 5 \
         RETURN a.id AS id, cnt ORDER BY id",
    );
    assert_eq!(
        rows,
        vec![
            vec!["Int64(1)".to_string(), "Int64(1)".to_string()],
            vec!["Int64(2)".to_string(), "Int64(1)".to_string()],
        ],
        "WHERE cnt < 5 after WITH aggregate must filter aggregated rows, got: {rows:?}"
    );
}

#[test]
fn test_p5340_with_aggregate_where_alias_filters() {
    // Same but threshold low enough that every row is filtered out — must
    // return zero rows rather than erroring or leaking unfiltered rows.
    let conn = setup_graph();
    let rows = query_rows(
        &conn,
        "MATCH (a:T) OPTIONAL MATCH (a)-[r:R]-(:T) \
         WITH a, COUNT(r) AS cnt \
         WHERE cnt > 100 \
         RETURN a.id AS id, cnt ORDER BY id",
    );
    assert_eq!(
        rows,
        Vec::<Vec<String>>::new(),
        "WHERE cnt > 100 must filter everything out, got: {rows:?}"
    );
}

#[test]
fn test_p5340_with_aggregate_where_no_optional_match() {
    // Minimal variant without OPTIONAL MATCH (group-by over plain scan).
    let conn = setup_graph();
    let rows = query_rows(
        &conn,
        "MATCH (a:T) WITH a.x AS x, COUNT(a) AS c WHERE c >= 1 RETURN x, c ORDER BY x",
    );
    assert_eq!(
        rows,
        vec![
            vec!["Int64(10)".to_string(), "Int64(1)".to_string()],
            vec!["Int64(20)".to_string(), "Int64(1)".to_string()],
        ],
        "minimal WITH-agg WHERE must group and filter, got: {rows:?}"
    );
}

#[test]
fn test_p5340_bare_var_group_key_direct_agg() {
    // Ground truth: direct aggregation (no WITH) with undirected edge pattern —
    // each endpoint participates in exactly 1 edge.
    let conn = setup_graph();
    let rows = query_rows(
        &conn,
        "MATCH (a:T)-[r:R]-(:T) RETURN a.id AS id, COUNT(r) AS c ORDER BY id",
    );
    assert_eq!(
        rows,
        vec![
            vec!["Int64(1)".to_string(), "Int64(1)".to_string()],
            vec!["Int64(2)".to_string(), "Int64(1)".to_string()],
        ],
        "direct undirected agg ground truth, got: {rows:?}"
    );
}

#[test]
fn test_p5340_bare_var_group_key_no_optional() {
    // Production shape WITHOUT optional match: bare node variable as group key.
    let conn = setup_graph();
    let rows = query_rows(
        &conn,
        "MATCH (a:T) WITH a, count(a) AS c RETURN a.id AS id, c ORDER BY id",
    );
    assert_eq!(
        rows,
        vec![
            vec!["Int64(1)".to_string(), "Int64(1)".to_string()],
            vec!["Int64(2)".to_string(), "Int64(1)".to_string()],
        ],
        "bare-var group key (no optional) must count 1 per node, got: {rows:?}"
    );
}

#[test]
fn test_p5340_bare_var_group_key_where() {
    // Production shape: bare node variable group key + WHERE on the alias.
    let conn = setup_graph();
    let rows = query_rows(
        &conn,
        "MATCH (a:T) WITH a, count(a) AS c WHERE c >= 1 RETURN a.id AS id, c ORDER BY id",
    );
    assert_eq!(
        rows,
        vec![
            vec!["Int64(1)".to_string(), "Int64(1)".to_string()],
            vec!["Int64(2)".to_string(), "Int64(1)".to_string()],
        ],
        "bare-var group key + WHERE alias must work, got: {rows:?}"
    );
}

#[test]
fn test_p5340_optional_raw_rows_no_duplication() {
    // Diagnostic: raw rows out of the OPTIONAL MATCH pipeline — each node must
    // appear exactly ONCE (its single edge). Duplication here is the root cause
    // of the inflated COUNT downstream.
    let conn = setup_graph();
    let rows = query_rows(
        &conn,
        "MATCH (a:T) OPTIONAL MATCH (a)-[r:R]-(:T) RETURN a.id AS id, r ORDER BY id",
    );
    assert_eq!(
        rows.len(),
        2,
        "OPTIONAL MATCH must emit exactly one row per node (1 edge in graph), got: {rows:?}"
    );
}

#[test]
fn test_p5340_optional_direct_count() {
    // Diagnostic: COUNT directly over the OPTIONAL MATCH output (no WITH).
    let conn = setup_graph();
    let rows = query_rows(
        &conn,
        "MATCH (a:T) OPTIONAL MATCH (a)-[r:R]-(:T) RETURN a.id AS id, COUNT(r) AS c ORDER BY id",
    );
    assert_eq!(
        rows,
        vec![
            vec!["Int64(1)".to_string(), "Int64(1)".to_string()],
            vec!["Int64(2)".to_string(), "Int64(1)".to_string()],
        ],
        "direct COUNT over OPTIONAL MATCH must be 1 per node, got: {rows:?}"
    );
}

#[test]
fn test_p5340_optional_order_by_and_limit_respected() {
    // P53.40: the scan_ops-empty fast path dropped ORDER BY and LIMIT for any
    // query whose last pipeline clause was an OPTIONAL MATCH. Both must apply.
    let conn = setup_graph();
    let rows = query_rows(
        &conn,
        "MATCH (a:T) OPTIONAL MATCH (a)-[r:R]-(:T) \
         RETURN a.id AS id ORDER BY id DESC LIMIT 1",
    );
    assert_eq!(
        rows,
        vec![vec!["Int64(2)".to_string()]],
        "ORDER BY DESC + LIMIT 1 after OPTIONAL MATCH must yield exactly the max id, got: {rows:?}"
    );
}
