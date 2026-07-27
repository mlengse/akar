use akar_main::connection::Connection;
use akar_main::database::{Database, SystemConfig};
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::Arc;
use tempfile::TempDir;

fn bench_insert_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("storage_io/insert");
    for &n in &[100, 1_000] {
        group.bench_function(format!("batch_{n}"), |b| {
            b.iter_custom(|iters| {
                let dir = TempDir::new().unwrap();
                let db_path = dir.path().join("bench");
                let config = SystemConfig::default();
                let database = Arc::new(Database::new(db_path, config).unwrap());
                let conn = Connection::new(&database);
                conn.query("CREATE NODE TABLE Bench(name STRING, val INT64, PRIMARY KEY (name))")
                    .unwrap();

                let csv_path = dir.path().join("data.csv");
                let mut content = String::from("name,val\n");
                for i in 0..n {
                    content.push_str(&format!("row_{i},{i}\n"));
                }
                std::fs::write(&csv_path, &content).unwrap();
                let fp = csv_path.to_string_lossy().replace('\\', "/");

                let start = std::time::Instant::now();
                for _ in 0..iters {
                    let q = format!("COPY Bench FROM '{fp}' (HEADER true)");
                    let _ = conn.query(black_box(&q));
                }
                start.elapsed()
            })
        });
    }
    group.finish();
}

fn bench_checkpoint(c: &mut Criterion) {
    c.bench_function("storage_io/checkpoint_1k_rows", |b| {
        b.iter_custom(|iters| {
            let dir = TempDir::new().unwrap();
            let db_path = dir.path().join("bench");
            let mut config = SystemConfig::default();
            config.checkpoint_threshold = -1;
            let database = Arc::new(Database::new(db_path, config).unwrap());
            let conn = Connection::new(&database);
            conn.query("CREATE NODE TABLE Bench(name STRING, val INT64, PRIMARY KEY (name))")
                .unwrap();

            let csv_path = dir.path().join("data.csv");
            let mut content = String::from("name,val\n");
            for i in 0..1_000 {
                content.push_str(&format!("row_{i},{i}\n"));
            }
            std::fs::write(&csv_path, &content).unwrap();
            let fp = csv_path.to_string_lossy().replace('\\', "/");
            conn.query(&format!("COPY Bench FROM '{fp}' (HEADER true)"))
                .unwrap();

            let start = std::time::Instant::now();
            for _ in 0..iters {
                let _ = conn.query(black_box("MATCH (n:Bench) RETURN COUNT(n)"));
            }
            start.elapsed()
        })
    });
}

criterion_group!(benches, bench_insert_throughput, bench_checkpoint);
criterion_main!(benches);
