//! P53.16 — RETURN/WITH `AS` aliases are honored (G5).
//!
//! `RETURN count(m) AS cnt` must expose the result column as `cnt` (not
//! `COUNT(m)`), and ORDER BY must be able to reference projected aliases
//! (`RETURN m.age AS a ORDER BY a`) the way Kuzu/Neo4j allow.
//!
//! Before the fix: `bind_return` dropped `ReturnItem.alias` entirely
//! (`binder/mod.rs`), so output column names came from the expression text;
//! ORDER BY items were resolved only against MATCH-bound variables, so an
//! alias reference failed with `Variable 'cnt' not in scope`. The optimizer's
//! `AggregateDetection` also rebuilt projection expressions with `alias: None`.
//!
//! Fix: `BoundExpression` carries `alias`; `bind_return` copies it from the
//! parser `AS` clause and resolves ORDER BY alias references against the
//! projected aliases (alias shadows scope variables); `map_projection` names
//! output columns from the alias (eval and plain-column paths); the aggregate
//! rewrite keeps the projection above the Aggregate and preserves the alias.

mod common;

use common::*;

/// Insert Memory rows with distinct ids, a numeric `val`, and a `grp`.
fn setup(conn: &Connection) {
    exec(
        conn,
        "CREATE NODE TABLE Memory (id INT64, val INT64, grp STRING, PRIMARY KEY (id))",
    );
    exec(conn, "CREATE (m:Memory {id: 1, val: 10, grp: 'a'})");
    exec(conn, "CREATE (m:Memory {id: 2, val: 20, grp: 'a'})");
    exec(conn, "CREATE (m:Memory {id: 5, val: 30, grp: 'b'})");
}

fn column_names(conn: &Connection, sql: &str) -> Vec<String> {
    let result = query(conn, sql);
    result.chunks.first().map(|c| c.field_names.clone()).unwrap_or_default()
}

fn read_rows(conn: &Connection, sql: &str) -> Vec<Vec<Value>> {
    let result = query(conn, sql);
    let mut rows = Vec::new();
    for chunk in &result.chunks {
        for row in chunk.iter_rows() {
            let mut vals = Vec::new();
            for col in 0..chunk.fields.len() {
                vals.push(chunk.get_value(col, row).unwrap());
            }
            rows.push(vals);
        }
    }
    rows
}

#[test]
fn alias_applied_to_aggregate_column() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // G5 probe: `count(m) AS cnt` → column named `cnt`, not `COUNT(m)`.
    let names = column_names(&conn, "MATCH (m:Memory) RETURN count(m) AS cnt");
    assert_eq!(names, vec!["cnt"], "aggregate alias becomes the output column name");

    let rows = read_rows(&conn, "MATCH (m:Memory) RETURN count(m) AS cnt");
    assert_eq!(rows, vec![vec![Value::Int64(3)]]);
}

#[test]
fn alias_applied_to_nested_aggregate() {
    let (_db, conn) = setup_db();
    setup(&conn);

    let names = column_names(&conn, "MATCH (m:Memory) RETURN COALESCE(MAX(m.id), 0) AS mx");
    assert_eq!(
        names,
        vec!["mx"],
        "nested-aggregate alias becomes the output column name"
    );

    let rows = read_rows(&conn, "MATCH (m:Memory) RETURN COALESCE(MAX(m.id), 0) AS mx");
    assert_eq!(rows, vec![vec![Value::Int64(5)]]);
}

#[test]
fn alias_applied_to_property() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // Plain-column projection (non-eval path) must also honor the alias.
    let names = column_names(&conn, "MATCH (m:Memory) RETURN m.grp AS g, m.val AS v");
    assert_eq!(names, vec!["g".to_string(), "v".to_string()]);
}

#[test]
fn alias_applied_to_constant_expression() {
    let (_db, conn) = setup_db();
    setup(&conn);

    let names = column_names(&conn, "MATCH (m:Memory) RETURN 1 + 1 AS two");
    assert_eq!(names, vec!["two"], "expression alias becomes the output column name");

    let rows = read_rows(&conn, "MATCH (m:Memory) RETURN 1 + 1 AS two");
    assert_eq!(
        rows,
        vec![vec![Value::Int64(2)], vec![Value::Int64(2)], vec![Value::Int64(2)]],
        "constant expression is emitted once per scanned row"
    );
}

#[test]
fn order_by_resolves_return_alias() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // ORDER BY must reference the projected alias, not the source variable.
    // Anti-correlate val vs grp so sorting by alias `v` (5,10,20,30) differs
    // from sorting by `g` (a,a,b,b) — a plain grp-sort would fail this.
    exec(&conn, "CREATE (m:Memory {id: 3, val: 5, grp: 'b'})");
    let rows = read_rows(&conn, "MATCH (m:Memory) RETURN m.grp AS g, m.val AS v ORDER BY v");
    assert_eq!(
        rows,
        vec![
            vec![Value::String("b".into()), Value::Int64(5)],
            vec![Value::String("a".into()), Value::Int64(10)],
            vec![Value::String("a".into()), Value::Int64(20)],
            vec![Value::String("b".into()), Value::Int64(30)],
        ],
        "ORDER BY alias sorts by the projected column"
    );
}

#[test]
fn order_by_alias_on_aggregate() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // Aggregate alias referenced from ORDER BY.
    // Add group 'c' (count 1) so sorting by alias `g` (a,b,c) differs from
    // sorting by `cnt` (b,c,a) — a wrong-column sort would fail this.
    exec(&conn, "CREATE (m:Memory {id: 9, val: 1, grp: 'c'})");
    let names = column_names(
        &conn,
        "MATCH (m:Memory) RETURN m.grp AS g, count(m.id) AS cnt ORDER BY g",
    );
    assert_eq!(names, vec!["g".to_string(), "cnt".to_string()]);

    let rows = read_rows(
        &conn,
        "MATCH (m:Memory) RETURN m.grp AS g, count(m.id) AS cnt ORDER BY g",
    );
    assert_eq!(
        rows,
        vec![
            vec![Value::String("a".into()), Value::Int64(2)],
            vec![Value::String("b".into()), Value::Int64(1)],
            vec![Value::String("c".into()), Value::Int64(1)],
        ],
        "group-by + aggregate alias + ORDER BY alias"
    );
}

#[test]
fn no_alias_keeps_expression_name() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // Regression: without an alias the aggregate keeps its expression name.
    let names = column_names(&conn, "MATCH (m:Memory) RETURN count(m)");
    assert_eq!(names, vec!["COUNT(m)"]);
}
