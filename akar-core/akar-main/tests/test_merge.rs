//! P1-MERGE-1 regression tests — fast-path (statement-level) MERGE match-or-create.
//!
//! The daemon's upsert shape `MERGE (n:N {pk: $pk}) SET n.x = $x` executes with
//! no RETURN clause, so the binder produces `BoundStatement::BoundMerge` and the
//! connection-level `handle_ddl` fast path runs it (ddl.rs). F2 observed that
//! such statement-level MERGEs failed to match existing rows:
//! - literal PK on an existing row → duplicate-primary-key error (should MATCH)
//! - parameterized PK → `NULL value not allowed for primary key` (PK evaluated
//!   as `Value::Null` instead of the substituted constant)
//!
//! These tests pin the fast-path behavior so any regression surfaces as a diff.

mod common;
use common::*;

fn setup_chain() -> (std::sync::Arc<Database>, Connection) {
    let (db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Chain(id INT64, value INT64, PRIMARY KEY (id))",
    );
    (db, conn)
}

#[test]
fn merge_statement_level_literal_matches_existing_row() {
    // Statement-level MERGE (no RETURN) must MATCH the row created via CREATE —
    // the fast path's existence check must agree with the INSERT hash index.
    let (_db, conn) = setup_chain();
    exec(&conn, "CREATE (:Chain {id: 1, value: 100})");

    let result = conn.query("MERGE (c:Chain {id: 1}) SET c.value = 200").unwrap();
    assert!(
        result.is_success(),
        "statement-level MERGE must succeed, got: {:?}",
        result.error_message
    );

    let after = query_rows(&conn, "MATCH (c:Chain {id: 1}) RETURN c.value");
    assert_eq!(
        after,
        vec![vec!["Int64(200)".to_string()]],
        "MERGE MATCH must update the existing row, got: {after:?}"
    );
}

#[test]
fn merge_statement_level_literal_idempotent_upsert() {
    // Running the same statement-level MERGE twice must not raise a duplicate
    // primary key error on the second run (F2 literal symptom).
    let (_db, conn) = setup_chain();
    exec(&conn, "CREATE (:Chain {id: 1, value: 100})");

    conn.query("MERGE (c:Chain {id: 1}) SET c.value = 200").unwrap();
    let second = conn.query("MERGE (c:Chain {id: 1}) SET c.value = 300").unwrap();
    assert!(
        second.is_success(),
        "repeat upsert must MATCH, not duplicate-CREATE, got: {:?}",
        second.error_message
    );

    let after = query_rows(&conn, "MATCH (c:Chain {id: 1}) RETURN c.value");
    assert_eq!(after, vec![vec!["Int64(300)".to_string()]]);
}

#[test]
fn merge_statement_level_param_roundtrip_create_then_match() {
    // Prepared statement-level MERGE with `$id`/`$v` params: first run creates,
    // second run must match the row the first run created.
    let (_db, conn) = setup_chain();
    let stmt = conn.prepare("MERGE (c:Chain {id: $id}) SET c.value = $v").unwrap();

    conn.execute(
        &stmt,
        vec![
            ("id", akar_common::types::Value::Int64(1)),
            ("v", akar_common::types::Value::Int64(500)),
        ],
    )
    .unwrap();
    let after_create = query_rows(&conn, "MATCH (c:Chain) RETURN c.id, c.value");
    assert_eq!(
        after_create,
        vec![vec!["Int64(1)".to_string(), "Int64(500)".to_string()]]
    );

    conn.execute(
        &stmt,
        vec![
            ("id", akar_common::types::Value::Int64(1)),
            ("v", akar_common::types::Value::Int64(600)),
        ],
    )
    .unwrap();
    let after_match = query_rows(&conn, "MATCH (c:Chain) RETURN c.id, c.value");
    assert_eq!(
        after_match,
        vec![vec!["Int64(1)".to_string(), "Int64(600)".to_string()]],
        "second param upsert must MATCH the existing row, got: {after_match:?}"
    );
}

#[test]
fn merge_statement_level_param_matches_literal_created_row() {
    // A row created via a literal CREATE must be updatable by a param MERGE —
    // proves substitution reaches the fast path and the existence check agrees.
    let (_db, conn) = setup_chain();
    exec(&conn, "CREATE (:Chain {id: 1, value: 100})");

    let stmt = conn.prepare("MERGE (c:Chain {id: $id}) SET c.value = $v").unwrap();
    conn.execute(
        &stmt,
        vec![
            ("id", akar_common::types::Value::Int64(1)),
            ("v", akar_common::types::Value::Int64(700)),
        ],
    )
    .unwrap();

    let after = query_rows(&conn, "MATCH (c:Chain {id: 1}) RETURN c.value");
    assert_eq!(
        after,
        vec![vec!["Int64(700)".to_string()]],
        "param MERGE must match a literal-created row, got: {after:?}"
    );
}

#[test]
fn merge_statement_level_param_pk_not_null() {
    // The F2 NULL-PK symptom: a `$param` in the pattern must evaluate to the
    // substituted constant, never `Value::Null` ("NULL value not allowed for
    // primary key").
    let (_db, conn) = setup_chain();
    let stmt = conn.prepare("MERGE (c:Chain {id: $id}) SET c.value = $v").unwrap();
    let res = conn.execute(
        &stmt,
        vec![
            ("id", akar_common::types::Value::Int64(1)),
            ("v", akar_common::types::Value::Int64(800)),
        ],
    );
    assert!(
        res.is_ok(),
        "param PK must not collapse to NULL, got error: {:?}",
        res.err()
    );

    let rows = query_rows(&conn, "MATCH (c:Chain {id: 1}) RETURN c.value");
    assert_eq!(rows, vec![vec!["Int64(800)".to_string()]]);
}
