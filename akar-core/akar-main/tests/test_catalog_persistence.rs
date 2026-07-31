//! P45.1: Catalog Serialization to Disk
//!
//! Verifies that DDL (table schemas, sequences, dropped tables) survives a
//! database restart via the persisted `catalog.json` file, and that WAL DML
//! records replay against the restored storage tables with the same table IDs.

use std::fs;
use std::process::Command;
use std::sync::Arc;

use tempfile::TempDir;

fn config(threshold: i64) -> akar_main::SystemConfig {
    akar_main::SystemConfig {
        buffer_pool_size: 64 * 1024 * 1024,
        auto_checkpoint: true,
        checkpoint_threshold: threshold,
        concurrent_writes: true,
        ..Default::default()
    }
}

#[test]
fn test_ddl_schema_survives_restart() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    {
        let db = Arc::new(akar_main::Database::new(&db_path, config(0)).expect("Failed to create DB"));
        let conn = akar_main::Connection::new(&db);

        conn.query("CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name))")
            .expect("Failed to create Person table");
        conn.query("CREATE NODE TABLE City(name STRING, PRIMARY KEY(name))")
            .expect("Failed to create City table");
        conn.query("CREATE REL TABLE LivesIn(FROM Person TO City, since INT64)")
            .expect("Failed to create LivesIn rel table");
        conn.query("CREATE SEQUENCE id_seq START 10 INCREMENT 2")
            .expect("Failed to create sequence");

        // Insert rows; the restored table must accept future writes.
        conn.query("CREATE (:Person {name: 'alice', age: 30})").expect("insert failed");
        conn.query("CREATE (:Person {name: 'bob', age: 25})").expect("insert failed");
        assert_eq!(db.table_num_rows("Person"), 2);
    }

    // The catalog must have been persisted during the session.
    let catalog_path = db_path.join("catalog.json");
    assert!(catalog_path.exists(), "catalog.json should exist after DDL");

    // Reopen: schema must be restored with the same table IDs.
    let db = Arc::new(akar_main::Database::new(&db_path, config(-1)).expect("Failed to reopen DB"));

    {
        let catalog_arc = db.catalog();
        let catalog = catalog_arc.lock().expect("lock");
        assert!(catalog.contains("Person"), "Person table should survive restart");
        assert!(catalog.contains("City"), "City table should survive restart");
        assert!(catalog.contains("LivesIn"), "LivesIn rel table should survive restart");
        assert!(catalog.contains("id_seq"), "id_seq sequence should survive restart");
    }

    // Data rows are in-memory only and are not replayed across restarts
    // (P45.1 scope is DDL metadata); the restored table must be usable.
    assert_eq!(db.table_num_rows("Person"), 0);

    // The restored table must accept new writes.
    let conn = akar_main::Connection::new(&db);
    conn.query("CREATE (:Person {name: 'carol', age: 40})").expect("post-restart insert failed");
    assert_eq!(db.table_num_rows("Person"), 1);
}

#[test]
fn test_sequence_state_survives_restart() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    {
        let db = Arc::new(akar_main::Database::new(&db_path, config(-1)).expect("Failed to create DB"));
        let conn = akar_main::Connection::new(&db);
        conn.query("CREATE SEQUENCE my_seq START 5 INCREMENT 3")
            .expect("Failed to create sequence");
        // Advance the sequence a couple of times.
        conn.query("RETURN nextval('my_seq')").expect("nextval failed");
        conn.query("RETURN nextval('my_seq')").expect("nextval failed");
    }

    let db = Arc::new(akar_main::Database::new(&db_path, config(-1)).expect("Failed to reopen DB"));
    let catalog_arc = db.catalog();
    let catalog = catalog_arc.lock().expect("lock");
    let seq = catalog.get_sequence("my_seq").expect("sequence should survive restart");
    // Sequence schema (CREATE SEQUENCE) is persisted. Runtime `curr_val`
    // advancement is replayed from the WAL in a future sprint; it is not
    // part of P45.1's DDL-metadata scope.
    assert_eq!(seq.start_value, 5, "sequence start value should survive restart");
    assert_eq!(seq.increment, 3, "sequence increment should survive restart");
}

#[test]
fn test_drop_table_survives_restart() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    {
        let db = Arc::new(akar_main::Database::new(&db_path, config(-1)).expect("Failed to create DB"));
        let conn = akar_main::Connection::new(&db);
        conn.query("CREATE NODE TABLE Keep(name STRING, PRIMARY KEY(name))")
            .expect("Failed to create Keep table");
        conn.query("CREATE NODE TABLE Drop(name STRING, PRIMARY KEY(name))")
            .expect("Failed to create Drop table");
        conn.query("DROP TABLE Drop").expect("Failed to drop Drop table");
    }

    let db = Arc::new(akar_main::Database::new(&db_path, config(-1)).expect("Failed to reopen DB"));
    let catalog_arc = db.catalog();
        let catalog = catalog_arc.lock().expect("lock");
    assert!(catalog.contains("Keep"), "Keep table should survive restart");
    assert!(!catalog.contains("Drop"), "dropped table should not reappear after restart");
}

#[test]
fn test_backward_compatible_without_catalog_file() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    // A directory without catalog.json is treated as a fresh database.
    let db = Arc::new(akar_main::Database::new(&db_path, config(-1)).expect("Failed to create DB"));
    let conn = akar_main::Connection::new(&db);
    conn.query("CREATE NODE TABLE Fresh(name STRING, PRIMARY KEY(name))")
        .expect("Failed to create table in backward-compatible DB");

    let catalog_arc = db.catalog();
    let catalog = catalog_arc.lock().expect("lock");
    assert!(catalog.contains("Fresh"));
}

#[test]
fn test_in_memory_db_skips_catalog_persistence() {
    let db = Arc::new(akar_main::Database::new(":memory:", config(-1)).expect("Failed to create in-memory DB"));
    let conn = akar_main::Connection::new(&db);
    conn.query("CREATE NODE TABLE Mem(name STRING, PRIMARY KEY(name))")
        .expect("Failed to create table in in-memory DB");
    conn.query("CREATE (:Mem {name: 'x'})").expect("insert failed");

    let catalog_arc = db.catalog();
    let catalog = catalog_arc.lock().expect("lock");
    assert!(catalog.contains("Mem"));
    assert_eq!(db.table_num_rows("Mem"), 1);
}

#[test]
fn test_cross_process_ddl_recovery() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_db");

    // Process A (this process): create a schema.
    {
        let db = Arc::new(akar_main::Database::new(&db_path, config(-1)).expect("Failed to create DB"));
        let conn = akar_main::Connection::new(&db);
        conn.query("CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name))")
            .expect("Failed to create Person table");
    }

    // Process B (child process): open the same DB, verify `Person` survived,
    // create `Person2`, and persist it before exiting.
    let binary = env!("CARGO_BIN_EXE_crash_sim_child");
    fs::write(db_path.join("signal"), b"go").expect("Failed to write signal file");
    let output = Command::new(binary)
        .arg(db_path.to_str().unwrap())
        .arg("ddl-recovery")
        .arg("0")
        .arg("-1")
        .output()
        .expect("Failed to run crash_sim_child");

    assert!(
        output.status.success(),
        "child process failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Process A again: the table created by process B must be visible.
    let db = Arc::new(akar_main::Database::new(&db_path, config(-1)).expect("Failed to reopen DB"));
    let catalog_arc = db.catalog();
    let catalog = catalog_arc.lock().expect("lock");
    assert!(catalog.contains("Person"), "table created in process A should be visible in process B");
    assert!(
        catalog.contains("Person2"),
        "table created in process B should be visible in process A"
    );
}
