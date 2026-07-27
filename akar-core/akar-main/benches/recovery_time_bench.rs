use akar_main::database::{Database, SystemConfig};
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::Arc;
use tempfile::TempDir;

fn bench_recovery_time(c: &mut Criterion) {
    let mut group = c.benchmark_group("recovery_time");
    for &n in &[100, 1_000] {
        group.bench_function(format!("wal_{n}_rows"), |b| {
            b.iter_custom(|iters| {
                let dir = TempDir::new().unwrap();
                let db_path = dir.path().join("bench");

                {
                    let config = SystemConfig::default();
                    let database = Arc::new(Database::new(db_path.clone(), config).unwrap());
                    let conn = akar_main::connection::Connection::new(&database);
                    conn.query("CREATE NODE TABLE Bench(name STRING, val INT64, PRIMARY KEY (name))")
                        .unwrap();

                    let csv_path = dir.path().join("data.csv");
                    let mut content = String::from("name,val\n");
                    for i in 0..n {
                        content.push_str(&format!("row_{i},{i}\n"));
                    }
                    std::fs::write(&csv_path, &content).unwrap();
                    let fp = csv_path.to_string_lossy().replace('\\', "/");
                    conn.query(&format!("COPY Bench FROM '{fp}' (HEADER true)"))
                        .unwrap();
                }

                let start = std::time::Instant::now();
                for _ in 0..iters {
                    let config = SystemConfig::default();
                    let _ = black_box(Database::new(db_path.clone(), config));
                }
                start.elapsed()
            })
        });
    }
    group.finish();
}

criterion_group!(benches, bench_recovery_time);
criterion_main!(benches);
