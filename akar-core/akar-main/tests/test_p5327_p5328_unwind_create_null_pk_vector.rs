//! P53.27/P53.28 regression tests — UNWIND+CREATE row materialization and
//! UNWIND+MATCH vector-column reads. Mirrors Kairos drop-in changes 1.2/1.3:
//! - 1.2 (P53.27): a NULL primary key in CREATE must error loudly instead of
//!   silently skipping the row (UNWIND+CREATE was a silent no-op).
//! - 1.3 (P53.28): every UNWIND element must produce a MATCH row (probe F gave
//!   1 of 2) and FLOAT[n] vector columns must be read back (probe F gave
//!   `embedding=NULL`).

mod common;
use common::*;

#[test]
fn test_p5327_unwind_create_null_pk_errors_loudly() {
    // `UNWIND $rows AS r CREATE (m:Memory {content: r.content})` with no `id`
    // per element → NULL primary key. Must error, not silently create 0 rows.
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, PRIMARY KEY (id))",
    );
    let err = exec_err(
        &conn,
        "UNWIND [{content: 'x'}, {content: 'y'}] AS r CREATE (m:Memory {content: r.content})",
    );
    assert!(
        err.contains("NULL value not allowed for primary key"),
        "expected a loud NULL-PK error, got: {err:?}"
    );
    let rows = query_rows(&conn, "MATCH (m:Memory) RETURN m.id");
    assert!(
        rows.is_empty(),
        "no rows may be created when every element has a NULL PK, got: {rows:?}"
    );
}

#[test]
fn test_p5327_unwind_create_with_pk_inserts_all_rows() {
    // Positive control: map UNWIND→CREATE with an explicit PK inserts one row
    // per element (regression guard for P53.26 map materialization).
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, PRIMARY KEY (id))",
    );
    exec(
        &conn,
        "UNWIND [{id: 1, content: 'a'}, {id: 2, content: 'b'}, {id: 3, content: 'c'}] AS r \
         CREATE (m:Memory {id: r.id, content: r.content})",
    );
    let rows = query_rows(&conn, "MATCH (m:Memory) RETURN m.id ORDER BY m.id");
    assert_eq!(
        rows,
        vec![
            vec!["Int64(1)".to_string()],
            vec!["Int64(2)".to_string()],
            vec!["Int64(3)".to_string()],
        ],
        "every UNWIND element must insert a node, got: {rows:?}"
    );
}

#[test]
fn test_p5328_unwind_match_all_rows_with_vector_column() {
    // `UNWIND $ids AS iid MATCH (m:Memory {id: iid}) RETURN m.id, m.embedding`
    // must produce one row per element (probe F gave 1 of 2) and read the
    // FLOAT[] embedding back as a non-NULL list (probe F gave `embedding=NULL`).
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, embedding FLOAT[], PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (:Memory {id: 1, content: 'one', embedding: [0.1, 0.2, 0.3]})");
    exec(&conn, "CREATE (:Memory {id: 2, content: 'two', embedding: [0.4, 0.5, 0.6]})");

    let rows = query_rows(
        &conn,
        "UNWIND [1, 2] AS iid MATCH (m:Memory {id: iid}) RETURN m.id, m.embedding",
    );
    assert_eq!(
        rows.len(),
        2,
        "every UNWIND element must produce a joined row, got: {rows:?}"
    );
    let ids: Vec<&str> = rows.iter().map(|r| r[0].as_str()).collect();
    assert_eq!(ids, vec!["Int64(1)", "Int64(2)"], "rows: {rows:?}");

    for (i, row) in rows.iter().enumerate() {
        assert!(
            !row[1].starts_with("null"),
            "embedding of element {} must be non-NULL, got: {:?}",
            i + 1,
            row[1]
        );
        assert!(
            row[1].contains("Double(0."),
            "embedding of element {} must contain float values, got: {:?}",
            i + 1,
            row[1]
        );
    }
}

#[test]
fn test_p5328_unwind_match_vector_column_join_keeps_values() {
    // The join output must preserve the vector column across the UNWIND→MATCH
    // join (not NULL out complex columns), matching the order of the UNWIND list.
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, embedding FLOAT[], PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (:Memory {id: 1, content: 'one', embedding: [0.5]})");
    exec(&conn, "CREATE (:Memory {id: 2, content: 'two', embedding: [0.75]})");

    // List order reversed vs id order — both rows must still be produced (join
    // output order is not guaranteed, so assert per-id).
    let rows = query_rows(
        &conn,
        "UNWIND [2, 1] AS iid MATCH (m:Memory {id: iid}) RETURN iid, m.embedding",
    );
    assert_eq!(
        rows.len(),
        2,
        "both UNWIND elements must produce rows, got: {rows:?}"
    );
    let by_id: std::collections::HashMap<String, String> = rows
        .iter()
        .map(|r| (r[0].clone(), r[1].clone()))
        .collect();
    assert_eq!(by_id.len(), 2, "rows: {rows:?}");
    assert!(
        by_id["Int64(1)"].contains("0.5"),
        "id 1 embedding, got: {rows:?}"
    );
    assert!(
        by_id["Int64(2)"].contains("0.75"),
        "id 2 embedding, got: {rows:?}"
    );
}
