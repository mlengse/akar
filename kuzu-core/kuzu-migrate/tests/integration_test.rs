use std::process::Command;
use std::fs;
use tempfile::tempdir;
use kuzu::{Database, SystemConfig, Connection};
use kuzu_main;

#[test]
fn test_migration_e2e() {
    let cpp_dir = tempdir().unwrap();
    let rust_dir = tempdir().unwrap();

    // 1. Create a C++ Kuzu database and populate it
    {
        let db = Database::new(cpp_dir.path(), SystemConfig::default()).unwrap();
        let conn = Connection::new(&db).unwrap();
        
        conn.query("CREATE NODE TABLE User(id INT64, name STRING, PRIMARY KEY(id));").unwrap();
        conn.query("INSERT (u:User {id: 1, name: 'Alice'})").unwrap();
        conn.query("INSERT (u:User {id: 2, name: 'Bob'})").unwrap();
    } // Drop connection and database to flush to disk

    // 2. Run kuzu-migrate
    let status = Command::new("cargo")
        .args(&["run", "-p", "kuzu-migrate", "--", "--from", cpp_dir.path().to_str().unwrap(), "--to", rust_dir.path().to_str().unwrap()])
        .status()
        .expect("Failed to execute kuzu-migrate");

    assert!(status.success(), "kuzu-migrate failed");

    // 3. Verify in Rust Kuzu database
    {
        let db = std::sync::Arc::new(kuzu_main::Database::new(rust_dir.path(), kuzu_main::SystemConfig::default()).unwrap());
        let conn = kuzu_main::Connection::new(&db);
        
        let mut result = conn.query("MATCH (u:User) RETURN u.id, u.name ORDER BY u.id").unwrap();
        
        let row1 = result.next().unwrap();
        assert_eq!(row1[0].to_string(), "1");
        assert_eq!(row1[1].to_string(), "Alice");
        
        let row2 = result.next().unwrap();
        assert_eq!(row2[0].to_string(), "2");
        assert_eq!(row2[1].to_string(), "Bob");
        
        assert!(result.next().is_none());
    }
}
