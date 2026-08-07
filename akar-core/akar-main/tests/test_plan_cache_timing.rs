//! Regression guard for the plan-cache hot path on a realistic 10K-row table.
//!
//! Cache hits must never be meaningfully slower than the full pipeline.
//! (On data-bound workloads execution dominates, so hits and misses converge;
//! the win is where planning dominates, e.g. complex plans on small data.)
use akar_common::types::Value;
use akar_main::{Connection, Database, SystemConfig};
use std::sync::Arc;
use std::time::Instant;

fn build_db() -> (Arc<Database>, Connection) {
    let db = Arc::new(Database::new(":memory:", SystemConfig::default()).unwrap());
    let conn = Connection::new(&db);
    conn.query("CREATE NODE TABLE Person(name STRING, age INT64, score DOUBLE, PRIMARY KEY (name))")
        .unwrap();
    {
        let catalog = db.table_catalog();
        let mut table = catalog.get_node_table_by_name_mut("Person").unwrap();
        for i in 0..10_000u64 {
            table
                .insert_row(vec![
                    Value::String(format!("Person{i}")),
                    Value::Int64((i as i64 * 7 + 13) % 101),
                    Value::Double(i as f64 * 0.001 + 50.0),
                ])
                .unwrap();
        }
    }
    (db, conn)
}

#[test]
fn test_plan_cache_no_hit_regression() {
    let (_db, conn) = build_db();

    // Warm up: first call populates the cache
    let r = conn.query("MATCH (p:Person) WHERE p.age > 50 RETURN COUNT(p)").unwrap();
    assert!(r.is_success());

    const N: u32 = 1000;

    // Cache-hit latency: repeated identical query strings
    let start = Instant::now();
    for _ in 0..N {
        let r = conn.query("MATCH (p:Person) WHERE p.age > 50 RETURN COUNT(p)").unwrap();
        assert!(r.is_success());
    }
    let hit = start.elapsed();
    eprintln!("{N} cache-HIT queries: {:?} ({:?}/query)", hit, hit / N);

    // Cache-miss latency: unique query strings (full parse/bind/plan/optimize).
    // 1000 distinct keys exceed the LRU capacity (100), so eviction keeps
    // these as genuine misses throughout.
    let start = Instant::now();
    for i in 0..N {
        let q = format!("MATCH (p:Person) WHERE p.age > {} RETURN COUNT(p)", i % 1000);
        let r = conn.query(&q).unwrap();
        assert!(r.is_success());
    }
    let miss = start.elapsed();
    eprintln!("{N} cache-MISS queries: {:?} ({:?}/query)", miss, miss / N);

    // Guard: hits must not be meaningfully slower than misses (>50% worse).
    let hit_ns = hit.as_nanos() as f64;
    let miss_ns = miss.as_nanos() as f64;
    eprintln!("hit/miss ratio: {:.3}", hit_ns / miss_ns);
    assert!(hit_ns <= miss_ns * 1.5, "Cache hits must not be 50% slower than misses");
}
