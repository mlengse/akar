mod common;
use common::{exec, setup_db};

#[test]
fn test_insert_and_read_mvcc() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, name STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, name: 'Alice'})");
    exec(&conn, "CREATE (p:Person {id: 2, name: 'Bob'})");
    let res = conn.query("MATCH (p:Person) RETURN p.id").unwrap();
    assert!(res.is_success());
    assert_eq!(res.num_rows(), 2);
}

#[test]
fn test_concurrent_read_snapshot() {
    let (db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Counter(id INT64, val INT64, PRIMARY KEY (id))");
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
