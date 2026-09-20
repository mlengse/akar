/// P110.2/P110.3 — the memory governor's admission gate and active reclaim, as
/// seen from the `Connection` boundary.
///
/// The unit tests in `akar-common/src/query_pool.rs` pin the governor's own
/// behaviour. These tests pin the *wiring*: that every query asks the governor
/// for a pool before it runs, that the pool is released when the statement
/// finishes, and that a refused admission surfaces as a query error instead of
/// running the query without a memory bound.
///
/// The external spill join (P111) is covered by `test_join_spill.rs`.
mod common;
use akar_common::query_pool::{AdmissionRejection, GovernorPolicy};
use common::*;

fn oversized_floor_policy() -> GovernorPolicy {
    GovernorPolicy {
        // No instance can grant this much headroom, so every query is refused.
        admit_min_grant_bytes: u64::MAX,
        ..GovernorPolicy::default()
    }
}

#[test]
fn query_acquires_and_releases_a_memory_pool() {
    let (db, conn) = setup_db();
    assert_eq!(db.memory_governor().active_queries(), 0);

    exec(&conn, "CREATE NODE TABLE Gov(id INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (:Gov {id: 1})");
    let rows = query_rows(&conn, "MATCH (g:Gov) RETURN g.id AS id");
    assert_eq!(rows, vec![vec!["Int64(1)".to_string()]]);

    // The pool lives exactly as long as the statement: every query above has
    // returned, so no grant may still be outstanding.
    assert_eq!(
        db.memory_governor().active_queries(),
        0,
        "a finished query must release its memory pool"
    );
    assert_eq!(db.memory_governor().live_pools(), 0);
}

#[test]
fn rejected_admission_fails_the_query_instead_of_running_it_unbounded() {
    let (db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Gov2(id INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (:Gov2 {id: 7})");

    // Arm a policy that refuses everything, then confirm the query fails.
    db.set_admission_policy(oversized_floor_policy());
    let err = conn
        .query("MATCH (g:Gov2) RETURN g.id AS id")
        .expect_err("a refused admission must not run the query");
    assert!(
        err.contains("Query rejected"),
        "error should name the governor, got: {err}"
    );
    assert!(
        err.contains("minimum"),
        "error should explain the headroom shortfall, got: {err}"
    );
    assert_eq!(db.memory_governor().rejected_queries(), 1);
    assert_eq!(
        db.memory_governor().active_queries(),
        0,
        "a refused query must not hold a slot"
    );

    // Disarming the gate restores service — the rejection was policy, not damage.
    db.set_admission_policy(GovernorPolicy::default());
    let rows = query_rows(&conn, "MATCH (g:Gov2) RETURN g.id AS id");
    assert_eq!(rows, vec![vec!["Int64(7)".to_string()]]);
}

#[test]
fn default_policy_admits_queries_and_reports_pressure() {
    let (db, conn) = setup_db();
    assert_eq!(db.admission_policy(), GovernorPolicy::default());

    exec(&conn, "CREATE NODE TABLE Gov3(id INT64, PRIMARY KEY (id))");
    for id in 0..5 {
        exec(&conn, &format!("CREATE (:Gov3 {{id: {id}}})"));
    }
    assert_eq!(
        query_rows(&conn, "MATCH (g:Gov3) RETURN COUNT(g.id) AS n")[0][0],
        "Int64(5)"
    );

    // The gate is armed by default but only bites once the budget is spent, so a
    // healthy instance must never record a rejection.
    assert_eq!(db.memory_governor().rejected_queries(), 0);
}

#[test]
fn governor_policy_is_readable_and_writable_through_the_database() {
    let (db, _conn) = setup_db();
    let policy = GovernorPolicy {
        admit_max_pressure: 0.5,
        admit_max_concurrent_queries: 3,
        admit_min_grant_bytes: 4096,
        reclaim_pressure: 0.25,
    };
    db.set_admission_policy(policy);
    assert_eq!(db.admission_policy(), policy);
}

/// Admission is reported by reason, not just as a boolean: an embedder that
/// wants to retry needs to know *why* a query was refused.
#[test]
fn admission_rejection_reason_is_actionable() {
    let (db, conn) = setup_db();
    db.set_admission_policy(oversized_floor_policy());
    let err = conn.query("RETURN 1 AS one").expect_err("expected rejection");
    assert!(err.contains("could be granted"), "unhelpful reason: {err}");

    // Each rejection variant renders a distinct, human-readable reason.
    let concurrency = AdmissionRejection::TooManyConcurrentQueries {
        active: 4,
        max_concurrent: 4,
    };
    assert!(concurrency.reason().contains("limit 4"), "{}", concurrency.reason());

    let pressure = AdmissionRejection::MemoryPressure {
        pressure: 0.99,
        max_pressure: 0.95,
    };
    assert!(pressure.reason().contains("99%"), "{}", pressure.reason());
    assert!(pressure.reason().contains("95%"), "{}", pressure.reason());
}
