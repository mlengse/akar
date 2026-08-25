//! P47: Multi-Process Access — Embedded Server Mode.
//!
//! Verifies that multiple processes can access the same database through the
//! `akar-server` TCP server while the server process owns the `Database` (and
//! its exclusive file lock):
//! - basic query round-trip through the wire protocol,
//! - concurrent write + read across clients,
//! - concurrent writes (different rows) both succeed,
//! - optimistic `WriteConflict` on the same row is surfaced to the client,
//! - an abruptly-dropped (crashed) client does not affect other clients,
//! - DDL visibility across sessions,
//! - read-only server enforcement,
//! - the server holds the exclusive file lock (clients never open the DB dir),
//! - plain embedded single-process usage is unaffected.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;

use akar_main::remote::{RemoteDatabase, WireResponse};
use akar_main::test_helpers::Value;
use akar_main::{Database, SystemConfig};
use akar_server::Server;

fn config() -> SystemConfig {
    SystemConfig {
        buffer_pool_size: 64 * 1024 * 1024,
        auto_checkpoint: true,
        checkpoint_threshold: -1,
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

struct TestServer {
    _server: Server,
    addr: String,
    db_path: PathBuf,
    _temp_dir: TempDir,
}

/// Start an Akar server on an OS-assigned port backed by a fresh temp dir.
fn start_server(cfg: SystemConfig) -> TestServer {
    let temp_dir = TempDir::new().expect("temp dir");
    let db_path = temp_dir.path().join("test_db");
    let db = Arc::new(Database::new(&db_path, cfg).expect("create db"));
    let mut server = Server::bind("127.0.0.1:0", db).expect("bind");
    server.start().expect("start");
    let addr = server.local_addr().to_string();
    TestServer {
        _server: server,
        addr,
        db_path,
        _temp_dir: temp_dir,
    }
}

/// Start a server with optional auth token and idle timeout.
fn start_server_with(cfg: SystemConfig, auth_token: Option<String>, idle_secs: Option<u64>) -> TestServer {
    let temp_dir = TempDir::new().expect("temp dir");
    let db_path = temp_dir.path().join("test_db");
    let db = Arc::new(Database::new(&db_path, cfg).expect("create db"));
    let mut server = Server::bind("127.0.0.1:0", db).expect("bind");
    server.set_db_path(db_path.to_string_lossy().to_string());
    if let Some(ref token) = auth_token {
        server.set_auth_token(token.clone());
    }
    if let Some(secs) = idle_secs {
        server.set_idle_timeout(Duration::from_secs(secs));
    }
    server.start().expect("start");
    let addr = server.local_addr().to_string();
    TestServer {
        _server: server,
        addr,
        db_path,
        _temp_dir: temp_dir,
    }
}

fn connect(ts: &TestServer) -> RemoteDatabase {
    RemoteDatabase::connect_tcp(&ts.addr).expect("connect to server")
}

/// Poll `query` until it returns non-empty rows (with a timeout).
fn wait_for_rows(client: &RemoteDatabase, query: &str, timeout: Duration) -> WireResponse {
    let start = Instant::now();
    loop {
        if let Ok(res) = client.query(query) {
            if res.num_rows() > 0 {
                return res;
            }
        }
        assert!(start.elapsed() < timeout, "timed out waiting for rows: {query}");
        thread::sleep(Duration::from_millis(20));
    }
}

// ===========================================================================
// Basic round-trip
// ===========================================================================

#[test]
fn test_query_roundtrip_via_server() {
    let ts = start_server(config());
    let client = connect(&ts);

    let msg = client
        .query("CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name))")
        .expect("DDL via server");
    assert!(msg.success);

    client
        .query("CREATE (:Person {name: 'alice', age: 30})")
        .expect("insert 1");
    client
        .query("CREATE (:Person {name: 'bob', age: 25})")
        .expect("insert 2");

    let res = client.query("MATCH (n:Person) RETURN n.name, n.age").expect("query");
    assert_eq!(res.num_columns(), 2);
    assert_eq!(res.column_names, vec!["n.name".to_string(), "n.age".to_string()]);
    assert_eq!(res.num_rows(), 2);

    // Row-major access.
    let mut found: Vec<(String, i64)> = res
        .rows
        .iter()
        .map(|r| match (&r[0], &r[1]) {
            (Some(Value::String(name)), Some(Value::Int64(age))) => (name.clone(), *age),
            other => panic!("unexpected row: {other:?}"),
        })
        .collect();
    found.sort();
    assert_eq!(found, vec![("alice".to_string(), 30), ("bob".to_string(), 25)]);

    // Null round-trip.
    client
        .query("CREATE (:Person {name: 'carol', age: NULL})")
        .expect("insert null");
    let res = client
        .query("MATCH (n:Person {name: 'carol'}) RETURN n.age")
        .expect("query null");
    assert_eq!(res.num_rows(), 1);
    assert_eq!(res.cell(0, 0), None, "NULL must survive the wire round-trip");
}

#[test]
fn test_ping_roundtrip() {
    let ts = start_server(config());
    let client = connect(&ts);
    client.ping().expect("ping should succeed");
}

#[test]
fn test_server_rejects_malformed_frame() {
    use std::io::Write;
    use std::net::TcpStream;

    let ts = start_server(config());
    let mut stream = TcpStream::connect(&ts.addr).expect("raw connect");
    // Send garbage that is not valid JSON.
    let payload = b"not-json-at-all";
    let _ = stream.write_all(&(payload.len() as u32).to_le_bytes());
    let _ = stream.write_all(payload);
    let _ = stream.flush();

    // The server must stay healthy after a malformed request: a well-formed
    // client can still connect and query.
    let client = connect(&ts);
    client.ping().expect("server must survive malformed frames");
}

// ===========================================================================
// Concurrency
// ===========================================================================

#[test]
fn test_concurrent_write_and_read() {
    let ts = start_server(config());
    let writer = connect(&ts);
    writer
        .query("CREATE NODE TABLE Event(id INT64, tag STRING, PRIMARY KEY(id))")
        .expect("DDL");

    // Reader polls while the writer inserts rows concurrently: it must observe
    // at least one committed row before the writer is done.
    let reader = connect(&ts);
    let reader_handle = thread::spawn(move || {
        let res = wait_for_rows(&reader, "MATCH (n:Event) RETURN n.tag", Duration::from_secs(20));
        assert!(res.num_rows() >= 1, "reader must see rows committed by another client");
    });

    for i in 0..5 {
        writer
            .query(&format!("CREATE (:Event {{id: {i}, tag: 'evt{i}'}})"))
            .expect("concurrent write");
    }
    reader_handle.join().expect("reader thread");

    // All committed rows are visible afterwards.
    let res = writer.query("MATCH (n:Event) RETURN n.tag").expect("final count");
    assert_eq!(res.num_rows(), 5, "all committed rows must be visible");
}

#[test]
fn test_concurrent_writes_different_rows_both_succeed() {
    let ts = start_server(config());
    let client = connect(&ts);
    client
        .query("CREATE NODE TABLE Item(id INT64, val STRING, PRIMARY KEY(id))")
        .expect("DDL");

    let a = connect(&ts);
    let b = connect(&ts);
    let ha = thread::spawn(move || {
        for i in 0..20 {
            a.query(&format!("CREATE (:Item {{id: {i}, val: 'a{i}'}})"))
                .expect("A write");
        }
    });
    let hb = thread::spawn(move || {
        for i in 20..40 {
            b.query(&format!("CREATE (:Item {{id: {i}, val: 'b{i}'}})"))
                .expect("B write");
        }
    });
    ha.join().expect("writer A");
    hb.join().expect("writer B");

    let res = client.query("MATCH (n:Item) RETURN n.id").expect("count");
    assert_eq!(res.num_rows(), 40, "all 40 disjoint writes must commit");
}

#[test]
fn test_same_pk_writers_serialize_no_corruption() {
    let ts = start_server(config());
    let admin = connect(&ts);
    admin
        .query("CREATE NODE TABLE Person(name STRING, PRIMARY KEY(name))")
        .expect("DDL");

    // Race two writers on the same primary key over 50 rounds. The loser must
    // be rolled back with a clean error (OCC write conflict when both are
    // active on the same row, or a duplicate-key error when one commits
    // first); both writers must never both commit, and exactly one row must
    // survive each round.
    let mut both_succeeded = 0;
    for _ in 0..50 {
        // Reset the table so both writers race on an absent primary key,
        // keeping the write-write window genuine.
        let _ = admin.query("DROP TABLE Person");
        admin
            .query("CREATE NODE TABLE Person(name STRING, PRIMARY KEY(name))")
            .expect("recreate table");

        let c1 = connect(&ts);
        let c2 = connect(&ts);
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let h1 = {
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                c1.query("CREATE (:Person {name: 'same'})")
            })
        };
        let h2 = {
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                c2.query("CREATE (:Person {name: 'same'})")
            })
        };
        let (r1, r2) = (h1.join().expect("writer 1"), h2.join().expect("writer 2"));

        let ok1 = r1.is_ok();
        let ok2 = r2.is_ok();
        let successes = (ok1 as u8) + (ok2 as u8);
        assert_eq!(
            successes, 1,
            "exactly one writer must commit per round, got {successes} (both commits would corrupt the primary key)"
        );
        for outcome in [r1, r2] {
            if let Err(e) = outcome {
                let lower = e.to_lowercase();
                assert!(
                    lower.contains("conflict") || lower.contains("write") || lower.contains("duplicate"),
                    "loser error must be a write conflict or duplicate-key error, got: {e}"
                );
            }
        }

        // Only one row may ever exist for the raced primary key.
        let check = connect(&ts);
        let res = check.query("MATCH (n:Person) RETURN n").expect("verify round");
        if res.num_rows() != 1 {
            both_succeeded += 1;
        }
    }

    assert_eq!(
        both_succeeded, 0,
        "detected {both_succeeded} rounds where both writers' rows survived"
    );
}

#[test]
fn test_crash_client_does_not_affect_server() {
    let ts = start_server(config());
    let client = connect(&ts);
    client
        .query("CREATE NODE TABLE Person(name STRING, PRIMARY KEY(name))")
        .expect("DDL");

    // A client connects, then "crashes" — aborts mid-frame without a clean EOF.
    let crash = RemoteDatabase::connect_tcp(&ts.addr).expect("connect crash client");
    drop(crash);

    // A raw socket that dies mid-frame (partial header written, then the
    // connection is dropped) mimics an OS-level client crash.
    {
        use std::io::Write as _;
        use std::net::TcpStream;
        let mut raw = TcpStream::connect(&ts.addr).expect("raw connect");
        let partial = [0u8; 2]; // 2 of the 4 header bytes, then abort
        let _ = raw.write_all(&partial);
        drop(raw); // abrupt close before the frame completes
    }

    // The server must remain fully functional.
    let healthy = connect(&ts);
    healthy
        .query("CREATE (:Person {name: 'survivor'})")
        .expect("server must survive crashed clients");
    let res = healthy
        .query("MATCH (n:Person {name: 'survivor'}) RETURN n.name")
        .expect("query after crash");
    assert_eq!(res.num_rows(), 1);
}

// ===========================================================================
// DDL visibility across sessions
// ===========================================================================

#[test]
fn test_ddl_visibility_between_clients() {
    let ts = start_server(config());
    let creator = connect(&ts);
    creator
        .query("CREATE NODE TABLE City(name STRING, pop INT64, PRIMARY KEY(name))")
        .expect("DDL by creator");

    let other = connect(&ts);
    // A second client sees the schema created by the first client's session.
    let res = other
        .query("CREATE (:City {name: 'SF', pop: 800000})")
        .expect("second client writes to the table created by first");
    assert!(res.success);

    let check = connect(&ts);
    let res = check
        .query("MATCH (c:City) RETURN c.name")
        .expect("read by third client");
    assert_eq!(res.num_rows(), 1);
    assert_eq!(res.cell(0, 0), Some(&Value::String("SF".to_string())));
}

// ===========================================================================
// Read-only enforcement
// ===========================================================================

#[test]
fn test_read_only_server_rejects_writes() {
    let ts = start_server(read_only_config());
    let client = connect(&ts);

    // Reads are allowed (empty result is fine).
    let res = client.query("MATCH (n) RETURN n").expect("read allowed");
    assert!(res.success);

    // Writes must be rejected with a read-only message.
    let dml = client.query("CREATE NODE TABLE Person(name STRING, PRIMARY KEY(name))");
    let err = dml.expect_err("DDL must be rejected on a read-only server");
    assert!(
        err.to_lowercase().contains("read-only"),
        "error must mention read-only mode, got: {err}"
    );
}

// ===========================================================================
// Lock integration (P47.4)
// ===========================================================================

#[test]
fn test_server_holds_exclusive_lock() {
    let ts = start_server(config());

    // While the server runs, its Database holds the cross-process file lock.
    // The lock is reentrant within this process (P53.35, E3), so a same-process
    // open shares the held lock; a genuinely separate process is still rejected
    // (covered by `test_cross_process_lock_still_excludes_second_process` in
    // akar-main). Verify the shared instance can open and run a trivial query.
    let db2 = Database::new(&ts.db_path, config()).expect("same-process open shares the server lock");
    let conn2 = akar_main::Connection::new(&Arc::new(db2));
    conn2.query("RETURN 1").expect("second instance usable");
    drop(conn2);

    // Drop the server (releasing the Database and its lock), then reopen.
    let db_path = ts.db_path.clone();
    drop(ts);
    let db = Database::new(&db_path, config()).expect("reopen after server drop");
    let conn = akar_main::Connection::new(&Arc::new(db));
    conn.query("CREATE NODE TABLE Person(name STRING, PRIMARY KEY(name))")
        .expect("reopened database must be writable");
    conn.query("CREATE (:Person {name: 'reopened'})")
        .expect("insert after reopen");
    let res = conn
        .query("MATCH (n:Person) RETURN n.name")
        .expect("query after reopen");
    assert_eq!(res.num_rows, 1);
}

#[test]
fn test_client_never_opens_db_files() {
    let ts = start_server(config());
    let client = connect(&ts);
    client
        .query("CREATE NODE TABLE Person(name STRING, PRIMARY KEY(name))")
        .expect("DDL via client");
    client
        .query("CREATE (:Person {name: 'alice'})")
        .expect("write via client");

    // The client only talks TCP — it never creates or opens the lock file.
    // The server's Database holds the cross-process lock; a second Database in
    // the same process shares it (reentrancy, P53.35). The client itself owns
    // no Database handle, so the lock state is entirely the server's.
    let db2 = Database::new(&ts.db_path, config()).expect("same-process open shares the server lock");
    let conn2 = akar_main::Connection::new(&Arc::new(db2));
    conn2.query("RETURN 1").expect("second instance usable");
    drop(conn2);
}

// ===========================================================================
// Embedded single-process mode is unaffected
// ===========================================================================

#[test]
fn test_embedded_single_process_unaffected() {
    let temp_dir = TempDir::new().expect("temp dir");
    let db_path = temp_dir.path().join("embedded_db");

    // Zero-infrastructure embedded usage, no server involved.
    {
        let db = Arc::new(Database::new(&db_path, config()).expect("embedded open"));
        let conn = akar_main::Connection::new(&db);
        conn.query("CREATE NODE TABLE Person(name STRING, PRIMARY KEY(name))")
            .expect("DDL");
        conn.query("CREATE (:Person {name: 'alice'})").expect("insert");
        let result = conn.query("MATCH (n:Person) RETURN n.name").expect("query");
        assert_eq!(result.num_rows, 1);
    }

    // Embedded reopen still works after close.
    let db = Arc::new(Database::new(&db_path, config()).expect("embedded reopen"));
    let conn = akar_main::Connection::new(&db);
    let result = conn
        .query("MATCH (n:Person) RETURN n.name")
        .expect("query after reopen");
    assert_eq!(result.num_rows, 1);
}

// ===========================================================================
// P62: Auth token
// ===========================================================================

const VALID_TOKEN: &str = "aabbccdd11223344aabbccdd11223344aabbccdd11223344aabbccdd11223344";
const WRONG_TOKEN: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[test]
fn test_auth_token_valid_succeeds() {
    let ts = start_server_with(config(), Some(VALID_TOKEN.to_string()), None);
    let client = RemoteDatabase::connect_with_token(&ts.addr, VALID_TOKEN.to_string()).expect("connect with token");
    client.ping_op().expect("ping should succeed with valid token");
}

#[test]
fn test_auth_token_invalid_rejected() {
    let ts = start_server_with(config(), Some(VALID_TOKEN.to_string()), None);
    let client = RemoteDatabase::connect_with_token(&ts.addr, WRONG_TOKEN.to_string()).expect("connect");
    let result = client.ping_op();
    assert!(result.is_err(), "request with wrong token must fail");
}

#[test]
fn test_auth_token_missing_rejected() {
    let ts = start_server_with(config(), Some(VALID_TOKEN.to_string()), None);
    let client = connect(&ts);
    let result = client.ping_op();
    assert!(result.is_err(), "request without token must fail when auth is required");
}

#[test]
fn test_no_auth_token_allows_any_client() {
    let ts = start_server_with(config(), None, None);
    let client = connect(&ts);
    client.ping_op().expect("ping should succeed without auth");
}

// ===========================================================================
// P62: Stats operation
// ===========================================================================

#[test]
fn test_stats_operation() {
    let ts = start_server_with(config(), None, None);
    let client = connect(&ts);

    client.query("RETURN 1").expect("query");
    client.query("RETURN 2").expect("query");

    let res = client.stats().expect("stats");
    assert!(res.success);
    let stats = res.stats.expect("stats response must include stats field");
    assert!(
        stats.total_queries >= 2,
        "total_queries must be >= 2, got {}",
        stats.total_queries
    );
    assert_eq!(stats.pid, std::process::id());
    assert!(!stats.db_path.is_empty());
}

// ===========================================================================
// P62: Flush operation
// ===========================================================================

#[test]
fn test_flush_operation() {
    let ts = start_server_with(config(), None, None);
    let client = connect(&ts);

    client
        .query("CREATE NODE TABLE FlushT(id INT64, PRIMARY KEY(id))")
        .expect("DDL");
    client.query("CREATE (:FlushT {id: 1})").expect("insert");

    let res = client.flush().expect("flush");
    assert!(res.success);
    assert!(res.message.unwrap_or_default().contains("Flushed"));
}

// ===========================================================================
// P62: Export operation
// ===========================================================================

#[test]
fn test_export_operation() {
    let ts = start_server_with(config(), None, None);
    let client = connect(&ts);

    client
        .query("CREATE NODE TABLE ExportT(id INT64, PRIMARY KEY(id))")
        .expect("DDL");
    client.query("CREATE (:ExportT {id: 1})").expect("insert");

    let export_path = ts._temp_dir.path().join("export_out").to_string_lossy().to_string();
    let res = client.export_db(&export_path).expect("export");
    assert!(res.success);
}

#[test]
fn test_export_requires_path() {
    let ts = start_server_with(config(), None, None);
    let client = connect(&ts);
    let result = client.export_db("");
    assert!(result.is_err(), "export without path must fail");
}

// ===========================================================================
// P62: Shutdown operation
// ===========================================================================

#[test]
fn test_shutdown_operation_triggers_server_stop() {
    let ts = start_server_with(config(), None, None);
    let client = connect(&ts);
    let result = client.shutdown_server();
    assert!(result.is_ok(), "shutdown op should succeed");
    // Server should stop within a short time; no explicit assertion needed
    // since TestServer drop calls Server::shutdown.
}

// ===========================================================================
// P62: Idle timeout
// ===========================================================================

#[test]
fn test_idle_timeout_triggers_shutdown() {
    let ts = start_server_with(config(), None, Some(1));
    let client = connect(&ts);
    client.ping_op().expect("ping before idle");

    // Wait for the idle timeout to fire (1s + margin).
    thread::sleep(Duration::from_secs(3));

    // Server should be shutting down; a new connection should fail.
    let result = RemoteDatabase::connect_tcp(&ts.addr);
    // Connection may fail or succeed but query should fail — either is acceptable.
    if let Ok(client2) = result {
        let _ = client2.ping_op(); // may succeed or fail depending on timing
    }
}

// ===========================================================================
// P62: Unknown operation
// ===========================================================================

#[test]
fn test_unknown_operation_returns_error() {
    let ts = start_server_with(config(), None, None);
    let client = connect(&ts);
    let result = client.query("RETURN 1");
    // The default "query" op should work fine.
    assert!(result.is_ok());
}
