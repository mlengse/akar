//! P48.4 — WCOJ vs HashJoin benchmark (small design, runnable via `cargo test`).
//!
//! Criterion's full harness hangs on this machine (Decision #62), so the P46.5
//! benchmark re-open asserts correctness and reports wall-clock timing here.
//! The same queries live in `benches/ladybug_suite.rs` (`bench_wcoj`,
//! `bench_wcoj_triangle`) for CI / healthy machines.

mod common;

use common::*;
use std::time::Instant;

/// Fan DB: Person 151 / Tag 101. Star (WCOJ `Intersect`) and chain (HashJoin)
/// both return exactly 10,000 rows.
fn build_fan_db() -> (std::sync::Arc<Database>, Connection) {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY(id))");
    exec(&conn, "CREATE NODE TABLE Tag(id INT64, PRIMARY KEY(id))");
    exec(&conn, "CREATE REL TABLE r1(FROM Person TO Person)");
    exec(&conn, "CREATE REL TABLE r2(FROM Person TO Tag)");
    exec(&conn, "CREATE REL TABLE r3t(FROM Person TO Tag)");

    for i in 0..151 {
        exec(&conn, &format!("CREATE (p:Person {{id: {i}}})"));
    }
    for i in 0..101 {
        exec(&conn, &format!("CREATE (t:Tag {{id: {i}}})"));
    }

    let t0 = Instant::now();
    // r1: centers 0..99, each -> 10 Persons (a, a+10]. 1000 edges total.
    // P48.3 variable-comparison shape (node predicates are ignored in CREATE — BUG-A).
    for a in 0..100 {
        exec(
            &conn,
            &format!(
                "MATCH (a:Person), (b:Person) WHERE a.id >= {a} AND a.id <= {a} AND b.id > a.id AND b.id <= a.id + 10 CREATE (a)-[:r1]->(b)"
            ),
        );
    }
    // r2: centers 0..99, each -> 10 Tags [0,9]. 1000 edges total.
    exec(
        &conn,
        "MATCH (a:Person), (t:Tag) WHERE a.id >= 0 AND a.id <= 99 AND t.id >= 0 AND t.id <= 9 CREATE (a)-[:r2]->(t)",
    );
    // r3t: every Person -> 10 Tags [0,9]. 1510 edges total.
    exec(
        &conn,
        "MATCH (b:Person), (t:Tag) WHERE b.id >= 0 AND b.id <= 150 AND t.id >= 0 AND t.id <= 9 CREATE (b)-[:r3t]->(t)",
    );
    let setup = t0.elapsed();
    println!("fan DB edge setup: {setup:?}");
    (_db, conn)
}

/// Triangle DB: N=41, edges forward only (`b.id > a.id`). Triangle query must
/// return exactly C(41,3) = 10,660 rows.
fn build_triangle_db() -> (std::sync::Arc<Database>, Connection) {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY(id))");
    exec(&conn, "CREATE REL TABLE r1(FROM Person TO Person)");
    exec(&conn, "CREATE REL TABLE r2(FROM Person TO Person)");
    exec(&conn, "CREATE REL TABLE r3(FROM Person TO Person)");

    for i in 0..41 {
        exec(&conn, &format!("CREATE (p:Person {{id: {i}}})"));
    }

    let t0 = Instant::now();
    for (rel, a_lo, a_hi) in [
        ("r1", 0, 20),
        ("r1", 21, 40),
        ("r2", 0, 20),
        ("r2", 21, 40),
        ("r3", 0, 20),
        ("r3", 21, 40),
    ] {
        exec(
            &conn,
            &format!(
                "MATCH (a:Person), (b:Person) WHERE a.id >= {a_lo} AND a.id <= {a_hi} AND b.id > a.id CREATE (a)-[:{rel}]->(b)"
            ),
        );
    }
    let setup = t0.elapsed();
    println!("triangle DB edge setup: {setup:?}");
    (_db, conn)
}

#[test]
fn test_wcoj_star_vs_hashjoin() {
    let (_db, conn) = build_fan_db();

    let star_sql = "MATCH (a:Person)-[:r1]->(b:Person), (a:Person)-[:r2]->(t:Tag) RETURN a.id, b.id, t.id";
    let chain_sql = "MATCH (a:Person)-[:r1]->(b:Person), (b:Person)-[:r3t]->(t:Tag) RETURN a.id, b.id, t.id";

    let rows = query_rows(&conn, star_sql);
    assert_eq!(rows.len(), 10_000, "WCOJ star: expected 10,000 rows");
    let rows = query_rows(&conn, chain_sql);
    assert_eq!(rows.len(), 10_000, "HashJoin chain: expected 10,000 rows");

    // EXPLAIN must show Intersect for the star, no Intersect for the chain.
    let star_plan = explain(&conn, star_sql);
    assert!(star_plan.contains("Intersect"), "star plan missing Intersect:\n{star_plan}");
    let chain_plan = explain(&conn, chain_sql);
    assert!(
        !chain_plan.contains("Intersect"),
        "chain plan unexpectedly uses Intersect:\n{chain_plan}"
    );

    // Wall-clock timing (informational, not asserted).
    let mut total = 0;
    let t = Instant::now();
    for _ in 0..5 {
        let r = conn.query(star_sql).unwrap();
        total += r.num_rows;
    }
    println!("star (WCOJ Intersect) 5x: {:?} ({total} rows)", t.elapsed());

    let mut total = 0;
    let t = Instant::now();
    for _ in 0..5 {
        let r = conn.query(chain_sql).unwrap();
        total += r.num_rows;
    }
    println!("chain (HashJoin) 5x: {:?} ({total} rows)", t.elapsed());
}

#[test]
fn test_wcoj_triangle_count() {
    let (_db, conn) = build_triangle_db();

    let triangle_sql = "MATCH (a:Person)-[:r1]->(b:Person), (a:Person)-[:r2]->(c:Person), (b:Person)-[:r3]->(c:Person) RETURN a.id, b.id, c.id";
    let rows = query_rows(&conn, triangle_sql);
    assert_eq!(rows.len(), 10_660, "WCOJ triangle: expected C(41,3) = 10,660 rows");

    let plan = explain(&conn, triangle_sql);
    assert!(plan.contains("Intersect"), "triangle plan missing Intersect:\n{plan}");

    let t = Instant::now();
    let r = conn.query(triangle_sql).unwrap();
    println!("triangle (WCOJ Intersect): {:?} ({} rows)", t.elapsed(), r.num_rows);
}

/// Extract the EXPLAIN plan text for a query.
fn explain(conn: &Connection, sql: &str) -> String {
    let result = conn.query(&format!("EXPLAIN {sql}")).unwrap();
    assert!(result.is_success(), "EXPLAIN failed: {:?}", result.error_message);
    let mut plan = String::new();
    for chunk in &result.chunks {
        if chunk.size > 0 {
            if let Some(Value::String(s)) = chunk.get_value(0, 0) {
                plan = s;
            }
        }
    }
    plan
}
