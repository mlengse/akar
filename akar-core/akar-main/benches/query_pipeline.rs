//! Query pipeline benchmarks.
//!
//! Measures end-to-end throughput: parse → bind → plan → optimize → execute
//! for representative Cypher queries.

use akar_main::connection::Connection;
use akar_main::database::{Database, SystemConfig};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
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

/// Generate a Person CSV with 10k rows (matching C++ benchmark dataset).
fn generate_10k_person_csv(dir: &tempfile::TempDir) -> String {
    let csv_path = dir.path().join("person_10k.csv");
    use std::io::Write;
    let mut f = std::fs::File::create(&csv_path).unwrap();
    writeln!(f, "ID,age,name").unwrap();
    for i in 0..10_000u64 {
        let age = (i * 7 + 13) % 101; // deterministic 0-100 distribution
        writeln!(f, "{i},{age},Person_{i}").unwrap();
    }
    f.flush().unwrap();
    csv_path.to_string_lossy().replace('\\', "/")
}

/// Set up database with 10k Person rows (matches C++ bench10k dataset).
fn setup_10k_db() -> (tempfile::TempDir, Arc<Database>, Connection) {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("bench_10k");
    let config = SystemConfig::default();
    let database = Arc::new(Database::new(db_path, config).unwrap());
    let conn = Connection::new(&database);

    conn.query("CREATE NODE TABLE Person(ID INT64, age INT64, name STRING, PRIMARY KEY (ID))")
        .unwrap();
    let fp = generate_10k_person_csv(&dir);
    conn.query(&format!("COPY Person FROM '{fp}' (HEADER true)")).unwrap();

    (dir, database, conn)
}

/// Apples-to-apples comparison with C++ akar_benchmark.
///
/// C++ measures: plan → optimize → execute (after one-time parse+bind).
/// Rust measures: `conn.execute()` after one-time `conn.prepare()`.
///
/// Query: `MATCH (p:Person) WHERE p.age > 30 RETURN COUNT(p)`
fn bench_query_filter_count_10k_vs_cpp(c: &mut Criterion) {
    let (_dir, _db, conn) = setup_10k_db();
    let prepared = conn
        .prepare("MATCH (p:Person) WHERE p.age > 30 RETURN COUNT(p)")
        .unwrap();

    let mut group = c.benchmark_group("e2e_vs_cpp");
    group.throughput(criterion::Throughput::Elements(10_000));
    group.bench_with_input(
        BenchmarkId::new("filter_count_10k", "execute_only"),
        &prepared,
        |b, p| {
            b.iter(|| {
                let result = conn.execute(p, vec![]);
                black_box(result.unwrap());
            })
        },
    );
    group.finish();
}

/// End-to-end benchmark (prepare + execute) for reference.
fn bench_query_filter_count_10k_e2e(c: &mut Criterion) {
    let (_dir, _db, conn) = setup_10k_db();

    let mut group = c.benchmark_group("e2e_vs_cpp");
    group.throughput(criterion::Throughput::Elements(10_000));
    group.bench_function("filter_count_10k_e2e", |b| {
        b.iter(|| {
            let result = conn.query(black_box("MATCH (p:Person) WHERE p.age > 30 RETURN COUNT(p)"));
            black_box(result.unwrap());
        })
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_query_match_return,
    bench_query_match_filter,
    bench_query_match_order,
    bench_query_match_limit,
    bench_query_create_node_table,
    bench_query_copy_csv,
    bench_query_filter_count_10k_vs_cpp,
    bench_query_filter_count_10k_e2e,
);
criterion_main!(benches);
