//! P53.37 — `ORDER BY` referencing a column that the `RETURN` projection does
//! not output must sort on the pre-projection columns.
//!
//! Evidence (kairos harness, search_bm25/search_entity): `MATCH (m:Memory) WHERE
//! ... RETURN m.id, m.label ORDER BY m.access_count DESC LIMIT $limit` failed
//! with `Variable 'm' not found in chunk field_names ["m.id", "m.label"]` because
//! the sort key was evaluated against the pruned projected chunk. Fix: the
//! planner pushes `ORDER BY` below the projection when a key is not covered by
//! the projection output (and there is no DISTINCT above).

mod common;

use common::*;

fn setup(conn: &Connection) {
    exec(
        conn,
        "CREATE NODE TABLE Memory (id INT64, label STRING, access_count INT64, PRIMARY KEY (id))",
    );
    exec(conn, "CREATE (:Memory {id: 1, label: 'b', access_count: 1})");
    exec(conn, "CREATE (:Memory {id: 2, label: 'a', access_count: 5})");
    exec(conn, "CREATE (:Memory {id: 3, label: 'c', access_count: 3})");
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

/// Kairos search shape: sort key not in the RETURN list.
#[test]
fn order_by_unprojected_column() {
    let (_db, conn) = setup_db();
    setup(&conn);

    let rows = read_rows(&conn, "MATCH (m:Memory) RETURN m.id, m.label ORDER BY m.access_count DESC");
    assert_eq!(
        rows,
        vec![
            vec![Value::Int64(2), Value::String("a".into())], // access 5
            vec![Value::Int64(3), Value::String("c".into())], // access 3
            vec![Value::Int64(1), Value::String("b".into())], // access 1
        ],
        "sorted by unprojected m.access_count DESC"
    );
}

/// Same, with a LIMIT (search_bm25 passes LIMIT $limit).
#[test]
fn order_by_unprojected_column_with_limit() {
    let (_db, conn) = setup_db();
    setup(&conn);

    let rows = read_rows(&conn, "MATCH (m:Memory) RETURN m.id ORDER BY m.access_count DESC LIMIT 2");
    assert_eq!(
        rows,
        vec![vec![Value::Int64(2)], vec![Value::Int64(3)]],
        "top-2 by m.access_count DESC"
    );
}

/// An aliased sort key (`RETURN m.access_count AS ac ORDER BY ac`) is covered by
/// the projection and must keep sorting on top.
#[test]
fn order_by_projected_alias() {
    let (_db, conn) = setup_db();
    setup(&conn);

    let rows = read_rows(&conn, "MATCH (m:Memory) RETURN m.access_count AS ac ORDER BY ac DESC");
    assert_eq!(
        rows,
        vec![
            vec![Value::Int64(5)],
            vec![Value::Int64(3)],
            vec![Value::Int64(1)],
        ],
        "ORDER BY projected alias"
    );
}

/// Sorting by a second unprojected property (label) still follows the scan
/// columns, not the projected output.
#[test]
fn order_by_second_unprojected_property() {
    let (_db, conn) = setup_db();
    setup(&conn);

    let rows = read_rows(&conn, "MATCH (m:Memory) RETURN m.id ORDER BY m.label DESC");
    assert_eq!(
        rows,
        vec![
            vec![Value::Int64(3)], // 'c'
            vec![Value::Int64(1)], // 'b'
            vec![Value::Int64(2)], // 'a'
        ],
        "ORDER BY m.label DESC on a non-projected column"
    );
}
