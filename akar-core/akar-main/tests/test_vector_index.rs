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

use akar_common::types::Value;
use akar_main::{Connection, Database, SystemConfig};
use std::sync::Arc;
use std::sync::mpsc;
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
        Err(e) => assert!(e.contains("already exists"), "expected 'already exists', got: {e}"),
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

    let res = conn
        .query("MATCH (m:Memory) RETURN m.id, m.content")
        .expect("scalar query");
    let chunk = res.chunks.first().expect("one chunk");
    assert_eq!(chunk.size, 1, "one row expected");
}

/// P53.12: `RETURN n.embedding` on a `FLOAT[]` column must yield a `Value::List`
/// instead of NULL. Previously the scan collapsed List/Array columns to Int64
/// and the Arrow builders emitted all-null arrays.
#[test]
fn complex_type_list_column_round_trips() {
    let (_dir, _db, conn) = setup();
    conn.query("CREATE (m:Memory {id: 1, content: 'hello', embedding: [0.1, 0.2, 0.3]})")
        .expect("insert row");

    let res = conn.query("MATCH (m:Memory) RETURN m.embedding").expect("list query");
    let chunk = res.chunks.first().expect("one chunk");
    assert_eq!(chunk.size, 1, "one row expected");

    let val = chunk.get_value(0, 0).expect("embedding must not be null");
    match val {
        Value::List(items) => {
            assert_eq!(items.len(), 3, "embedding has 3 elements, got {items:?}");
            for item in &items {
                assert!(
                    matches!(item, Value::Float(_) | Value::Double(_)),
                    "expected numeric embedding element, got {item:?}"
                );
            }
        }
        other => panic!("expected Value::List, got {other:?}"),
    }
}

/// P53.12: `RETURN {id: n.id}` (a map literal) must yield a `Value::Struct`
/// instead of NULL. The map-literal projection now goes through
/// `evaluate_arrow` → `arrow_array_from_values` (StructArray), bypassing the
/// ValueVector that had no side-storage.
#[test]
fn complex_type_map_literal_returns_struct() {
    let (_dir, _db, conn) = setup();
    conn.query("CREATE (m:Memory {id: 1, content: 'hello', embedding: [0.1, 0.2]})")
        .expect("insert row");

    let res = conn
        .query("MATCH (m:Memory) RETURN {id: m.id}")
        .expect("map literal query");
    let chunk = res.chunks.first().expect("one chunk");
    assert_eq!(chunk.size, 1, "one row expected");

    let val = chunk.get_value(0, 0).expect("map literal must not be null");
    match val {
        Value::Struct(entries) => {
            assert_eq!(entries.len(), 1, "one field, got {entries:?}");
            assert_eq!(entries[0].0, "id");
            assert_eq!(entries[0].1, Value::Int64(1));
        }
        other => panic!("expected Value::Struct, got {other:?}"),
    }
}

/// P53.12: `array_cosine_similarity` over a `FLOAT[]` column + list literal
/// must return a Double. Both arguments now resolve through the Arrow-native
/// path (ListArray) so the scalar function sees real list values.
#[test]
fn complex_type_array_cosine_similarity_returns_double() {
    let (_dir, _db, conn) = setup();
    conn.query("CREATE (m:Memory {id: 1, content: 'hello', embedding: [1.0, 0.0]})")
        .expect("insert row");

    let res = conn
        .query("MATCH (m:Memory) RETURN array_cosine_similarity(m.embedding, [1.0, 0.0])")
        .expect("cosine query");
    let chunk = res.chunks.first().expect("one chunk");
    assert_eq!(chunk.size, 1, "one row expected");

    let val = chunk.get_value(0, 0).expect("cosine similarity must not be null");
    match val {
        Value::Double(d) => assert!((d - 1.0).abs() < 1e-9, "expected ~1.0, got {d}"),
        other => panic!("expected Value::Double, got {other:?}"),
    }
}

/// P71.1: `CALL vector_similarity_scan('Table','col',[q], k)` must actually run
/// the HNSW ANN scan and return the k nearest rows plus a distance column,
/// instead of erroring in the table-function registry (which rejects
/// `TableFunction::Custom`). The call is routed in
/// `DbStandaloneCallHandler::execute_vector_similarity_scan_call` directly to a
/// `PhysicalVectorSimilarityScan`.
#[test]
fn call_vector_similarity_scan_runs_hnsw() {
    let (_dir, _db, conn) = setup();

    conn.query("CREATE VECTOR INDEX mem_vec ON (Memory.embedding) WITH (metric=cosine, dims=2)")
        .expect("create vector index");

    conn.query("CREATE (m:Memory {id: 1, content: 'one', embedding: [1.0, 0.0]})")
        .expect("insert one");
    conn.query("CREATE (m:Memory {id: 2, content: 'two', embedding: [0.0, 1.0]})")
        .expect("insert two");

    // dims=2 vectors and k=2: both rows are candidates. Query [0.9, 0.1] is
    // far closer to [1,0] (id 1) than to [0,1] (id 2).
    let res = conn
        .query("CALL vector_similarity_scan('Memory', 'embedding', [0.9, 0.1], 2)")
        .expect("vector_similarity_scan must execute (previously errored)");

    let chunk = res.chunks.first().expect("one chunk");
    assert_eq!(chunk.size, 2, "expected 2 nearest rows, got {}", chunk.size);

    // Output layout: all table columns [id, content, embedding] + distance in
    // the final column (Memory has 3 columns, so distance is column index 3).
    let distance_col = 3;
    let mut ids = vec![];
    for row in 0..chunk.size {
        let id = chunk.get_value(0, row).expect("id column present");
        ids.push(id.clone());
        let dist = chunk.get_value(distance_col, row).expect("distance column present");
        match dist {
            Value::Double(_) => {}
            other => panic!("distance must be a Double, got {other:?}"),
        }
    }

    let mut got: Vec<i64> = ids
        .into_iter()
        .map(|v| match v {
            Value::Int64(i) => i,
            other => panic!("id must be Int64, got {other:?}"),
        })
        .collect();
    got.sort_unstable();
    assert_eq!(got, vec![1, 2], "both inserted rows should be returned");
}
