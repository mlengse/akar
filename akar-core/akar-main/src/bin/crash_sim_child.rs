/// Crash simulation child process for P41.1 crash recovery testing.
///
/// Usage:
///   crash_sim_child <db_path> <mode> <num_rows> <checkpoint_threshold>
///
/// Modes:
///   write              — Insert rows one-by-one, commit each
///   write-burst        — Insert rows in batches of 100
///   write-and-checkpoint — Insert rows then force a CHECKPOINT
///   ddl-recovery       — Open an existing DB, verify `Person` exists (created
///                        by the parent process), create `Person2`, insert a row
///   hold-lock          — Open the DB and hold the cross-process lock; prints
///                        `LOCK-HELD` on success (then waits for the signal file
///                        like the other modes) or `LOCK-ERROR: <msg>` + exit 2
///                        if the open is rejected. Used to verify the
///                        cross-process lock guard (P53.35).
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
        eprintln!("Usage: crash_sim_child <db_path> <mode> <num_rows> <checkpoint_threshold>");
        eprintln!("Modes: write, write-burst, write-and-checkpoint");
        std::process::exit(1);
    }

    let db_path = PathBuf::from(&args[1]);
    let mode = &args[2];
    let num_rows: usize = args[3].parse().expect("num_rows must be a number");
    let checkpoint_threshold: i64 = args[4].parse().expect("checkpoint_threshold must be a number");

    let signal_path = db_path.join("signal");

    let config = SystemConfig {
        buffer_pool_size: 64 * 1024 * 1024,
        max_num_threads: 4,
        auto_checkpoint: checkpoint_threshold != 0,
        checkpoint_threshold,
        concurrent_writes: true,
        ..Default::default()
    };

    if mode == "hold-lock" {
        match Database::new(&db_path, config) {
            Ok(_db) => println!("LOCK-HELD"),
            Err(e) => {
                println!("LOCK-ERROR: {e}");
                std::process::exit(2);
            }
        }
    } else {
        let db = Arc::new(Database::new(&db_path, config).expect("Failed to create/open database"));
        let conn = Connection::new(&db);

        if mode != "ddl-recovery" {
            // Create the Person table (in-memory catalog, per-process)
            conn.query(
                "CREATE NODE TABLE Person(name STRING, age INT64, score DOUBLE, active BOOL, PRIMARY KEY(name))",
            )
            .expect("Failed to create Person table");
        }

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
                // All rows written and committed (each query's durable commit
                // completed, including WAL flush + column-mirror persist).
                // Signal the parent so it can kill us while idle — a hard
                // kill (SIGKILL) with no clean shutdown, but with all
                // committed rows already durable on disk.
                fs::write(db_path.join("write_done"), b"").expect("Failed to write write_done marker");
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
            "ddl-recovery" => {
                // Cross-process DDL recovery: the `Person` table was created by the
                // parent process and persisted to catalog.json. It must be visible
                // here without re-creating it.
                let person_exists = db.catalog().lock().map(|c| c.contains("Person")).unwrap_or(false);
                if !person_exists {
                    eprintln!("DDL-RECOVERY-FAIL: 'Person' table missing after restart");
                    std::process::exit(2);
                }
                // Create a new table in this process; it must be persisted so the
                // parent process can see it after we exit.
                conn.query("CREATE NODE TABLE Person2(name STRING, PRIMARY KEY(name))")
                    .expect("Failed to create Person2 table");
                conn.query("CREATE (:Person2 {name: 'child_row'})")
                    .expect("insert failed");
                println!("DDL-RECOVERY-OK");
            }
            _ => {
                eprintln!("Unknown mode: {}", mode);
                std::process::exit(1);
            }
        }

        drop(conn);
        drop(db);
    }

    for _ in 0..6000 {
        if signal_path.exists() {
            let _ = fs::remove_file(&signal_path);
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
}
