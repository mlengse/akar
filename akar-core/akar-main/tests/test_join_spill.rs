//! P111 — the external spill hash join, exercised end to end through Cypher.
//!
//! The algorithm's own unit tests live in
//! `akar-processor/src/physical/join_spill.rs` and compare a spilled join against
//! the in-memory join on the same data. These tests prove the wiring: that a real
//! query spills when its budget is tight, that it still returns the correct
//! answer when it does, and that EXPLAIN reports the spill path.

mod common;
use common::*;

/// Build a database whose memory budget is small enough that a sizeable join
/// cannot hold its build side in memory, with the budget tight enough to matter
/// but not so tight that unrelated machinery misbehaves.
fn setup_tight_memory_db(max_db_size: u64) -> (std::sync::Arc<Database>, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_db");
    let config = SystemConfig {
        max_db_size,
        ..SystemConfig::default()
    };
    let database = std::sync::Arc::new(Database::new(db_path, config).unwrap());
    let conn = Connection::new(&database);
    std::mem::forget(dir);
    (database, conn)
}

/// Two node tables joined on `id`, sized so the build side is comfortably larger
/// than the grants used below.
fn build_join_fixture(conn: &Connection, rows: i64) {
    exec(conn, "CREATE NODE TABLE LB(id INT64, v INT64, PRIMARY KEY (id))");
    exec(conn, "CREATE NODE TABLE LP(id INT64, w INT64, PRIMARY KEY (id))");
    for i in 0..rows {
        exec(conn, &format!("CREATE (:LB {{id: {i}, v: {}}})", i * 3));
        exec(conn, &format!("CREATE (:LP {{id: {i}, w: {}}})", i + 1));
    }
}

/// The join query: `COUNT` plus a checksum, so a wrong or partial join cannot
/// pass by accident.
const JOIN_SQL: &str = "MATCH (b:LB), (p:LP) WHERE b.id = p.id \
     RETURN COUNT(b.id) AS n, SUM(b.v + p.w) AS total";

/// Sum of `(3*i) + (i+1)` for `i` in `0..rows`.
fn expected_totals(rows: i64) -> (i64, i64) {
    let n = rows;
    let total: i64 = (0..rows).map(|i| 3 * i + i + 1).sum();
    (n, total)
}

#[test]
fn hash_join_is_correct_with_a_roomy_budget() {
    let (db, conn) = setup_tight_memory_db(u32::MAX as u64);
    build_join_fixture(&conn, 200);

    let (n, total) = expected_totals(200);
    let rows = query_rows(&conn, JOIN_SQL);
    assert_eq!(
        rows,
        vec![vec![format!("Int64({n})"), format!("Int64({total})")]],
        "in-memory join result changed"
    );
    assert_eq!(
        db.memory_governor().spill_events(),
        0,
        "a join that fits must not spill"
    );
}

#[test]
fn hash_join_spills_under_a_tight_budget_and_still_returns_every_row() {
    // 16 KiB of budget for a build side that needs more than that.
    let (db, conn) = setup_tight_memory_db(16 * 1024);
    build_join_fixture(&conn, 150);

    let (n, total) = expected_totals(150);
    let rows = query_rows(&conn, JOIN_SQL);
    assert_eq!(
        rows,
        vec![vec![format!("Int64({n})"), format!("Int64({total})")]],
        "the spilled join must return exactly the same aggregate as the in-memory join"
    );
    assert!(
        db.memory_governor().spill_events() > 0,
        "with a {} byte budget the join must have spilled",
        16 * 1024
    );
}

#[test]
fn explain_reports_the_spill_path_available_to_a_join() {
    let (_db, conn) = setup_tight_memory_db(u32::MAX as u64);
    build_join_fixture(&conn, 3);

    // A Connection always binds a memory pool and a spill directory, so a hash
    // join declares the spill path available to it.
    let rows = query_rows(
        &conn,
        "EXPLAIN MATCH (b:LB), (p:LP) WHERE b.id = p.id RETURN b.id AS id",
    );
    let plan = rows.first().and_then(|r| r.first()).cloned().unwrap_or_default();
    assert!(plan.contains("HashJoin("), "expected a hash join in the plan:\n{plan}");
    assert!(
        plan.contains("[Spill="),
        "EXPLAIN must report the spill path available to the join:\n{plan}"
    );
}

/// The marker is a property of the processor, not of the plan: a processor built
/// without a governor renders the join with no spill marker at all, which is why
/// adding it changed no pre-existing EXPLAIN expectation.
#[test]
fn explain_without_a_memory_pool_omits_the_spill_marker() {
    use akar_planner::logical_operator::{LogicalCrossProduct, LogicalFlatten, LogicalHashJoin, LogicalOperator};

    let leaf = || {
        LogicalOperator::Flatten(LogicalFlatten {
            group_pos: 0,
            children: Vec::new(),
            cardinality: 0,
        })
    };
    let op = LogicalOperator::HashJoin(LogicalHashJoin {
        join_keys: Vec::new(),
        build_side: Box::new(LogicalOperator::CrossProduct(LogicalCrossProduct {
            left: Box::new(leaf()),
            right: Box::new(leaf()),
            cardinality: 0,
        })),
        probe_side: Box::new(leaf()),
        cardinality: 1,
        push_down_eligible: false,
    });

    let with_path = akar_processor::processor::plan_serializer::serialize_plan_tree(&op, 0, 256);
    assert!(with_path.contains("HashJoin(0 keys) [Spill=256]"), "{with_path}");

    let without = akar_processor::processor::plan_serializer::serialize_plan_tree(&op, 0, 0);
    assert!(without.contains("HashJoin(0 keys)"), "{without}");
    assert!(!without.contains("Spill="), "no pool means no spill path:\n{without}");
}
