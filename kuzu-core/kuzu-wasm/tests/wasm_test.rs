//! WASM integration tests for kuzu-wasm.
//!
//! Run with: `wasm-pack test --node kuzu-wasm`

#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::*;
use kuzu_wasm::{KuzuDatabase, KuzuConnection};

wasm_bindgen_test_configure!(run_in_node);

#[wasm_bindgen_test]
fn test_create_database() {
    let db = KuzuDatabase::new(":memory:").expect("Failed to create in-memory database");
    let conn = KuzuConnection::new(&db).expect("Failed to create connection");
    
    // DDL
    let result = conn.query("CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name))");
    assert!(result.is_ok(), "CREATE NODE TABLE should succeed");
}

#[wasm_bindgen_test]
fn test_query_and_iterate() {
    let db = KuzuDatabase::new(":memory:").expect("Failed to create database");
    let conn = KuzuConnection::new(&db).expect("Failed to create connection");
    
    conn.query("CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name))").unwrap();
    conn.query("CREATE (:Person {name: 'Alice', age: 30})").unwrap();
    conn.query("CREATE (:Person {name: 'Bob', age: 25})").unwrap();
    
    let mut result = conn.query("MATCH (p:Person) RETURN p.name, p.age ORDER BY p.age").unwrap();
    assert!(result.is_success(), "Query should succeed");
    assert_eq!(result.get_num_rows(), 2, "Should return 2 rows");
    
    // Iterate rows
    let mut names = Vec::new();
    while result.has_next() {
        let row = result.get_next().unwrap();
        names.push(format!("{:?}", row));
    }
    assert_eq!(names.len(), 2);
}

#[wasm_bindgen_test]
fn test_prepared_statement() {
    let db = KuzuDatabase::new(":memory:").expect("Failed to create database");
    let conn = KuzuConnection::new(&db).expect("Failed to create connection");
    
    conn.query("CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name))").unwrap();
    
    let stmt = conn.prepare("CREATE (:Person {name: $name, age: $age})").unwrap();
    
    let params = js_sys::Object::new();
    js_sys::Reflect::set(&params, &"name".into(), &"Charlie".into()).unwrap();
    js_sys::Reflect::set(&params, &"age".into(), &35.into()).unwrap();
    
    let result = conn.execute(&stmt, &params).unwrap();
    assert!(result.is_success(), "Prepared statement execution should succeed");
}

#[wasm_bindgen_test]
fn test_column_names() {
    let db = KuzuDatabase::new(":memory:").expect("Failed to create database");
    let conn = KuzuConnection::new(&db).expect("Failed to create connection");
    
    conn.query("CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name))").unwrap();
    conn.query("CREATE (:Person {name: 'Test', age: 1})").unwrap();
    
    let result = conn.query("MATCH (p:Person) RETURN p.name, p.age").unwrap();
    let cols = result.get_column_names().unwrap();
    assert_eq!(cols.length(), 2, "Should have 2 columns");
}

#[wasm_bindgen_test]
fn test_error_handling() {
    let db = KuzuDatabase::new(":memory:").expect("Failed to create database");
    let conn = KuzuConnection::new(&db).expect("Failed to create connection");
    
    // Invalid Cypher should return error
    let result = conn.query("INVALID CYPHER SYNTAX");
    assert!(result.is_err(), "Invalid query should return error");
}

#[wasm_bindgen_test]
fn test_reset_iterator() {
    let db = KuzuDatabase::new(":memory:").expect("Failed to create database");
    let conn = KuzuConnection::new(&db).expect("Failed to create connection");
    
    conn.query("CREATE NODE TABLE Person(name STRING, PRIMARY KEY(name))").unwrap();
    conn.query("CREATE (:Person {name: 'A'})").unwrap();
    conn.query("CREATE (:Person {name: 'B'})").unwrap();
    
    let mut result = conn.query("MATCH (p:Person) RETURN p.name ORDER BY p.name").unwrap();
    
    // First pass
    assert!(result.has_next());
    let _ = result.get_next();
    assert!(result.has_next());
    let _ = result.get_next();
    assert!(!result.has_next());
    
    // Reset and second pass
    result.reset_iterator();
    assert!(result.has_next());
    let _ = result.get_next();
    assert!(result.has_next());
}
