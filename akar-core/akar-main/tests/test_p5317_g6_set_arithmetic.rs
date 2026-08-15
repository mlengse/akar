//! P53.17 — `SET n.prop = <expression>` evaluates the expression against the
//! OLD (pre-update) row values (G6).
//!
//! Evidence (audit-p53-compat-harness.md): `SET m.access_count = m.access_count + 1`
//! wrote `None` instead of incrementing, and a two-item `SET m.a=123.0, m.b=m.b+1`
//! left `m.b` at its old value. Two bugs:
//!
//! 1. `evaluate_expression_for_row` only handled constants/lists/maps — any other
//!    expression (binary arithmetic, property reads) fell through to a stub that
//!    returned `chunk.get_value(1, row)` (column 1), never evaluating the RHS.
//! 2. A multi-item SET is planned as a chain `[Scan, Set(a), Set(b)]`; each item
//!    after the first only receives the previous item's *count* chunk, so the
//!    scan columns were unavailable and `b+1` could never see the old `b`.
//!
//! Fix: `PhysicalSet` evaluates its value expression against the pre-update row
//! data read from the table itself (via the function registry's expression
//! evaluator), keyed by the `_id` row offsets the scan emits. Self-referential
//! and multi-item SETs therefore always read true pre-write values.

mod common;

use common::*;

fn setup(conn: &Connection) {
    exec(
        conn,
        "CREATE NODE TABLE Counter (id INT64, access_count INT64, total INT64, score DOUBLE, PRIMARY KEY (id))",
    );
    exec(
        conn,
        "CREATE (c:Counter {id: 1, access_count: 0, total: 10, score: 0.0})",
    );
    exec(
        conn,
        "CREATE (c:Counter {id: 2, access_count: 3, total: 100, score: 4.0})",
    );
    exec(
        conn,
        "CREATE (c:Counter {id: 5, access_count: 700, total: 1000, score: 2.5})",
    );
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

/// G6 probe: `SET m.access_count = m.access_count + 1` previously wrote NULL.
#[test]
fn set_self_reference_increments() {
    let (_db, conn) = setup_db();
    setup(&conn);

    exec(
        &conn,
        "MATCH (m:Counter {id: 1}) SET m.access_count = m.access_count + 1",
    );
    let rows = read_rows(&conn, "MATCH (m:Counter {id: 1}) RETURN m.access_count");
    assert_eq!(rows, vec![vec![Value::Int64(1)]], "first increment reads old 0");

    exec(
        &conn,
        "MATCH (m:Counter {id: 1}) SET m.access_count = m.access_count + 1",
    );
    let rows = read_rows(&conn, "MATCH (m:Counter {id: 1}) RETURN m.access_count");
    assert_eq!(rows, vec![vec![Value::Int64(2)]], "second increment reads old 1");
}

/// Arithmetic across two property columns plus a literal.
#[test]
fn set_arithmetic_combines_columns() {
    let (_db, conn) = setup_db();
    setup(&conn);

    exec(&conn, "MATCH (m:Counter {id: 2}) SET m.total = m.access_count * 2 + 1");
    let rows = read_rows(&conn, "MATCH (m:Counter {id: 2}) RETURN m.total");
    assert_eq!(rows, vec![vec![Value::Int64(7)]], "3 * 2 + 1");
}

/// G6 probe: two-item SET must apply BOTH items, and the second item must see
/// the pre-update value of its own column (not 0, not a stale column-1 read).
#[test]
fn set_multi_item_evaluates_old_state() {
    let (_db, conn) = setup_db();
    setup(&conn);

    exec(
        &conn,
        "MATCH (m:Counter {id: 2}) SET m.score = 123.0, m.total = m.total + 1",
    );
    let rows = read_rows(&conn, "MATCH (m:Counter {id: 2}) RETURN m.score, m.total");
    assert_eq!(
        rows,
        vec![vec![Value::Double(123.0), Value::Int64(101)]],
        "both items applied; total read old 100"
    );
}

/// Each item of a multi-item SET reads the PRE-write state, so a later item
/// referencing an earlier item's target column uses the old value.
#[test]
fn set_multi_item_self_reference_uses_old_value() {
    let (_db, conn) = setup_db();
    setup(&conn);

    exec(
        &conn,
        "MATCH (m:Counter {id: 5}) SET m.access_count = m.access_count + 1, m.total = m.access_count * 10",
    );
    let rows = read_rows(&conn, "MATCH (m:Counter {id: 5}) RETURN m.access_count, m.total");
    assert_eq!(
        rows,
        vec![vec![Value::Int64(701), Value::Int64(7000)]],
        "total reads OLD access_count 700 (not the incremented 701)"
    );
}

/// `SET n.x = n.y` copies a property value (property access on the RHS).
#[test]
fn set_property_to_property() {
    let (_db, conn) = setup_db();
    setup(&conn);

    exec(&conn, "MATCH (m:Counter {id: 1}) SET m.total = m.access_count");
    let rows = read_rows(&conn, "MATCH (m:Counter {id: 1}) RETURN m.total");
    assert_eq!(rows, vec![vec![Value::Int64(0)]], "copies access_count value");
}

/// Function calls in SET values require the registry (COALESCE fold).
#[test]
fn set_function_call_in_value() {
    let (_db, conn) = setup_db();
    setup(&conn);

    exec(
        &conn,
        "MATCH (m:Counter {id: 2}) SET m.total = COALESCE(m.total, 0) + 1",
    );
    let rows = read_rows(&conn, "MATCH (m:Counter {id: 2}) RETURN m.total");
    assert_eq!(rows, vec![vec![Value::Int64(101)]], "registry-evaluated RHS");
}

/// Floating-point arithmetic on a DOUBLE column.
#[test]
fn set_float_arithmetic() {
    let (_db, conn) = setup_db();
    setup(&conn);

    exec(&conn, "MATCH (m:Counter {id: 5}) SET m.score = m.score * 1.5");
    let rows = read_rows(&conn, "MATCH (m:Counter {id: 5}) RETURN m.score");
    assert_eq!(rows, vec![vec![Value::Double(3.75)]], "2.5 * 1.5");
}

/// Predicate-scoped SET: only matching rows update, and row offsets map
/// correctly through the filtered scan.
#[test]
fn set_filtered_rows_only() {
    let (_db, conn) = setup_db();
    setup(&conn);

    exec(
        &conn,
        "MATCH (m:Counter) WHERE m.access_count < 100 SET m.access_count = m.access_count + 10",
    );
    let rows = read_rows(&conn, "MATCH (m:Counter) RETURN m.id, m.access_count ORDER BY m.id");
    assert_eq!(
        rows,
        vec![
            vec![Value::Int64(1), Value::Int64(10)],
            vec![Value::Int64(2), Value::Int64(13)],
            vec![Value::Int64(5), Value::Int64(700)],
        ],
        "only rows with access_count < 100 increment (id 5 is 700, untouched)"
    );
}
