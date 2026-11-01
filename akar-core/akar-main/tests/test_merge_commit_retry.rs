//! Contract B2/B6 + SC-002: an auto-commit MERGE absorbs a transient OCC write
//! conflict with no retry logic in the caller.
//!
//! The deterministic persistent-contention case (contract B3) needs a writer
//! that stays active across commits. Holding one open requires the crate-private
//! `Database::transaction_manager`, so that case is covered next to the retry
//! loop in `akar-main/src/connection/retry.rs` (`conflict_behavior::*`). This
//! file covers the same contract through the public API under real contention.

mod common;

use common::*;
use std::sync::Arc;
use std::time::{Duration, Instant};

const WORKERS: usize = 8;
const ROUNDS: usize = 5;
const KEY: i64 = 777;

fn setup() -> (Arc<Database>, Connection) {
    let (db, conn) = setup_db();
    conn.query("CREATE NODE TABLE Memory (id INT64, content STRING, PRIMARY KEY (id))")
        .expect("create Memory table");
    (db, conn)
}

fn count_rows(conn: &Connection) -> usize {
    query_rows(conn, &format!("MATCH (m:Memory {{id: {KEY}}}) RETURN m.id")).len()
}

/// B2/B6/SC-002: eight clients MERGE the same key at the same instant, round
/// after round, with no application-level retry. Every statement must succeed
/// inside the engine and exactly one node may remain.
///
/// Each round is the original flake shape (eight clients, one MERGE apiece).
/// Sustained back-to-back contention beyond the retry budget is contract B3 —
/// it fails loud by design and is covered deterministically in
/// `connection::retry::conflict_behavior`.
#[test]
fn merge_transient_conflict_resolves() {
    let (db, conn) = setup();
    let started = Instant::now();
    let mut failures = Vec::new();

    for round in 0..ROUNDS {
        let barrier = Arc::new(std::sync::Barrier::new(WORKERS));
        let handles: Vec<_> = (0..WORKERS)
            .map(|worker| {
                let database = Arc::clone(&db);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let connection = Connection::new(&database);
                    barrier.wait();
                    let query = format!("MERGE (m:Memory {{id: {KEY}}}) SET m.content = 'w{worker}-{round}'");
                    connection.query(&query)
                })
            })
            .collect();

        for handle in handles {
            if let Err(error) = handle.join().expect("worker thread") {
                failures.push(error);
            }
        }
    }

    let elapsed = started.elapsed();

    assert!(
        failures.is_empty(),
        "the engine must absorb transient conflicts without caller retries: {failures:?}"
    );
    assert!(elapsed < Duration::from_secs(2), "SC-002 window exceeded: {elapsed:?}");
    assert_eq!(count_rows(&conn), 1, "concurrent MERGE must leave exactly one node");
}

/// B6: MERGE on a key that does not exist yet is match-or-create even when all
/// clients race to create it — no duplicate rows, no lost insert.
#[test]
fn concurrent_merge_creates_one_row() {
    let (db, conn) = setup();
    const FRESH_KEY: i64 = 4242;

    let handles: Vec<_> = (0..WORKERS)
        .map(|worker| {
            let database = Arc::clone(&db);
            std::thread::spawn(move || {
                let connection = Connection::new(&database);
                let query = format!("MERGE (m:Memory {{id: {FRESH_KEY}}}) SET m.content = 'fresh-{worker}'");
                connection.query(&query)
            })
        })
        .collect();

    for handle in handles {
        let result = handle.join().expect("worker thread");
        assert!(result.is_ok(), "MERGE should succeed under contention: {result:?}");
    }

    assert_eq!(
        query_rows(&conn, &format!("MATCH (m:Memory {{id: {FRESH_KEY}}}) RETURN m.id")).len(),
        1,
        "the racing MERGEs must converge on one row"
    );
}
