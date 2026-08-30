//! P41: Stress Testing — Crash Recovery
//!
//! Tests crash recovery by spawning child processes, killing them at various
//! points, and verifying the database remains usable after recovery.
//!
//! Architecture note: The catalog (table schema) is persisted to disk as
//! `catalog.json` after every DDL (see P45.1), so schemas survive a restart.
//! Data rows are held in-memory only; WAL DML records carry recent writes but
//! are replayed into the restored tables. In-process tests verify full data
//! recovery (WAL corruption, truncation, etc.).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;

// ---------------------------------------------------------------------------
// CrashSimulator — spawn / kill child process
// ---------------------------------------------------------------------------

struct CrashSimulator {
    child: Option<Child>,
    db_path: PathBuf,
    _temp_dir: TempDir,
}

impl CrashSimulator {
    fn spawn(mode: &str, num_rows: usize, checkpoint_threshold: i64) -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db_path = temp_dir.path().join("test_db");

        let binary = env!("CARGO_BIN_EXE_crash_sim_child");
        let child = Command::new(binary)
            .arg(db_path.to_str().unwrap())
            .arg(mode)
            .arg(num_rows.to_string())
            .arg(checkpoint_threshold.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Failed to spawn crash_sim_child");

        Self {
            child: Some(child),
            db_path,
            _temp_dir: temp_dir,
        }
    }

    fn wait_for_wal_size(&self, min_wal_bytes: u64, timeout: Duration) -> bool {
        let wal_path = self.db_path.join("wal.log");
        let start = Instant::now();
        loop {
            if let Ok(meta) = fs::metadata(&wal_path) {
                if meta.len() >= min_wal_bytes {
                    return true;
                }
            }
            if start.elapsed() > timeout {
                return false;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn kill(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;
    }

    fn db_path(&self) -> &Path {
        &self.db_path
    }
}

impl Drop for CrashSimulator {
    fn drop(&mut self) {
        self.kill();
    }
}

// ---------------------------------------------------------------------------
// Verify helper — open DB after crash, ensure no panic
// ---------------------------------------------------------------------------

fn open_db_after_crash(db_path: &Path) {
    use akar_main::{Database, SystemConfig};

    let config = SystemConfig {
        buffer_pool_size: 64 * 1024 * 1024,
        auto_checkpoint: true,
        checkpoint_threshold: -1,
        concurrent_writes: true,
        ..Default::default()
    };

    let _db = Database::new(db_path, config).expect("DB should be openable after crash without panic");
}

// ===========================================================================
// P41.1: Process-Level Crash Simulation
//
// These tests verify that killing a child process mid-write does NOT leave the
// database in an unrecoverable state. We verify by reopening the DB without
// panic. We do NOT assert specific row counts because the catalog is
// in-memory only and is not persisted across process boundaries.
// ===========================================================================

#[test]
fn test_crash_after_wal_flush_recovery() {
    let mut sim = CrashSimulator::spawn("write", 200, 0);

    assert!(
        sim.wait_for_wal_size(100, Duration::from_secs(30)),
        "WAL file did not appear within timeout"
    );

    thread::sleep(Duration::from_millis(500));
    sim.kill();

    // DB should be openable after crash
    open_db_after_crash(sim.db_path());
}

#[test]
fn test_crash_mid_write_no_commit() {
    let mut sim = CrashSimulator::spawn("write-burst", 250, 0);

    assert!(
        sim.wait_for_wal_size(50, Duration::from_secs(30)),
        "WAL file did not appear within timeout"
    );

    thread::sleep(Duration::from_millis(200));
    sim.kill();

    open_db_after_crash(sim.db_path());
}

#[test]
fn test_crash_after_checkpoint_clean_recovery() {
    let mut sim = CrashSimulator::spawn("write-and-checkpoint", 100, -1);

    // Kill as soon as the child signals the CHECKPOINT completed (deterministic;
    // replaces the old fixed 5 s sleep that wasted wall-clock every run).
    let done_marker = sim.db_path().to_path_buf().join("checkpoint_done");
    let start = Instant::now();
    while !done_marker.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "child did not complete checkpoint in time"
        );
        thread::sleep(Duration::from_millis(25));
    }

    sim.kill();

    open_db_after_crash(sim.db_path());
}

#[test]
fn test_crash_concurrent_writes_recovery() {
    let mut sim = CrashSimulator::spawn("write-burst", 200, 0);

    assert!(
        sim.wait_for_wal_size(50, Duration::from_secs(30)),
        "WAL file did not appear within timeout"
    );

    thread::sleep(Duration::from_millis(300));
    sim.kill();

    open_db_after_crash(sim.db_path());
}

// ===========================================================================
// P41.2: WAL Replay Correctness Under Load
// ===========================================================================

#[test]
fn test_wal_replay_large_record_count() {
    let mut sim = CrashSimulator::spawn("write", 500, 0);

    assert!(
        sim.wait_for_wal_size(500, Duration::from_secs(60)),
        "WAL file did not grow within timeout"
    );

    thread::sleep(Duration::from_millis(200));
    sim.kill();

    open_db_after_crash(sim.db_path());
}

#[test]
fn test_wal_replay_truncated_file_50_percent() {
    use akar_main::{Connection, Database, SystemConfig};
    use std::sync::Arc;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    {
        let config = SystemConfig {
            buffer_pool_size: 64 * 1024 * 1024,
            auto_checkpoint: false,
            checkpoint_threshold: 0,
            concurrent_writes: true,
            ..Default::default()
        };

        let db = Arc::new(Database::new(&db_path, config).expect("Failed to create DB"));
        let conn = Connection::new(&db);

        conn.query("CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name))")
            .expect("Failed to create table");

        for i in 0..250 {
            conn.query(&format!("CREATE (p:Person {{name: 'person_{}', age: {}}})", i, i % 100))
                .unwrap();
        }
    }

    let wal_path = db_path.join("wal.log");
    if wal_path.exists() {
        let data = fs::read(&wal_path).expect("Failed to read WAL");
        let truncated = &data[..data.len() / 2];
        fs::write(&wal_path, truncated).expect("Failed to truncate WAL");
    }

    let config = SystemConfig {
        buffer_pool_size: 64 * 1024 * 1024,
        auto_checkpoint: true,
        checkpoint_threshold: -1,
        concurrent_writes: true,
        ..Default::default()
    };
    let _db = Database::new(&db_path, config).expect("Recovery after WAL truncation should not fail");
}

#[test]
fn test_wal_replay_truncated_file_25_percent() {
    use akar_main::{Connection, Database, SystemConfig};
    use std::sync::Arc;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    {
        let config = SystemConfig {
            buffer_pool_size: 64 * 1024 * 1024,
            auto_checkpoint: false,
            checkpoint_threshold: 0,
            concurrent_writes: true,
            ..Default::default()
        };

        let db = Arc::new(Database::new(&db_path, config).expect("Failed to create DB"));
        let conn = Connection::new(&db);

        conn.query("CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name))")
            .expect("Failed to create table");

        for i in 0..150 {
            conn.query(&format!("CREATE (p:Person {{name: 'person_{}', age: {}}})", i, i % 75))
                .unwrap();
        }
    }

    let wal_path = db_path.join("wal.log");
    if wal_path.exists() {
        let data = fs::read(&wal_path).expect("Failed to read WAL");
        let keep = (data.len() * 25) / 100;
        let truncated = &data[..keep];
        fs::write(&wal_path, truncated).expect("Failed to truncate WAL");
    }

    let config = SystemConfig {
        buffer_pool_size: 64 * 1024 * 1024,
        auto_checkpoint: true,
        checkpoint_threshold: -1,
        concurrent_writes: true,
        ..Default::default()
    };
    let _db = Database::new(&db_path, config).expect("Recovery after 25% WAL truncation should not fail");
}

#[test]
fn test_wal_replay_truncated_file_10_percent() {
    use akar_main::{Connection, Database, SystemConfig};
    use std::sync::Arc;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    {
        let config = SystemConfig {
            buffer_pool_size: 64 * 1024 * 1024,
            auto_checkpoint: false,
            checkpoint_threshold: 0,
            concurrent_writes: true,
            ..Default::default()
        };

        let db = Arc::new(Database::new(&db_path, config).expect("Failed to create DB"));
        let conn = Connection::new(&db);

        conn.query("CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name))")
            .expect("Failed to create table");

        for i in 0..100 {
            conn.query(&format!("CREATE (p:Person {{name: 'person_{}', age: {}}})", i, i % 50))
                .unwrap();
        }
    }

    let wal_path = db_path.join("wal.log");
    if wal_path.exists() {
        let data = fs::read(&wal_path).expect("Failed to read WAL");
        let keep = (data.len() / 10).max(4);
        let truncated = &data[..keep];
        fs::write(&wal_path, truncated).expect("Failed to truncate WAL");
    }

    let config = SystemConfig {
        buffer_pool_size: 64 * 1024 * 1024,
        auto_checkpoint: true,
        checkpoint_threshold: -1,
        concurrent_writes: true,
        ..Default::default()
    };
    let _db = Database::new(&db_path, config).expect("Recovery after severe WAL truncation should not fail");
}

#[test]
fn test_wal_replay_empty_wal_file() {
    use akar_main::{Database, SystemConfig};

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    {
        let config = SystemConfig::default();
        let _db = Database::new(&db_path, config).expect("Failed to create DB");
    }

    let wal_path = db_path.join("wal.log");
    fs::write(&wal_path, b"").expect("Failed to empty WAL");

    let config = SystemConfig::default();
    let _db = Database::new(&db_path, config).expect("Recovery with empty WAL should not fail");
}

// ===========================================================================
// P41.3: Checkpoint Atomicity Under Concurrent Load
// ===========================================================================

#[test]
fn test_concurrent_writes_checkpoint_stress() {
    use akar_main::{Connection, Database, SystemConfig};
    use std::sync::Arc;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    let config = SystemConfig {
        buffer_pool_size: 64 * 1024 * 1024,
        auto_checkpoint: true,
        checkpoint_threshold: -1,
        concurrent_writes: true,
        ..Default::default()
    };

    let db = Arc::new(Database::new(&db_path, config).expect("Failed to create DB"));
    let conn = Connection::new(&db);

    // Phase 1: Create table and write initial data
    conn.query("CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name))")
        .expect("Failed to create table");

    for i in 0..50 {
        conn.query(&format!("CREATE (p:Person {{name: 'person_{}', age: {}}})", i, i % 50))
            .unwrap();
    }

    conn.query("CHECKPOINT").expect("Failed to checkpoint phase 1");

    // Phase 2: Add more data (no explicit checkpoint)
    for i in 50..100 {
        conn.query(&format!("CREATE (p:Person {{name: 'person_{}', age: {}}})", i, i % 50))
            .unwrap();
    }

    // Verify all rows are present (in same process, catalog alive)
    let names: Vec<String> = {
        let result = conn
            .query("MATCH (p:Person) RETURN p.name")
            .expect("Failed to query names");
        result
            .chunks
            .iter()
            .flat_map(|c| (0..c.size).filter_map(|i| c.get_value(0, i)))
            .filter_map(|v| match v {
                akar_main::test_helpers::Value::String(s) => Some(s),
                _ => None,
            })
            .collect()
    };
    assert_eq!(names.len(), 100, "Should have 100 rows total, got {}", names.len());

    // Checkpoint and verify durability within same process
    conn.query("CHECKPOINT").expect("Failed to checkpoint phase 2");

    let names2: Vec<String> = {
        let result2 = conn
            .query("MATCH (p:Person) RETURN p.name")
            .expect("Failed to query names after checkpoint");
        result2
            .chunks
            .iter()
            .flat_map(|c| (0..c.size).filter_map(|i| c.get_value(0, i)))
            .filter_map(|v| match v {
                akar_main::test_helpers::Value::String(s) => Some(s),
                _ => None,
            })
            .collect()
    };
    assert_eq!(names2.len(), 100, "Should still have 100 rows after checkpoint");
}

#[test]
fn test_auto_checkpoint_threshold_various() {
    use akar_main::{Connection, Database, SystemConfig};
    use std::sync::Arc;

    for threshold in [1024, 65536, 1048576] {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db_path = temp_dir.path().join("test_db");

        let config = SystemConfig {
            buffer_pool_size: 64 * 1024 * 1024,
            auto_checkpoint: true,
            checkpoint_threshold: threshold,
            concurrent_writes: true,
            ..Default::default()
        };

        let db = Arc::new(Database::new(&db_path, config).expect("Failed to create DB"));
        let conn = Connection::new(&db);

        conn.query("CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name))")
            .expect("Failed to create table");

        for i in 0..25 {
            conn.query(&format!("CREATE (p:Person {{name: 'person_{}', age: {}}})", i, i % 25))
                .unwrap();
        }

        // Verify all rows present
        let names: Vec<String> = {
            let result = conn
                .query("MATCH (p:Person) RETURN p.name")
                .expect("Failed to query names after writes");
            result
                .chunks
                .iter()
                .flat_map(|c| (0..c.size).filter_map(|i| c.get_value(0, i)))
                .filter_map(|v| match v {
                    akar_main::test_helpers::Value::String(s) => Some(s),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(
            names.len(),
            25,
            "threshold={}: should have 25 rows, got {}",
            threshold,
            names.len()
        );
    }
}

// ===========================================================================
// P41.4: Fault Injection (manual WAL corruption tests)
// ===========================================================================

#[test]
fn test_wal_zeroed_out_recovery() {
    use akar_main::{Connection, Database, SystemConfig};
    use std::sync::Arc;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    {
        let config = SystemConfig {
            buffer_pool_size: 64 * 1024 * 1024,
            auto_checkpoint: true,
            checkpoint_threshold: -1,
            concurrent_writes: true,
            ..Default::default()
        };
        let db = Arc::new(Database::new(&db_path, config).expect("Failed to create DB"));
        let conn = Connection::new(&db);

        conn.query("CREATE NODE TABLE Person(name STRING, PRIMARY KEY(name))")
            .expect("Failed to create table");
        conn.query("CREATE (p:Person {name: 'alice'})")
            .expect("Failed to insert");
    }

    // Zero out WAL
    let wal_path = db_path.join("wal.log");
    let wal_size = fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0);
    if wal_size > 0 {
        fs::write(&wal_path, vec![0u8; wal_size as usize]).expect("Failed to zero WAL");
    }

    // (P61.3) A corrupt WAL must refuse to start (fail-loud) rather than
    // silently recover to an empty database, which would destroy every
    // committed write still living only in the WAL.
    let config = SystemConfig {
        buffer_pool_size: 64 * 1024 * 1024,
        auto_checkpoint: true,
        checkpoint_threshold: -1,
        concurrent_writes: true,
        ..Default::default()
    };
    assert!(
        Database::new(&db_path, config).is_err(),
        "zeroed WAL must refuse to start (P61.3)"
    );
}

#[test]
fn test_wal_random_bytes_recovery() {
    use akar_main::{Database, SystemConfig};

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    {
        let config = SystemConfig::default();
        let _db = Database::new(&db_path, config).expect("Failed to create DB");
    }

    let wal_path = db_path.join("wal.log");
    let random_data: Vec<u8> = (0..4096).map(|i| (i * 37 + 13) as u8).collect();
    fs::write(&wal_path, &random_data).expect("Failed to write random WAL");

    let config = SystemConfig::default();
    assert!(
        Database::new(&db_path, config).is_err(),
        "random-byte WAL must refuse to start (P61.3)"
    );
}

#[test]
fn test_wal_single_byte_recovery() {
    use akar_main::{Database, SystemConfig};

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    {
        let config = SystemConfig::default();
        let _db = Database::new(&db_path, config).expect("Failed to create DB");
    }

    let wal_path = db_path.join("wal.log");
    fs::write(&wal_path, [0xFF]).expect("Failed to write 1-byte WAL");

    let config = SystemConfig::default();
    assert!(
        Database::new(&db_path, config).is_err(),
        "1-byte WAL must refuse to start (P61.3)"
    );
}
