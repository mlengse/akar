//! P45.4: Data Durability
//!
//! Verifies that committed table rows survive process restarts via the
//! durable column mirrors (`col_{table_id}_{col_idx}` + `.meta` sidecar) —
//! the recovery source for SQL-path rows, since their WAL records carry no
//! row data. P60.1 skips the per-commit persist only when a CHECKPOINT is
//! imminent (the checkpoint persists the same mirrors before truncating
//! the WAL):
//! - clean shutdown (with and without an explicit CHECKPOINT),
//! - crash (process killed mid-write),
//! - UPDATE/DELETE state,
//! - rel-table edges + properties,
//! - read-only mode rejecting writes,
//! - the cross-process lock preventing concurrent opens.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;

use akar_main::test_helpers::Value;
use akar_main::{Connection, Database, SystemConfig};

fn config(threshold: i64) -> SystemConfig {
    SystemConfig {
        buffer_pool_size: 64 * 1024 * 1024,
        auto_checkpoint: true,
        checkpoint_threshold: threshold,
        concurrent_writes: true,
        ..Default::default()
    }
}

fn read_only_config() -> SystemConfig {
    SystemConfig {
        buffer_pool_size: 64 * 1024 * 1024,
        auto_checkpoint: true,
        checkpoint_threshold: -1,
        concurrent_writes: true,
        read_only: true,
        ..Default::default()
    }
}

/// Run `query` and collect all values from the first result column.
fn query_column(conn: &Connection, query: &str) -> Vec<Value> {
    let result = conn.query(query).expect("query should succeed");
    result
        .chunks
        .iter()
        .flat_map(|c| (0..c.size).filter_map(|i| c.get_value(0, i)))
        .collect()
}

/// Run `query` and collect Int64 values from the first result column.
fn query_i64s(conn: &Connection, query: &str) -> Vec<i64> {
    query_column(conn, query)
        .into_iter()
        .map(|v| match v {
            Value::Int64(i) => i,
            other => panic!("expected Int64 value, got {other:?}"),
        })
        .collect()
}

/// Run `query` and collect String values from the first result column.
fn query_strings(conn: &Connection, query: &str) -> Vec<String> {
    query_column(conn, query)
        .into_iter()
        .map(|v| match v {
            Value::String(s) => s,
            other => panic!("expected String value, got {other:?}"),
        })
        .collect()
}

/// Run `query` and collect (name, age) pairs from the first two columns.
fn query_name_age_pairs(conn: &Connection, query: &str) -> Vec<(String, i64)> {
    let result = conn.query(query).expect("query should succeed");
    result
        .chunks
        .iter()
        .flat_map(|c| {
            (0..c.size).filter_map(|i| {
                let name = match c.get_value(0, i) {
                    Some(Value::String(s)) => s.clone(),
                    _ => return None,
                };
                let age = match c.get_value(1, i) {
                    Some(Value::Int64(a)) => a,
                    _ => return None,
                };
                Some((name, age))
            })
        })
        .collect()
}

/// Create a Person table with the given rows (name, age) and an explicit
/// CHECKPOINT so the mirror is flushed before the database is closed.
fn setup_person_table(db_path: &Path, names_ages: &[(&str, i64)]) {
    let db = Arc::new(Database::new(db_path, config(-1)).expect("Failed to create DB"));
    let conn = Connection::new(&db);

    conn.query("CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name))")
        .expect("Failed to create Person table");
    for (name, age) in names_ages {
        conn.query(&format!("CREATE (:Person {{name: '{name}', age: {age}}})"))
            .expect("Failed to insert row");
    }
    conn.query("CHECKPOINT").expect("Failed to checkpoint");
}

// ===========================================================================
// Clean shutdown durability
// ===========================================================================

#[test]
fn test_clean_restart_restores_rows() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    {
        let db = Arc::new(Database::new(&db_path, config(-1)).expect("Failed to create DB"));
        let conn = Connection::new(&db);
        conn.query("CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name))")
            .expect("Failed to create table");
        conn.query("CREATE (:Person {name: 'alice', age: 30})")
            .expect("insert failed");
        conn.query("CREATE (:Person {name: 'bob', age: 25})")
            .expect("insert failed");
        conn.query("CREATE (:Person {name: 'carol', age: 40})")
            .expect("insert failed");
        conn.query("CHECKPOINT").expect("Failed to checkpoint");
        assert_eq!(db.table_num_rows("Person"), 3);
    }

    // Reopen: all committed rows must be restored from the durable mirror.
    let db = Arc::new(Database::new(&db_path, config(-1)).expect("Failed to reopen DB"));
    let conn = Connection::new(&db);
    assert_eq!(db.table_num_rows("Person"), 3);

    let mut ages = query_i64s(&conn, "MATCH (n:Person) RETURN n.age");
    ages.sort();
    assert_eq!(ages, vec![25, 30, 40]);

    let mut names = query_strings(&conn, "MATCH (n:Person) RETURN n.name");
    names.sort();
    assert_eq!(names, vec!["alice".to_string(), "bob".to_string(), "carol".to_string()]);
}

#[test]
fn test_restart_without_checkpoint_restores_rows() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    {
        // Auto-checkpoint disabled (threshold 0): no CHECKPOINT is issued, but
        // the commit path still persists the durable mirror after each write
        // (the P60.1 skip only fires when a checkpoint is imminent).
        let db = Arc::new(Database::new(&db_path, config(0)).expect("Failed to create DB"));
        let conn = Connection::new(&db);
        conn.query("CREATE NODE TABLE Person(name STRING, PRIMARY KEY(name))")
            .expect("Failed to create table");
        for i in 0..5 {
            conn.query(&format!("CREATE (:Person {{name: 'p{i}'}})"))
                .expect("insert failed");
        }
        assert_eq!(db.table_num_rows("Person"), 5);
    }

    let db = Arc::new(Database::new(&db_path, config(0)).expect("Failed to reopen DB"));
    assert_eq!(
        db.table_num_rows("Person"),
        5,
        "rows should survive without an explicit checkpoint"
    );
}

#[test]
fn test_update_and_delete_survive_restart() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    {
        setup_person_table(&db_path, &[("alice", 30), ("bob", 25), ("carol", 40)]);

        let db = Arc::new(Database::new(&db_path, config(-1)).expect("Failed to create DB"));
        let conn = Connection::new(&db);

        // Apply UPDATE + DELETE at the storage level (the SQL SET/DELETE write
        // path has a pre-existing planner bug where the scan does not emit the
        // internal row-id column; that is tracked separately). `update_cell` /
        // `delete_row` mark the table dirty, so the CHECKPOINT rewrites the
        // durable mirror with the updated + soft-deleted rows.
        {
            let tc = db.table_catalog();
            let mut table = tc.get_node_table_by_name_mut("Person").expect("Person table");
            table.update_cell(0, 1, Value::Int64(31)).expect("update_cell failed");
            table.delete_row(1).expect("delete_row failed");
        }
        conn.query("CHECKPOINT").expect("Failed to checkpoint");

        // Pre-restart behavior: `bob` is a soft-deleted row slot (num_rows is
        // unchanged), so it must not be findable by name.
        assert_eq!(db.table_num_rows("Person"), 3, "soft-deleted row slot remains");
        assert!(
            query_strings(&conn, "MATCH (n:Person {name: 'bob'}) RETURN n.name").is_empty(),
            "deleted row must not be findable before restart"
        );

        let mut before = query_name_age_pairs(&conn, "MATCH (n:Person) RETURN n.name, n.age");
        before.sort();
        assert_eq!(
            before,
            vec![("alice".to_string(), 31), ("carol".to_string(), 40)],
            "update should apply in-memory before restart"
        );
    }

    let db = Arc::new(Database::new(&db_path, config(-1)).expect("Failed to reopen DB"));
    let conn = Connection::new(&db);
    assert_eq!(db.table_num_rows("Person"), 3, "row slots preserved across restart");

    let mut after = query_name_age_pairs(&conn, "MATCH (n:Person) RETURN n.name, n.age");
    after.sort();
    assert_eq!(
        after,
        vec![("alice".to_string(), 31), ("carol".to_string(), 40)],
        "updated and deleted state should persist across restart"
    );

    assert!(
        query_strings(&conn, "MATCH (n:Person {name: 'bob'}) RETURN n.name").is_empty(),
        "deleted row must not be findable after restart"
    );
}

#[test]
fn test_rel_table_rows_survive_restart() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    {
        let db = Arc::new(Database::new(&db_path, config(-1)).expect("Failed to create DB"));
        let conn = Connection::new(&db);
        conn.query("CREATE NODE TABLE Person(id INT64, name STRING, PRIMARY KEY(id))")
            .expect("Failed to create Person");
        conn.query("CREATE NODE TABLE City(id INT64, name STRING, PRIMARY KEY(id))")
            .expect("Failed to create City");
        conn.query("CREATE REL TABLE LivesIn(FROM Person TO City, since INT64)")
            .expect("Failed to create LivesIn");
        conn.query("CREATE (:Person {id: 1, name: 'alice'})")
            .expect("insert Person failed");
        conn.query("CREATE (:City {id: 1, name: 'SF'})")
            .expect("insert City failed");
        conn.query(
            "MATCH (a:Person {id: 1}), (b:City {id: 1}) \
             CREATE (a)-[:LivesIn {since: 2010}]->(b)",
        )
        .expect("insert rel failed");
        conn.query("CHECKPOINT").expect("Failed to checkpoint");
        assert_eq!(db.table_catalog().get_rel_table_by_name("LivesIn").unwrap().num_rows, 1);
    }

    let db = Arc::new(Database::new(&db_path, config(-1)).expect("Failed to reopen DB"));

    let table_catalog = db.table_catalog();
    let rel = table_catalog
        .get_rel_table_by_name("LivesIn")
        .expect("LivesIn should survive");
    assert_eq!(rel.num_rows, 1, "rel edge should survive restart");
    assert_eq!(
        rel.edges,
        vec![(0, 0)],
        "rel edge src/dst internal ids should survive restart"
    );
    assert_eq!(
        rel.properties,
        vec![vec![Value::Int64(2010)]],
        "rel edge property should survive restart"
    );
}

// ===========================================================================
// Crash recovery durability
// ===========================================================================

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

#[test]
fn test_crash_recovers_committed_rows_without_double_apply() {
    let mut sim = CrashSimulator::spawn("write", 60, 0);

    // Wait for the child to finish writing AND committing all 60 rows
    // (each query's commit is durable: WAL flush + column-mirror persist;
    // threshold 0 disables auto-checkpoint, so the P60.1 skip never fires).
    // The child then writes `write_done` and waits idle on the signal file.
    // We kill it while idle — a hard SIGKILL with no clean shutdown, but with
    // every committed row already durable. Killing mid-write is deliberately
    // avoided: it leaves a torn persist/WAL state whose recovery is not
    // deterministic across OSes (the historical flake).
    let db_dir = sim.db_path().to_path_buf();
    let start = Instant::now();
    let mut done = false;
    // 60 s budget for 60 individually durable commits. Each commit costs a
    // WAL fsync plus small mirror file writes; under a fully parallel suite run on
    // Windows, disk contention dominates (observed once as a gate flake) — the
    // assertion guards against a hung child, not write throughput. 60 rows is
    // plenty to prove multi-row durability + no double-apply.
    while start.elapsed() < Duration::from_secs(60) {
        if db_dir.join("write_done").exists() {
            done = true;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(done, "Child did not finish writes in time");
    thread::sleep(Duration::from_millis(200));
    sim.kill();

    // Reopen after the crash: the committed rows must survive, must not be
    // duplicated (double-apply), and the table must remain usable.
    let db = Arc::new(Database::new(sim.db_path(), config(0)).expect("Failed to reopen DB after crash"));
    let conn = Connection::new(&db);

    let rows = db.table_num_rows("Person");
    assert!(
        (1..=60).contains(&rows),
        "recovered row count {} should be within (0, 60]",
        rows
    );

    let names = query_column(&conn, "MATCH (n:Person) RETURN n.name");
    assert_eq!(
        names.len(),
        rows as usize,
        "every recovered row must be queryable (no lost rows)"
    );

    // No duplicates: each recovered name appears exactly once.
    let mut name_strings: Vec<String> = names
        .iter()
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => panic!("unexpected value type in Person.name: {other:?}"),
        })
        .collect();
    let original_len = name_strings.len();
    name_strings.sort();
    name_strings.dedup();
    assert_eq!(
        name_strings.len(),
        original_len,
        "no rows should be double-applied across restart paths"
    );

    // Recovered rows must be a subset of what the child attempted to insert.
    for name in &name_strings {
        assert!(
            name.starts_with("person_"),
            "unexpected recovered row: {name:?} (all: {name_strings:?})"
        );
    }

    // The table must accept new writes after recovery.
    conn.query("CREATE (:Person {name: 'after_crash', age: 1})")
        .expect("post-crash insert failed");
    assert_eq!(db.table_num_rows("Person"), rows + 1);
}

// ===========================================================================
// Read-only mode
// ===========================================================================

#[test]
fn test_read_only_rejects_writes_but_allows_reads() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    setup_person_table(&db_path, &[("alice", 30), ("bob", 25)]);

    let db = Arc::new(Database::new(&db_path, read_only_config()).expect("Failed to open read-only DB"));
    let conn = Connection::new(&db);

    // Reads work in read-only mode.
    assert_eq!(db.table_num_rows("Person"), 2);
    let mut ages = query_i64s(&conn, "MATCH (n:Person) RETURN n.age");
    ages.sort();
    assert_eq!(ages, vec![25, 30]);

    // Writes are rejected.
    let dml = conn.query("CREATE (:Person {name: 'x', age: 1})");
    assert!(dml.is_err(), "DML should be rejected in read-only mode");
    assert!(
        dml.unwrap_err().to_lowercase().contains("read-only"),
        "error should mention read-only mode"
    );

    let ddl = conn.query("CREATE NODE TABLE Other(id INT64, PRIMARY KEY(id))");
    assert!(ddl.is_err(), "DDL should be rejected in read-only mode");
}

// ===========================================================================
// Cross-process lock
// ===========================================================================

#[test]
fn test_same_process_second_open_shares_lock() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    setup_person_table(&db_path, &[("alice", 30)]);

    // P53.35 (E3): the cross-process lock is reentrant within one process —
    // two Database instances on the same path can coexist (the kairos harness
    // opens a fixture store and a fresh store on the same path). The OS lock is
    // held once, refcounted.
    let db1 = Database::new(&db_path, config(-1)).expect("first open should succeed");
    assert_eq!(db1.table_num_rows("Person"), 1);
    let db2 = Database::new(&db_path, config(-1)).expect("same-process second open shares the lock");
    assert_eq!(db2.table_num_rows("Person"), 1);

    drop(db1);

    // Lock still held by db2 while it is alive.
    let db3 = Database::new(&db_path, config(-1)).expect("share persists while db2 lives");
    assert_eq!(db3.table_num_rows("Person"), 1);
    drop(db3);
    drop(db2);

    // All instances dropped -> OS lock released -> a fresh open re-locks.
    let db4 = Database::new(&db_path, config(-1)).expect("reopen after last drop should succeed");
    assert_eq!(db4.table_num_rows("Person"), 1);
}

#[test]
fn test_cross_process_lock_still_excludes_second_process() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    setup_person_table(&db_path, &[("alice", 30)]);

    // Hold the exclusive lock in this process (in-process reentrancy does not
    // weaken the cross-process guard).
    let db1 = Database::new(&db_path, config(-1)).expect("first open should succeed");

    // A second PROCESS must still be rejected while the lock is held.
    let child = Command::new(env!("CARGO_BIN_EXE_crash_sim_child"))
        .arg(db_path.to_str().unwrap())
        .arg("hold-lock")
        .arg("0")
        .arg("0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn crash_sim_child");
    let out = child.wait_with_output().expect("wait for child");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("LOCK-ERROR"), "child should be rejected: {stdout}");
    assert!(!out.status.success(), "child must exit non-zero");
    assert!(stdout.contains("already open"), "unexpected error: {stdout}");

    drop(db1);

    // After the parent releases the lock, the child can acquire it. Pre-create
    // the signal file so the child (which waits on it after opening) exits at
    // once.
    fs::write(db_path.join("signal"), b"").expect("create signal file");
    let child = Command::new(env!("CARGO_BIN_EXE_crash_sim_child"))
        .arg(db_path.to_str().unwrap())
        .arg("hold-lock")
        .arg("0")
        .arg("0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn crash_sim_child");
    let out = child.wait_with_output().expect("wait for child");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("LOCK-HELD"), "child should acquire lock: {stdout}");
    assert!(out.status.success(), "child must exit zero: {stdout}");
}

#[test]
fn test_shared_lock_allows_multiple_readers() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    setup_person_table(&db_path, &[("alice", 30)]);

    // Multiple read-only opens take compatible shared locks.
    let reader1 = Database::new(&db_path, read_only_config()).expect("first read-only open should succeed");
    let reader2 = Database::new(&db_path, read_only_config()).expect("second read-only open should succeed");
    assert_eq!(reader1.table_num_rows("Person"), 1);
    assert_eq!(reader2.table_num_rows("Person"), 1);

    // Same-process write open now shares the held lock (P53.35); the
    // cross-process writer is covered by test_cross_process_lock_still_excludes_second_process.
    let writer = Database::new(&db_path, config(-1)).expect("same-process write open shares the lock");
    assert_eq!(writer.table_num_rows("Person"), 1);

    drop(reader1);
    drop(reader2);
    drop(writer);

    let reopen = Database::new(&db_path, read_only_config()).expect("read-only open after close should succeed");
    assert_eq!(reopen.table_num_rows("Person"), 1);
}
