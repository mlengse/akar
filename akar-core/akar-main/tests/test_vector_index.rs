//! Regression tests for the vector index path (requires `vector-extension`).
//!
//! The gate `test [akar-core]` runs without features, so this file is compiled
//! out there; run it explicitly with `cargo test --features vector-extension`.
//!
//! P53.x: `bind_create_vector_index` used to hold the catalog `MutexGuard`
//! across a second `self.catalog.lock()`, self-deadlocking on the same thread
//! (a `WaitOnAddress` hang) for any valid metric. The smoke test surfaced it
//! as a hang on `CALL CREATE_VECTOR_INDEX`. Fixed by scoping the first guard.

#![cfg(feature = "vector-extension")]

use akar_main::{Connection, Database, SystemConfig};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;

fn setup() -> (tempfile::TempDir, Arc<Database>, Connection) {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::new(dir.path().to_str().unwrap(), SystemConfig::default()).unwrap());
    let conn = Connection::new(&db);
    conn.query("CREATE NODE TABLE Memory (id INT64, content STRING, embedding FLOAT[], PRIMARY KEY (id))")
        .expect("create Memory table");
    (dir, db, conn)
}

/// A second `CREATE VECTOR INDEX` with the same name must fail fast with
/// "already exists" — never hang. Before the binder fix this deadlocked on the
/// catalog mutex (re-entrant guard in `bind_create_vector_index`). The
/// watchdog converts a deadlock into a test failure instead of a hang.
#[test]
fn create_vector_index_twice_no_deadlock() {
    let (_dir, _db, conn) = setup();

    conn.query("CREATE VECTOR INDEX mem_vec ON (Memory.embedding) WITH (metric=cosine, dims=384)")
        .expect("first create should succeed");

    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let r = conn.query("CREATE VECTOR INDEX mem_vec ON (Memory.embedding) WITH (metric=cosine, dims=384)");
        let _ = tx.send(r);
    });

    let result = match rx.recv_timeout(Duration::from_secs(15)) {
        Ok(r) => r,
        Err(_) => {
            panic!("second CREATE VECTOR INDEX deadlocked on the catalog mutex");
        }
    };
    handle.join().expect("worker thread panicked");

    match result {
        Err(e) => assert!(
            e.contains("already exists"),
            "expected 'already exists', got: {e}"
        ),
        Ok(_) => panic!("second CREATE VECTOR INDEX should have errored with 'already exists'"),
    }
}

/// Different index name on the same column/table is allowed, and the table can
/// still be queried (scalar path) after vector indexes exist.
#[test]
fn vector_index_does_not_break_scalar_queries() {
    let (_dir, _db, conn) = setup();

    conn.query("CREATE VECTOR INDEX mem_vec ON (Memory.embedding) WITH (metric=cosine, dims=384)")
        .expect("first create");
    conn.query("CREATE VECTOR INDEX mem_vec_2 ON (Memory.embedding) WITH (metric=cosine, dims=384)")
        .expect("second create on same column");

    conn.query("CREATE (m:Memory {id: 1, content: 'hello', embedding: [0.1, 0.2]})")
        .expect("insert row");

    let res = conn.query("MATCH (m:Memory) RETURN m.id, m.content").expect("scalar query");
    let chunk = res.chunks.first().expect("one chunk");
    assert_eq!(chunk.size, 1, "one row expected");
}
