mod common;
use common::{setup_db, exec};
use kuzu_main::Connection;
use std::sync::Arc;
use std::thread;

#[test]
fn test_concurrent_reads_many_threads() {
    let (db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, name STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, name: 'Alice'})");
    exec(&conn, "CREATE (p:Person {id: 2, name: 'Bob'})");
    
    let mut handles = vec![];
    
    for _ in 0..20 {
        let db_clone = Arc::clone(&db);
        handles.push(thread::spawn(move || {
            let thread_conn = Connection::new(&db_clone);
            let res = thread_conn.query("MATCH (p:Person) RETURN p.name").unwrap();
            assert!(res.is_success());
            assert_eq!(res.num_rows(), 2);
        }));
    }
    
    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
fn test_concurrent_writes() {
    let (db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, name STRING, PRIMARY KEY (id))");
    
    let mut handles = vec![];
    
    for i in 0..10 {
        let db_clone = Arc::clone(&db);
        handles.push(thread::spawn(move || {
            let thread_conn = Connection::new(&db_clone);
            let query = format!("CREATE (p:Person {{id: {}, name: 'Thread{}'}})", i, i);
            let res = thread_conn.query(&query).unwrap();
            assert!(res.is_success());
        }));
    }
    
    for handle in handles {
        handle.join().unwrap();
    }
    
    let res = conn.query("MATCH (p:Person) RETURN p.id").unwrap();
    assert_eq!(res.num_rows(), 10);
}

#[test]
fn test_concurrent_reads_and_writes() {
    let (db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 0})");
    
    let mut handles = vec![];
    
    // Writers
    for i in 1..=5 {
        let db_clone = Arc::clone(&db);
        handles.push(thread::spawn(move || {
            let thread_conn = Connection::new(&db_clone);
            let query = format!("CREATE (p:Person {{id: {}}})", i);
            thread_conn.query(&query).unwrap();
        }));
    }
    
    // Readers
    for _ in 0..5 {
        let db_clone = Arc::clone(&db);
        handles.push(thread::spawn(move || {
            let thread_conn = Connection::new(&db_clone);
            // Just verify query succeeds, number of rows might be anywhere from 1 to 6
            let res = thread_conn.query("MATCH (p:Person) RETURN p.id").unwrap();
            assert!(res.is_success());
            assert!(res.num_rows() >= 1);
        }));
    }
    
    for handle in handles {
        handle.join().unwrap();
    }
    
    let res = conn.query("MATCH (p:Person) RETURN p.id").unwrap();
    assert_eq!(res.num_rows(), 6);
}

#[test]
fn test_many_connections() {
    let (db, _conn) = setup_db();
    let mut connections = vec![];
    for _ in 0..100 {
        connections.push(Connection::new(&db));
    }
    
    exec(&connections[0], "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    
    for i in 1..10 {
        let query = format!("CREATE (p:Person {{id: {}}})", i);
        exec(&connections[i], &query);
    }
    
    let res = connections[99].query("MATCH (p:Person) RETURN p.id").unwrap();
    assert_eq!(res.num_rows(), 9);
}

#[test]
#[ignore = "DDL during DML might not be safely supported or locks the whole DB"]
fn test_concurrent_ddl_and_dml() {
    let (db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    
    let db_clone1 = Arc::clone(&db);
    let h1 = thread::spawn(move || {
        let c = Connection::new(&db_clone1);
        c.query("CREATE NODE TABLE Animal(id INT64, PRIMARY KEY (id))").unwrap();
    });
    
    let db_clone2 = Arc::clone(&db);
    let h2 = thread::spawn(move || {
        let c = Connection::new(&db_clone2);
        c.query("CREATE (p:Person {id: 1})").unwrap();
    });
    
    h1.join().unwrap();
    h2.join().unwrap();
    
    let res1 = conn.query("MATCH (a:Animal) RETURN a.id").unwrap();
    assert!(res1.is_success());
    let res2 = conn.query("MATCH (p:Person) RETURN p.id").unwrap();
    assert_eq!(res2.num_rows(), 1);
}

#[test]
fn test_concurrent_scans_with_different_filters() {
    let (db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))");
    for i in 0..100 {
        exec(&conn, &format!("CREATE (p:Person {{id: {}}})", i));
    }
    
    let mut handles = vec![];
    for i in 0..10 {
        let db_clone = Arc::clone(&db);
        handles.push(thread::spawn(move || {
            let thread_conn = Connection::new(&db_clone);
            let query = format!("MATCH (p:Person) WHERE p.id % 10 = {} RETURN p.id", i);
            let res = thread_conn.query(&query).unwrap();
            assert!(res.is_success());
            assert_eq!(res.num_rows(), 10);
        }));
    }
    
    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
fn test_thread_safe_query_result() {
    let (db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, name STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, name: 'Alice'})");
    exec(&conn, "CREATE (p:Person {id: 2, name: 'Bob'})");
    
    let res = Arc::new(conn.query("MATCH (p:Person) RETURN p.name").unwrap());
    
    let mut handles = vec![];
    for _ in 0..4 {
        let res_clone = Arc::clone(&res);
        handles.push(thread::spawn(move || {
            assert!(res_clone.is_success());
            assert_eq!(res_clone.num_rows(), 2);
        }));
    }
    
    for handle in handles {
        handle.join().unwrap();
    }
}
