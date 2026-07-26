/// Crash simulation child process for P41.1 crash recovery testing.
///
/// Usage:
///   crash_sim_child <db_path> <mode> <num_rows> <checkpoint_threshold>
///
/// Modes:
///   write              — Insert rows one-by-one, commit each
///   write-burst        — Insert rows in batches of 100
///   write-and-checkpoint — Insert rows then force a CHECKPOINT
///
/// The parent process is expected to have already created the DB, table, and
/// checkpointed it. This process only performs DML (INSERT).
///
/// The process waits for `<db_path>/signal` file to appear before clean exit.
/// If killed before the signal, it simulates a crash (no cleanup).
use akar_main::{Connection, Database, SystemConfig};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!(
            "Usage: crash_sim_child <db_path> <mode> <num_rows> <checkpoint_threshold>"
        );
        eprintln!("Modes: write, write-burst, write-and-checkpoint");
        std::process::exit(1);
    }

    let db_path = PathBuf::from(&args[1]);
    let mode = &args[2];
    let num_rows: usize = args[3].parse().expect("num_rows must be a number");
    let checkpoint_threshold: i64 = args[4]
        .parse()
        .expect("checkpoint_threshold must be a number");

    let signal_path = db_path.join("signal");

    let config = SystemConfig {
        buffer_pool_size: 64 * 1024 * 1024,
        max_num_threads: 4,
        auto_checkpoint: checkpoint_threshold != 0,
        checkpoint_threshold,
        concurrent_writes: true,
        ..Default::default()
    };

    let db = Arc::new(Database::new(&db_path, config).expect("Failed to create/open database"));
    let conn = Connection::new(&db);

    // Create the Person table (in-memory catalog, per-process)
    conn.query(
        "CREATE NODE TABLE Person(name STRING, age INT64, score DOUBLE, active BOOL, PRIMARY KEY(name))",
    )
    .expect("Failed to create Person table");

    match mode.as_str() {
        "write" => {
            for i in 0..num_rows {
                let name = format!("person_{}", i);
                let age = (i % 100) as i64;
                let score = (i as f64) * 1.5;
                let active = i % 2 == 0;
                conn.query(&format!(
                    "CREATE (p:Person {{name: '{}', age: {}, score: {}, active: {}}})",
                    name, age, score, active
                ))
                .unwrap_or_else(|e| panic!("Failed to insert row {}: {}", i, e));
            }
        }
        "write-burst" => {
            let batch_size = 100;
            let mut inserted = 0;
            while inserted < num_rows {
                let batch_end = (inserted + batch_size).min(num_rows);
                for i in inserted..batch_end {
                    let name = format!("person_{}", i);
                    let age = (i % 100) as i64;
                    let score = (i as f64) * 1.5;
                    let active = i % 2 == 0;
                    conn.query(&format!(
                        "CREATE (p:Person {{name: '{}', age: {}, score: {}, active: {}}})",
                        name, age, score, active
                    ))
                    .unwrap_or_else(|e| panic!("Failed to insert row {}: {}", i, e));
                }
                inserted = batch_end;
            }
        }
        "write-and-checkpoint" => {
            for i in 0..num_rows {
                let name = format!("person_{}", i);
                let age = (i % 100) as i64;
                let score = (i as f64) * 1.5;
                let active = i % 2 == 0;
                conn.query(&format!(
                    "CREATE (p:Person {{name: '{}', age: {}, score: {}, active: {}}})",
                    name, age, score, active
                ))
                .unwrap_or_else(|e| panic!("Failed to insert row {}: {}", i, e));
            }
            conn.query("CHECKPOINT").expect("Failed to checkpoint");
        }
        _ => {
            eprintln!("Unknown mode: {}", mode);
            std::process::exit(1);
        }
    }

    drop(conn);
    drop(db);

    for _ in 0..6000 {
        if signal_path.exists() {
            let _ = fs::remove_file(&signal_path);
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
}
