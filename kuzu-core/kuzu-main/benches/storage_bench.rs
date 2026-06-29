//! Storage engine benchmarks.
//!
//! Measures BufferManager throughput (pin/unpin), table scan throughput,
//! and columnar read/write performance.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use kuzu_common::memory::MemoryManager;
use kuzu_main::connection::Connection;
use kuzu_main::database::{Database, SystemConfig};
use kuzu_storage::buffer_manager::{BufferManager, BufferManagerConfig};
use std::sync::Arc;
use std::sync::Mutex;
use tempfile::tempdir;

fn bench_buffer_pin_unpin(c: &mut Criterion) {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("storage");
    let mm = Arc::new(MemoryManager::new(u64::MAX));
    let config = BufferManagerConfig::default();
    let bm = Mutex::new(BufferManager::new(db_path, mm, config));

    // Pre-allocate a page
    let file_name = "bench".to_string();
    {
        let mut guard = bm.lock().unwrap();
        guard.pin_mut(&file_name, 0).unwrap();
        guard.unpin(&file_name, 0);
    }

    c.bench_function("buffer/pin_unpin", |b| {
        b.iter(|| {
            let mut guard = bm.lock().unwrap();
            let frame = guard.pin(&file_name, black_box(0));
            black_box(frame.unwrap());
        })
    });
}

fn bench_table_scan_small(c: &mut Criterion) {
    let (dir, _db, conn) = setup_table_with_rows(100);
    let _ = &dir;

    c.bench_function("scan/small_100_rows", |b| {
        b.iter(|| {
            let result = conn.query(black_box("MATCH (n:Bench) RETURN n.val"));
            black_box(result.unwrap());
        })
    });
}

fn bench_table_scan_medium(c: &mut Criterion) {
    let (_dir, _db, conn) = setup_table_with_rows(1000);

    c.bench_function("scan/medium_1k_rows", |b| {
        b.iter(|| {
            let result = conn.query(black_box("MATCH (n:Bench) RETURN n.val"));
            black_box(result.unwrap());
        })
    });
}

/// Set up a Bench table with N rows.
fn setup_table_with_rows(n: usize) -> (tempfile::TempDir, Arc<Database>, Connection) {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("scan_bench");
    let config = SystemConfig::default();
    let database = Arc::new(Database::new(db_path, config).unwrap());
    let conn = Connection::new(&database);

    conn.query("CREATE NODE TABLE Bench(name STRING, val INT64, PRIMARY KEY (name))")
        .unwrap();

    // Generate CSV with N rows
    let csv_path = dir.path().join("scan_data.csv");
    let mut content = String::from("name,val\n");
    for i in 0..n {
        content.push_str(&format!("row_{i},{i}\n"));
    }
    std::fs::write(&csv_path, &content).unwrap();
    let fp = csv_path.to_string_lossy().replace('\\', "/");
    conn.query(&format!("COPY Bench FROM '{fp}' (HEADER true)")).unwrap();

    (dir, database, conn)
}

criterion_group!(
    benches,
    bench_buffer_pin_unpin,
    bench_table_scan_small,
    bench_table_scan_medium,
);
criterion_main!(benches);
