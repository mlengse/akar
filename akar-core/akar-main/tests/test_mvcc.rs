mod common;
use akar_main::test_helpers::Value;
use akar_main::Connection;
use common::{exec, setup_db};

#[test]
fn test_insert_and_read_mvcc() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(id INT64, name STRING, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (p:Person {id: 1, name: 'Alice'})");
    exec(&conn, "CREATE (p:Person {id: 2, name: 'Bob'})");
    let res = conn.query("MATCH (p:Person) RETURN p.id").unwrap();
    assert!(res.is_success());
    assert_eq!(res.num_rows(), 2);
}

#[test]
fn test_concurrent_read_snapshot() {
    let (db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Counter(id INT64, val INT64, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (c:Counter {id: 1, val: 0})");
    let db_clone = std::sync::Arc::clone(&db);
    let handle = std::thread::spawn(move || {
        let thread_conn = common::Connection::new(&db_clone);
        let res = thread_conn.query("MATCH (c:Counter) RETURN c.id").unwrap();
        assert!(res.is_success());
        assert!(res.num_rows() >= 1);
    });
    handle.join().unwrap();
    let res = conn.query("MATCH (c:Counter) RETURN c.id").unwrap();
    assert!(res.is_success());
}

#[test]
fn test_multiple_sequential_writes() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, name STRING, PRIMARY KEY (id))");
    for i in 0..10 {
        exec(&conn, &format!("CREATE (t:T {{id: {i}, name: 'n{i}'}})"));
    }
    let res = conn.query("MATCH (t:T) RETURN t.id").unwrap();
    assert!(res.is_success());
    assert_eq!(res.num_rows(), 10);
}

#[test]
fn test_write_then_read_same_connection() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Log(id INT64, msg STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE (l:Log {id: 1, msg: 'first'})");
    let res = conn.query("MATCH (l:Log) RETURN l.id").unwrap();
    assert!(res.is_success());
    assert_eq!(res.num_rows(), 1);
}

#[test]
fn test_concurrent_writes_different_connections() {
    let (db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, PRIMARY KEY (id))");
    let mut handles = vec![];
    for i in 0..5 {
        let db_clone = std::sync::Arc::clone(&db);
        handles.push(std::thread::spawn(move || {
            let thread_conn = common::Connection::new(&db_clone);
            let q = format!("CREATE (t:T {{id: {i}}})");
            let res = thread_conn.query(&q).unwrap();
            assert!(res.is_success());
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let res = conn.query("MATCH (t:T) RETURN t.id").unwrap();
    assert!(res.is_success());
    assert_eq!(res.num_rows(), 5);
}

#[test]
fn test_insert_batch_and_read() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Batch(id INT64, val INT64, PRIMARY KEY (id))");
    for i in 0..20 {
        exec(&conn, &format!("CREATE (b:Batch {{id: {i}, val: {i} * 10}})"));
    }
    let res = conn.query("MATCH (b:Batch) RETURN b.id").unwrap();
    assert!(res.is_success());
    assert_eq!(res.num_rows(), 20);
}

/// Read the value column of `CALL current_setting('key')`.
fn current_setting_value(conn: &Connection, key: &str) -> Value {
    let res = conn
        .query(&format!("CALL current_setting('{key}')"))
        .expect("current_setting should succeed");
    res.chunks
        .iter()
        .flat_map(|c| (0..c.size).filter_map(|i| c.get_value(1, i)))
        .next()
        .unwrap_or(Value::Null)
}

#[test]
fn test_set_spill_threshold_zero_disables() {
    let (db, conn) = setup_db();

    conn.query("SET spill_threshold=4096").unwrap();
    assert_eq!(current_setting_value(&conn, "spill_threshold"), Value::String("4096".into()));
    assert_eq!(db.effective_spill_threshold(), 4096);

    // SET spill_threshold=0 must explicitly disable spilling (effective 0),
    // not silently fall back to the config/buffer default (P52.50).
    conn.query("SET spill_threshold=0").unwrap();
    assert_eq!(current_setting_value(&conn, "spill_threshold"), Value::String("0".into()));
    assert_eq!(db.effective_spill_threshold(), 0);
}

#[test]
fn test_set_concurrent_writes_reflects_in_current_setting() {
    let (_db, conn) = setup_db();

    conn.query("SET concurrent_writes=false").unwrap();
    assert_eq!(
        current_setting_value(&conn, "concurrent_writes"),
        Value::String("false".into())
    );

    conn.query("SET concurrent_writes=true").unwrap();
    assert_eq!(
        current_setting_value(&conn, "concurrent_writes"),
        Value::String("true".into())
    );
}
