//! Vector write cost benchmark (WP-A1 / SC-001).
//!
//! Measures the median time of a single-row write into a node table with one
//! vector index, parameterised over 1,000 and 10,000 pre-existing rows.

use akar_common::types::Value;
use akar_main::connection::Connection;
use akar_main::database::{Database, SystemConfig};
use akar_main::{TypedNode, insert_nodes};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

struct MemoryNode {
    id: i64,
    embedding: Vec<f32>,
}

impl TypedNode for MemoryNode {
    const TABLE: &'static str = "Memory";
    const COLUMNS: &'static [&'static str] = &["id", "embedding"];

    fn to_values(&self) -> Vec<Value> {
        vec![
            Value::Int64(self.id),
            Value::List(self.embedding.iter().map(|f| Value::Double(*f as f64)).collect()),
        ]
    }
}

fn setup_indexed_db(size: usize) -> (tempfile::TempDir, Arc<Database>, Connection, AtomicI64) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("bench_db");
    let config = SystemConfig::default();
    let db = Arc::new(Database::new(db_path, config).expect("open db"));
    let conn = Connection::new(&db);

    conn.query("CREATE NODE TABLE Memory (id INT64, embedding FLOAT[3], PRIMARY KEY (id))")
        .expect("create Memory table");

    #[cfg(feature = "vector-extension")]
    db.create_vector_index(
        "mem_vec_idx".to_string(),
        "Memory".to_string(),
        "embedding".to_string(),
        akar_vector::hnsw::DistanceMetric::Cosine,
        3,
    )
    .expect("create vector index");

    if size > 0 {
        let rows: Vec<MemoryNode> = (1..=size as i64)
            .map(|i| MemoryNode {
                id: i,
                embedding: vec![i as f32, 0.5, 0.25],
            })
            .collect();
        insert_nodes(&conn, &rows, 500).expect("bulk insert pre-existing rows");
    }

    let next_id = AtomicI64::new(size as i64 + 1);
    (dir, db, conn, next_id)
}

fn bench_vector_write_cost(c: &mut Criterion) {
    let mut group = c.benchmark_group("vector_write_cost");
    group.sample_size(20);
    group.measurement_time(std::time::Duration::from_secs(3));

    for &size in &[1_000, 10_000] {
        let mode = if size <= 50_000 { "exact" } else { "graph" };
        println!("\n[vector_write_cost] corpus_size: {size}, index_mode: {mode}");

        let (_dir, _db, conn, counter) = setup_indexed_db(size);

        group.bench_with_input(BenchmarkId::new("single_insert", size), &size, |b, _| {
            b.iter(|| {
                let id = counter.fetch_add(1, Ordering::Relaxed);
                let q = format!("CREATE (m:Memory {{id: {id}, embedding: [1.0, 0.5, 0.25]}})");
                black_box(conn.query(&q).expect("single insert"));
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_vector_write_cost);
criterion_main!(benches);
