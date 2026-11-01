//! Contract B5 / FR-004a: a MERGE issued inside an explicit transaction never
//! retries.
//!
//! The retry predicate (`connection::retry::should_retry_statement`) is proven
//! false while `explicit_txn_active` is set by unit tests inside the crate.
//! This file checks the observable half through the public API: an explicit
//! transaction reports its conflict to the client instead of hiding it behind
//! engine retries.

mod common;

use common::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const HAMMERS: usize = 8;
const ATTEMPTS: usize = 40;
const KEY: i64 = 888;

#[test]
fn merge_inside_explicit_transaction_is_not_retried() {
    let (db, conn) = setup_db();
    conn.query("CREATE NODE TABLE Memory (id INT64, content STRING, PRIMARY KEY (id))")
        .expect("create Memory table");
    conn.query(&format!("CREATE (:Memory {{id: {KEY}, content: 'seed'}})"))
        .expect("seed row");

    // Keep the row hot: these auto-commit MERGEs do retry, so overlap with the
    // explicit statement is frequent and the conflict window stays open.
    let stop = Arc::new(AtomicBool::new(false));
    let hammers: Vec<_> = (0..HAMMERS)
        .map(|worker| {
            let database = Arc::clone(&db);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let connection = Connection::new(&database);
                while !stop.load(Ordering::Acquire) {
                    let query = format!("MERGE (m:Memory {{id: {KEY}}}) SET m.content = 'h{worker}'");
                    let _ = connection.query(&query);
                }
            })
        })
        .collect();

    let mut conflicts = Vec::new();
    for _ in 0..ATTEMPTS {
        conn.query("BEGIN TRANSACTION").expect("begin explicit transaction");
        match conn.query(&format!("MERGE (m:Memory {{id: {KEY}}}) SET m.content = 'explicit'")) {
            Ok(_) => {}
            Err(error) => conflicts.push(error),
        }
        let _ = conn.query("ROLLBACK");
    }

    stop.store(true, Ordering::Release);
    for hammer in hammers {
        hammer.join().expect("hammer thread");
    }

    assert!(
        !conflicts.is_empty(),
        "an explicit MERGE under contention must report its conflict, not retry it"
    );
    for conflict in &conflicts {
        assert!(
            conflict.starts_with("Transaction commit failed: write conflict on table "),
            "unexpected error from the explicit transaction: {conflict}"
        );
    }
    assert_eq!(
        query_rows(&conn, &format!("MATCH (m:Memory {{id: {KEY}}}) RETURN m.id")).len(),
        1,
        "the explicit attempts must not duplicate the row"
    );
}
