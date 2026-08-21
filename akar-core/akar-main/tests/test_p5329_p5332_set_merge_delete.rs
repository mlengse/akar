//! P53.29–P53.32 regression tests — SET / MERGE / DELETE write correctness.
//!
//! Kairos drop-in gap analysis found that a write operator's result was a bare
//! count chunk, so a following RETURN (or the same-statement SET/MERGE write)
//! misbehaved:
//! - P53.29: SET value evaluation dropped complex literals (FLOAT[] embeddings)
//!   to `Value::Null` via the legacy ValueVector (no List storage).
//! - P53.30: `MATCH ... SET ... RETURN n.prop` resolved `n.prop` against the
//!   SET operator's count chunk instead of the post-update row.
//! - P53.31: `MERGE ... SET ... RETURN n.prop` wrote the new value to the wrong
//!   physical row (row id = merged count) and returned the count.
//! - P53.32: a soft-deleted row kept its (all-null) slot, so full scans still
//!   returned the deleted node as a row of `null`s.

mod common;
use common::*;

fn setup_chain() -> (std::sync::Arc<Database>, Connection) {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Chain(id INT64, value INT64, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (:Chain {id: 1, value: 100})");
    (_db, conn)
}

#[test]
fn test_p5330_match_set_return_value() {
    // `MATCH ... SET ... RETURN n.value` must return the post-update value, not
    // the SET operator's count chunk (P53.30).
    let (_db, conn) = setup_chain();
    let rows = query_rows(&conn, "MATCH (c:Chain {id: 1}) SET c.value = 300 RETURN c.value");
    assert_eq!(
        rows,
        vec![vec!["Int64(300)".to_string()]],
        "RETURN after SET must yield the updated value, got: {rows:?}"
    );
    let after = query_rows(&conn, "MATCH (c:Chain {id: 1}) RETURN c.value");
    assert_eq!(after, vec![vec!["Int64(300)".to_string()]]);
}

#[test]
fn test_p5330_match_set_return_multi_rows() {
    // Per-row values must survive SET (not one count per statement).
    let (_db, conn) = setup_chain();
    exec(&conn, "CREATE (:Chain {id: 2, value: 200})");
    let rows = query_rows(
        &conn,
        "MATCH (c:Chain) SET c.value = c.id * 10 RETURN c.id, c.value ORDER BY c.id",
    );
    assert_eq!(
        rows,
        vec![
            vec!["Int64(1)".to_string(), "Int64(10)".to_string()],
            vec!["Int64(2)".to_string(), "Int64(20)".to_string()],
        ],
        "SET must write per-row values, got: {rows:?}"
    );
}

#[test]
fn test_p5339_match_set_return_no_match_empty() {
    // P53.39 / kairos P59.1: `MATCH ... SET ... RETURN` with NO matching row
    // must return ZERO rows — previously the SET emitted a phantom `count=0`
    // chunk that the RETURN projection turned into `[[0]]`.
    let (_db, conn) = setup_chain();
    let rows = query_rows(
        &conn,
        "MATCH (c:Chain {id: 999}) SET c.value = 42 RETURN c.id",
    );
    assert_eq!(
        rows,
        Vec::<Vec<String>>::new(),
        "MATCH..SET..RETURN with no match must return 0 rows, got: {rows:?}"
    );
}

#[test]
fn test_p5339_match_set_no_return_reports_zero() {
    // Terminal SET (no RETURN) still reports "updated: 0" via the count column.
    let (_db, conn) = setup_chain();
    let res = conn
        .query("MATCH (c:Chain {id: 999}) SET c.value = 42")
        .map_err(|e| e.to_string())
        .unwrap();
    let chunk = res.chunks.first().unwrap();
    let val = chunk.get_i64(0, 0).unwrap();
    assert_eq!(val, 0, "terminal SET with no match should report 0 updated");
}

#[test]
fn test_p5331_merge_set_return_create_writes_value() {
    // On the CREATE path, `MERGE ... SET ... RETURN n.value` must write the new
    // value to the created row and return it (previously row id = merged count
    // was out of range, so the write was silently dropped → null).
    let (_db, conn) = setup_chain();
    let rows = query_rows(&conn, "MERGE (c:Chain {id: 5}) SET c.value = 500 RETURN c.value");
    assert_eq!(
        rows,
        vec![vec!["Int64(500)".to_string()]],
        "MERGE CREATE + SET must write and return the value, got: {rows:?}"
    );
    let after = query_rows(&conn, "MATCH (c:Chain {id: 5}) RETURN c.value");
    assert_eq!(after, vec![vec!["Int64(500)".to_string()]]);
}

#[test]
fn test_p5331_merge_set_return_match_updates_value() {
    // On the MATCH path the value must be written and returned (previously the
    // SET targeted row 0 by luck and RETURN resolved the count chunk).
    let (_db, conn) = setup_chain();
    exec(&conn, "CREATE (:Chain {id: 6, value: 600})");
    let rows = query_rows(&conn, "MERGE (c:Chain {id: 6}) SET c.value = 601 RETURN c.value");
    assert_eq!(
        rows,
        vec![vec!["Int64(601)".to_string()]],
        "MERGE MATCH + SET must update and return the value, got: {rows:?}"
    );
    let after = query_rows(&conn, "MATCH (c:Chain {id: 6}) RETURN c.value");
    assert_eq!(after, vec![vec!["Int64(601)".to_string()]]);
}

#[test]
fn test_p5331_merge_on_create_set_return() {
    // ON CREATE SET (statement-level MERGE, no bare SET clause) is executed by
    // the connection-level handler and returns a success message rather than
    // rows; the SET write itself must persist (read-after check).
    let (_db, conn) = setup_chain();
    let result = conn
        .query("MERGE (c:Chain {id: 7}) ON CREATE SET c.value = 700")
        .unwrap();
    assert!(result.is_success(), "ON CREATE SET should succeed");
    let after = query_rows(&conn, "MATCH (c:Chain {id: 7}) RETURN c.value");
    assert_eq!(
        after,
        vec![vec!["Int64(700)".to_string()]],
        "ON CREATE SET must write the value, got: {after:?}"
    );
}

#[test]
fn test_p5331_unwind_merge_set_multi_row() {
    // Kairos `_store_many` shape: one MERGE+SET per UNWIND element, mixing a
    // matched and a created row in the same statement. Each element's SET value
    // must target its own row (P53.31) and the UNWIND variable must resolve.
    let (_db, conn) = setup_chain();
    let rows = query_rows(
        &conn,
        "UNWIND [{id: 1, v: 1000}, {id: 8, v: 800}] AS r \
         MERGE (c:Chain {id: r.id}) SET c.value = r.v RETURN c.id, c.value ORDER BY c.id",
    );
    assert_eq!(
        rows,
        vec![
            vec!["Int64(1)".to_string(), "Int64(1000)".to_string()],
            vec!["Int64(8)".to_string(), "Int64(800)".to_string()],
        ],
        "UNWIND → MERGE → SET must apply per element, got: {rows:?}"
    );
    let after = query_rows(&conn, "MATCH (c:Chain) RETURN c.id, c.value ORDER BY c.id");
    assert_eq!(
        after,
        vec![
            vec!["Int64(1)".to_string(), "Int64(1000)".to_string()],
            vec!["Int64(8)".to_string(), "Int64(800)".to_string()],
        ],
        "both rows must persist their per-element SET value, got: {after:?}"
    );
}

#[test]
fn test_p5329_set_list_literal_roundtrip() {
    // SET with a FLOAT[] (embedding) literal must write the list, not null
    // (P53.29): the legacy ValueVector has no List storage.
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Embedding(id INT64, vec FLOAT[], PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (:Embedding {id: 1, vec: [1.0, 2.0, 3.0]})");
    let rows = query_rows(
        &conn,
        "MATCH (e:Embedding {id: 1}) SET e.vec = [4.0, 5.0, 6.0] RETURN e.vec",
    );
    assert_eq!(
        rows,
        vec![vec!["List([Double(4.0), Double(5.0), Double(6.0)])".to_string()]],
        "SET of a list literal must return the list, got: {rows:?}"
    );
    let after = query_rows(&conn, "MATCH (e:Embedding {id: 1}) RETURN e.vec");
    assert_eq!(
        after,
        vec![vec!["List([Double(4.0), Double(5.0), Double(6.0)])".to_string()]],
        "the written list must survive a full scan, got: {after:?}"
    );
}

#[test]
fn test_p5332_delete_row_gone_from_scan() {
    // A soft-deleted row keeps its slot but every column is nulled; scans must
    // not return the deleted node (P53.32).
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE ConnectionHistory(history_id STRING, PRIMARY KEY (history_id))",
    );
    exec(&conn, "CREATE (:ConnectionHistory {history_id: 'h1'})");
    exec(&conn, "CREATE (:ConnectionHistory {history_id: 'h2'})");
    let del = query_rows(
        &conn,
        "MATCH (h:ConnectionHistory {history_id: 'h1'}) DELETE h RETURN count(*)",
    );
    assert_eq!(del, vec![vec!["Int64(1)".to_string()]]);
    let after = query_rows(
        &conn,
        "MATCH (h:ConnectionHistory) RETURN h.history_id ORDER BY h.history_id",
    );
    assert_eq!(
        after,
        vec![vec!["String(\"h2\")".to_string()]],
        "deleted row must not appear in a full scan, got: {after:?}"
    );
}

#[test]
fn test_p5332_delete_then_reinsert_same_pk() {
    // The deleted PK is dropped from the index, so the same value is reusable.
    let (_db, conn) = setup_chain();
    exec(&conn, "MATCH (c:Chain {id: 1}) DELETE c");
    exec(&conn, "CREATE (:Chain {id: 1, value: 999})");
    let rows = query_rows(&conn, "MATCH (c:Chain) RETURN c.id, c.value");
    assert_eq!(
        rows,
        vec![vec!["Int64(1)".to_string(), "Int64(999)".to_string()]],
        "same PK must be re-insertable after DELETE, got: {rows:?}"
    );
}
