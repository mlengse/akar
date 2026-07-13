use kuzu_main::{Connection, Database, SystemConfig};
use std::sync::Arc;
use std::thread;

fn setup_db() -> (Arc<Database>, Connection) {
    let db = Arc::new(Database::new(":memory:", SystemConfig::default()).unwrap());
    let conn = Connection::new(&db);
    (db, conn)
}

fn exec(conn: &Connection, query: &str) -> String {
    let result = conn.query(query).unwrap();
    assert!(
        result.is_success(),
        "Query failed: {query} → {:?}",
        result.error_message
    );
    result.to_string()
}

#[test]
fn test_concurrent_reads() {
    let (db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, name STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE (p:Person {id: 1, name: 'Alice'})");
    exec(&conn, "CREATE (p:Person {id: 2, name: 'Bob'})");
    
    let mut handles = vec![];
    
    for _ in 0..10 {
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
