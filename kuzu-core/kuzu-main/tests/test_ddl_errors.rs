use kuzu_main::{Connection, Database, SystemConfig};

fn setup_db() -> (std::sync::Arc<Database>, Connection) {
    let db = std::sync::Arc::new(Database::new(":memory:", SystemConfig::default()).unwrap());
    let conn = Connection::new(&db);
    (db, conn)
}

#[test]
fn test_ddl_errors() {
    let (_db, conn) = setup_db();
    
    // Create table should succeed
    let res = conn.query("CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))").unwrap();
    assert!(res.is_success());
    
    // Create same table should fail
    let res = conn.query("CREATE NODE TABLE Person(id INT64, PRIMARY KEY (id))").unwrap();
    assert!(!res.is_success());
    let err_msg1 = res.error_message.unwrap();
    assert!(err_msg1.contains("already exists") || err_msg1.contains("AlreadyExists"), "Expected already exists error, got: {:?}", err_msg1);
    
    // Drop non-existent table should fail
    let res = conn.query("DROP TABLE NonExistent").unwrap();
    assert!(!res.is_success());
    let err_msg2 = res.error_message.unwrap();
    assert!(err_msg2.contains("does not exist") || err_msg2.contains("NotFound"), "Expected not found error, got: {:?}", err_msg2);
}
