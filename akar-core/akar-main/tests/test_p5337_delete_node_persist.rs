//! P53.37c — DELETE node rows must persist (kairos `prune_connection_history`, audit D1).
//!
//! Kairos query:
//!   MATCH (h:ConnectionHistory) WHERE h.changed_at < $cutoff DELETE h RETURN count(*)
//! Probe J2: `deleted=1` yet a re-count still returns 1 — the soft-deleted node
//! row (all columns NULLed) is still emitted by the MVCC snapshot scan because
//! `to_column_major_data_with_snapshot_and_predicate_and_ids` pushes
//! `Value::Null` for invisible rows without skipping the row itself.

mod common;
use common::*;

/// D1 shape: DELETE nodes selected by a WHERE clause, then re-count.
#[test]
fn test_p5337_delete_node_persists() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE ConnectionHistory(id INT64, changed_at DOUBLE, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (:ConnectionHistory {id: 1, changed_at: 100.0})");
    exec(&conn, "CREATE (:ConnectionHistory {id: 2, changed_at: 200.0})");

    // kairos prune_connection_history shape: WHERE changed_at < cutoff → DELETE → count.
    let rows = query_rows(
        &conn,
        "MATCH (h:ConnectionHistory) WHERE h.changed_at < 150.0 DELETE h RETURN count(*)",
    );
    assert_eq!(
        rows,
        vec![vec!["Int64(1)".to_string()]],
        "DELETE must report 1 deleted, got: {rows:?}"
    );

    // The deleted node must be gone from a fresh read (audit D1: count ulang 1).
    let rows = query_rows(&conn, "MATCH (h:ConnectionHistory) RETURN count(*)");
    assert_eq!(
        rows,
        vec![vec!["Int64(1)".to_string()]],
        "deleted node must not re-appear in count, got: {rows:?}"
    );
    let rows = query_rows(&conn, "MATCH (h:ConnectionHistory) RETURN h.id");
    assert_eq!(
        rows,
        vec![vec!["Int64(2)".to_string()]],
        "only the surviving node must remain, got: {rows:?}"
    );
}

/// D1 variant: DELETE every row — table must be empty afterwards.
#[test]
fn test_p5337_delete_all_nodes_leaves_empty_table() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE ConnectionHistory(id INT64, changed_at DOUBLE, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (:ConnectionHistory {id: 1, changed_at: 100.0})");
    exec(&conn, "CREATE (:ConnectionHistory {id: 2, changed_at: 200.0})");

    let rows = query_rows(&conn, "MATCH (h:ConnectionHistory) DELETE h RETURN count(*)");
    assert_eq!(rows, vec![vec!["Int64(2)".to_string()]]);

    let rows = query_rows(&conn, "MATCH (h:ConnectionHistory) RETURN count(*)");
    assert_eq!(
        rows,
        vec![vec!["Int64(0)".to_string()]],
        "table must be empty after deleting all nodes, got: {rows:?}"
    );
}

/// D1 exact probe: a single node deleted via the kairos query shape must not
/// re-appear in a re-count.
#[test]
fn test_p5337_delete_single_node_then_count() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE ConnectionHistory(id INT64, changed_at DOUBLE, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (:ConnectionHistory {id: 1, changed_at: 100.0})");

    let rows = query_rows(
        &conn,
        "MATCH (h:ConnectionHistory) WHERE h.changed_at < 150.0 DELETE h RETURN count(*)",
    );
    assert_eq!(
        rows,
        vec![vec!["Int64(1)".to_string()]],
        "deleted=1 expected, got: {rows:?}"
    );
    let rows = query_rows(&conn, "MATCH (h:ConnectionHistory) RETURN count(*)");
    assert_eq!(
        rows,
        vec![vec!["Int64(0)".to_string()]],
        "node must be gone after DELETE, got: {rows:?}"
    );
}

/// D1 harness shape: `prune_connection_history` on an empty table must report 0.
#[test]
fn test_p5337_delete_zero_rows_reports_zero() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE ConnectionHistory(id INT64, changed_at DOUBLE, PRIMARY KEY (id))",
    );
    let rows = query_rows(
        &conn,
        "MATCH (h:ConnectionHistory) WHERE h.changed_at IS NOT NULL AND h.changed_at < 999.0 DELETE h RETURN count(*)",
    );
    assert_eq!(
        rows,
        vec![vec!["Int64(0)".to_string()]],
        "zero rows deleted must report 0, got: {rows:?}"
    );
}

/// D1 contrast: rel-edge DELETE already persists (prune_weak passes); node
/// tables must behave the same — deleted node id is re-insertable.
#[test]
fn test_p5337_deleted_pk_is_reinsertable() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE ConnectionHistory(id INT64, changed_at DOUBLE, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (:ConnectionHistory {id: 1, changed_at: 100.0})");

    exec(&conn, "MATCH (h:ConnectionHistory) DELETE h");
    // Same PK must be insertable again after the soft delete removed it from
    // the PK indexes (P52.16).
    exec(&conn, "CREATE (:ConnectionHistory {id: 1, changed_at: 300.0})");
    let rows = query_rows(&conn, "MATCH (h:ConnectionHistory) RETURN h.id, h.changed_at");
    assert_eq!(
        rows,
        vec![vec!["Int64(1)".to_string(), "Double(300.0)".to_string()]],
        "re-inserted node with the old PK must be the only row, got: {rows:?}"
    );
}
