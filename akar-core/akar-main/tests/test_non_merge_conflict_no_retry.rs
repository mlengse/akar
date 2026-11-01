//! Contract B4 / FR-004b: non-MERGE writes keep reporting OCC conflicts.
//!
//! The retry predicate (`connection::retry::should_retry_statement`) is proven
//! false for non-MERGE statements by unit tests inside the crate. This file
//! checks the observable half of the contract through the public API: under
//! real contention, `SET` writes surface their conflict instead of being
//! retried or masked.

mod common;

use common::*;
use std::sync::{Arc, Barrier};

const WORKERS: usize = 8;
const WRITES_PER_WORKER: usize = 50;
const KEY: i64 = 1;

#[test]
fn non_merge_writes_report_conflicts() {
    let (db, conn) = setup_db();
    conn.query("CREATE NODE TABLE Memory (id INT64, content STRING, PRIMARY KEY (id))")
        .expect("create Memory table");
    conn.query(&format!("CREATE (:Memory {{id: {KEY}, content: 'seed'}})"))
        .expect("seed row");

    let barrier = Arc::new(Barrier::new(WORKERS));
    let handles: Vec<_> = (0..WORKERS)
        .map(|worker| {
            let database = Arc::clone(&db);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let connection = Connection::new(&database);
                barrier.wait();
                let mut errors = Vec::new();
                for _ in 0..WRITES_PER_WORKER {
                    let query = format!("MATCH (m:Memory {{id: {KEY}}}) SET m.content = 'w{worker}'");
                    if let Err(error) = connection.query(&query) {
                        errors.push(error);
                    }
                }
                errors
            })
        })
        .collect();

    let mut errors = Vec::new();
    for handle in handles {
        errors.extend(handle.join().expect("worker thread"));
    }

    assert!(
        !errors.is_empty(),
        "with {WORKERS} writers on one row, some non-MERGE write must surface its conflict"
    );
    for error in &errors {
        assert!(
            error.starts_with("Transaction commit failed: write conflict on table "),
            "only genuine commit conflicts are expected here: {error}"
        );
    }
    assert_eq!(
        query_rows(&conn, &format!("MATCH (m:Memory {{id: {KEY}}}) RETURN m.id")).len(),
        1,
        "a rejected write must not duplicate the row"
    );
}
