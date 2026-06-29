//! Query pipeline benchmarks.
//!
//! Measures end-to-end throughput: parse → bind → plan → optimize → execute
//! for representative Cypher queries.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use kuzu_main::connection::Connection;
use kuzu_main::database::{Database, SystemConfig};
use std::sync::Arc;
use tempfile::tempdir;

/// Set up a database with a Person table and sample data.
fn setup_db() -> (tempfile::TempDir, Arc<Database>, Connection) {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("bench_db");
    let config = SystemConfig::default();
    let database = Arc::new(Database::new(db_path, config).unwrap());
    let conn = Connection::new(&database);

    // Create schema
    conn.query("CREATE NODE TABLE Person(name STRING, age INT64, score DOUBLE, active BOOL, PRIMARY KEY (name))")
        .unwrap();

    // Insert some data via COPY FROM
    let csv_path = dir.path().join("people.csv");
    std::fs::write(&csv_path,
        "name,age,score,active\nAlice,30,95.5,true\nBob,25,87.3,false\nCharlie,35,91.2,true\nDavid,28,88.0,true\nEve,22,76.5,false\n"
    ).unwrap();
    let fp = csv_path.to_string_lossy().replace('\\', "/");
    conn.query(&format!("COPY Person FROM '{fp}' (HEADER true)")).unwrap();

    (dir, database, conn)
}

fn bench_query_match_return(c: &mut Criterion) {
    let (_dir, _db, conn) = setup_db();
    c.bench_function("query/match_return_all", |b| {
        b.iter(|| {
            let result = conn.query(black_box("MATCH (n:Person) RETURN n.name, n.age, n.score"));
            black_box(result.unwrap());
        })
    });
}

fn bench_query_match_filter(c: &mut Criterion) {
    let (_dir, _db, conn) = setup_db();
    c.bench_function("query/match_filter", |b| {
        b.iter(|| {
            let result = conn.query(black_box("MATCH (n:Person) WHERE n.age > 25 RETURN n.name"));
            black_box(result.unwrap());
        })
    });
}

fn bench_query_match_order(c: &mut Criterion) {
    let (_dir, _db, conn) = setup_db();
    c.bench_function("query/match_order_by", |b| {
        b.iter(|| {
            let result = conn.query(black_box("MATCH (n:Person) RETURN n.name ORDER BY n.age"));
            black_box(result.unwrap());
        })
    });
}

fn bench_query_match_limit(c: &mut Criterion) {
    let (_dir, _db, conn) = setup_db();
    c.bench_function("query/match_limit", |b| {
        b.iter(|| {
            let result = conn.query(black_box("MATCH (n:Person) RETURN n.name LIMIT 3"));
            black_box(result.unwrap());
        })
    });
}

fn bench_query_create_node_table(c: &mut Criterion) {
    let (dir, _db, conn) = setup_db();
    let _ = &dir;
    c.bench_function("query/create_node_table", |b| {
        b.iter(|| {
            let result = conn.query(black_box(
                "CREATE NODE TABLE City(name STRING, population INT64, PRIMARY KEY (name))",
            ));
            black_box(result.unwrap());
        })
    });
}

fn bench_query_copy_csv(c: &mut Criterion) {
    let (dir, _db, conn) = setup_db();
    conn.query("CREATE NODE TABLE T(name STRING, val INT64, PRIMARY KEY (name))")
        .unwrap();
    let csv_path = dir.path().join("copy_bench.csv");
    std::fs::write(&csv_path, "name,val\nx,1\ny,2\nz,3\n").unwrap();
    let fp = csv_path.to_string_lossy().replace('\\', "/");

    c.bench_function("query/copy_csv", |b| {
        b.iter(|| {
            let result = conn.query(black_box(&format!("COPY T FROM '{fp}' (HEADER true)")));
            black_box(result.unwrap());
        })
    });
}

criterion_group!(
    benches,
    bench_query_match_return,
    bench_query_match_filter,
    bench_query_match_order,
    bench_query_match_limit,
    bench_query_create_node_table,
    bench_query_copy_csv,
);
criterion_main!(benches);
