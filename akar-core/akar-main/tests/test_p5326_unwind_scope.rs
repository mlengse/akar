//! P53.26 regression tests — UNWIND clause variable visibility in subsequent
//! clauses (MATCH / CREATE / SET / MERGE). Mirrors the Kairos drop-in failures
//! A1 (scalar UNWIND→MATCH returns 1 of 2 rows), A2/A3 (map UNWIND→MATCH→SET
//! "Variable 'row' not found"), A4 (map UNWIND→CREATE silent no-op).

mod common;
use common::*;

fn setup_memory() -> (std::sync::Arc<Database>, Connection) {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, access_count INT64, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (:Memory {id: 1, content: 'a', access_count: 0})");
    exec(&conn, "CREATE (:Memory {id: 2, content: 'b', access_count: 0})");
    (_db, conn)
}

#[test]
fn test_p5326_unwind_scalar_match_all_rows() {
    // A1: `UNWIND $ids AS iid MATCH (m:Memory {id: iid}) RETURN ...` with ids=[1,2]
    // must produce 2 rows (one per element), not 1.
    let (_db, conn) = setup_memory();
    let rows = query_rows(&conn, "UNWIND [1, 2] AS iid MATCH (m:Memory {id: iid}) RETURN m.id");
    assert_eq!(
        rows,
        vec![vec!["Int64(1)".to_string()], vec!["Int64(2)".to_string()]],
        "each UNWIND element must produce a joined row, got: {rows:?}"
    );
}

#[test]
fn test_p5326_unwind_map_create_roundtrip() {
    // A4: `UNWIND $rows AS r CREATE (m:Memory {... r.id ...})` — a list of maps
    // must insert one node per element (previously silent no-op → all properties
    // NULL → PK-NULL rows skipped).
    let (_db, conn) = setup_memory();
    exec(
        &conn,
        "UNWIND [{id: 3, content: 'c'}, {id: 4, content: 'd'}] AS row \
         CREATE (m:Memory {id: row.id, content: row.content})",
    );
    let rows = query_rows(&conn, "MATCH (m:Memory) RETURN m.id ORDER BY m.id");
    assert_eq!(
        rows,
        vec![
            vec!["Int64(1)".to_string()],
            vec!["Int64(2)".to_string()],
            vec!["Int64(3)".to_string()],
            vec!["Int64(4)".to_string()],
        ],
        "UNWIND CREATE must insert one row per element, got: {rows:?}"
    );
}

#[test]
fn test_p5326_unwind_map_match_set() {
    // A2/A3: `UNWIND $batch AS row MATCH (m:Memory {id: row.id}) SET m.content =
    // row.content` — the UNWIND variable `row` must resolve inside MATCH (via
    // `row.id`) and the SET value expression must read `row.content`.
    let (_db, conn) = setup_memory();
    exec(
        &conn,
        "UNWIND [{id: 1, content: 'updated'}] AS row \
         MATCH (m:Memory {id: row.id}) SET m.content = row.content",
    );
    let rows = query_rows(&conn, "MATCH (m:Memory {id: 1}) RETURN m.content");
    assert_eq!(
        rows,
        vec![vec!["String(\"updated\")".to_string()]],
        "SET must write the value read from the UNWIND variable, got: {rows:?}"
    );
}

#[test]
fn test_p5326_unwind_scalar_match_set_all_rows() {
    // P53.26 scope: UNWIND→MATCH→SET must apply to every matched row (one per
    // UNWIND element) and use the per-row UNWIND value in the SET expression.
    let (_db, conn) = setup_memory();
    exec(
        &conn,
        "UNWIND [1, 2] AS iid MATCH (m:Memory {id: iid}) SET m.access_count = iid",
    );
    let rows = query_rows(&conn, "MATCH (m:Memory) RETURN m.id, m.access_count ORDER BY m.id");
    assert_eq!(
        rows,
        vec![
            vec!["Int64(1)".to_string(), "Int64(1)".to_string()],
            vec!["Int64(2)".to_string(), "Int64(2)".to_string()],
        ],
        "SET must apply per UNWIND element, got: {rows:?}"
    );
}
