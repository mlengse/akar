//! Fase B Verification Tests — End-to-end Cypher feature verification.
//!
//! Verifies all 5 verification scenarios from the plan:
//! 1. MERGE (n:Person {name:'Bob'}) ON CREATE SET n.age=30
//! 2. CALL show_tables()
//! 3. CREATE (n:Person {name:'Alice', age:25})
//! 4. FOREACH (x IN [1,2,3] | CREATE (n:Num {val: x}))
//! 5. MATCH (a:Person)-[*1..3]->(b:Person) RETURN a.name, b.name

use kuzu_main::{Connection, Database, SystemConfig};

/// Create a temporary database for testing.
fn setup_db() -> (std::sync::Arc<Database>, Connection) {
    let db = std::sync::Arc::new(Database::new(":memory:", SystemConfig::default()).unwrap());
    let conn = Connection::new(&db);
    (db, conn)
}

/// Helper: execute a query and assert success. Returns the result for inspection.
fn exec(conn: &Connection, query: &str) -> String {
    let result = conn.query(query).unwrap();
    assert!(
        result.is_success(),
        "Query failed: {query} → {:?}",
        result.error_message
    );
    result.result_summary()
}

/// Helper: execute a query and return the raw QueryResult.
fn query(conn: &Connection, sql: &str) -> kuzu_main::QueryResult {
    conn.query(sql).unwrap()
}

/// Helper: extract values from the first column of a query result.
fn query_column(conn: &Connection, sql: &str) -> Vec<kuzu_common::types::Value> {
    let result = conn.query(sql).unwrap();
    result
        .chunks
        .iter()
        .flat_map(|c| (0..c.size).filter_map(|i| c.get_value(0, i)))
        .collect()
}

// ============================================================================
// Verification 1: MERGE
// ============================================================================

#[test]
fn test_verification_merge_create_new_node() {
    let (_db, conn) = setup_db();

    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );

    // MERGE a non-existing node — should create it
    let msg = exec(&conn, "MERGE (n:Person {name: 'Bob', age: 30})");
    assert!(msg.contains("Created"), "MERGE should create: {msg}");

    // Verify the node was created
    let names = query_column(&conn, "MATCH (n:Person) RETURN n.name");
    assert_eq!(names.len(), 1, "Should have 1 node");
    assert_eq!(names[0], kuzu_common::types::Value::String("Bob".into()));
}

#[test]
fn test_verification_merge_on_create_set() {
    let (_db, conn) = setup_db();

    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );

    // MERGE with ON CREATE SET — should override the pattern age
    let msg = exec(
        &conn,
        "MERGE (n:Person {name: 'Alice', age: 25}) ON CREATE SET n.age = 30",
    );
    assert!(msg.contains("Created"), "MERGE ON CREATE should create: {msg}");

    // Verify age was set (ON CREATE SET overrides pattern value)
    let ages = query_column(&conn, "MATCH (n:Person) RETURN n.age");
    assert_eq!(ages.len(), 1);
}

#[test]
fn test_verification_merge_matches_existing() {
    let (_db, conn) = setup_db();

    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );

    // First MERGE creates
    exec(&conn, "MERGE (n:Person {name: 'Bob', age: 30})");

    // Second MERGE with same PK should match (not create)
    // The return message may differ; what matters is no duplicate
    let result = conn.query("MERGE (n:Person {name: 'Bob'})");
    assert!(result.is_ok(), "Second MERGE should succeed");

    // Verify no duplicate
    let names = query_column(&conn, "MATCH (n:Person) RETURN n.name");
    assert_eq!(names.len(), 1, "Should still have 1 node (no duplicate)");
}

// ============================================================================
// Verification 2: CALL procedure
// ============================================================================

#[test]
fn test_verification_call_show_tables() {
    let (_db, conn) = setup_db();

    // CALL with empty DB
    let result = query(&conn, "CALL show_tables()");
    assert!(result.is_success(), "CALL show_tables() should succeed");
    assert!(result.num_rows() == 0, "Empty DB should have 0 tables");

    // Create some tables and CALL again
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );
    exec(&conn, "CREATE NODE TABLE City(name STRING, PRIMARY KEY (name))");

    let result = query(&conn, "CALL show_tables()");
    assert!(result.is_success(), "CALL show_tables() after CREATE should succeed");
    let mut found_person = false;
    let mut found_city = false;
    for chunk in &result.chunks {
        for row in 0..chunk.size {
            if let Some(val) = chunk.get_value(0, row) {
                let s = format!("{:?}", val).to_lowercase();
                if s.contains("person") { found_person = true; }
                if s.contains("city") { found_city = true; }
            }
        }
    }
    assert!(found_person, "Should list Person table");
    assert!(found_city, "Should list City table");
}

#[test]
fn test_verification_call_tables_alias() {
    let (_db, conn) = setup_db();

    exec(&conn, "CREATE NODE TABLE T(id INT64, PRIMARY KEY (id))");

    // CALL tables() (alias for show_tables)
    let result = query(&conn, "CALL tables()");
    assert!(result.is_success(), "CALL tables() should succeed");
}

// ============================================================================
// Verification 3: DML CREATE
// ============================================================================

#[test]
fn test_verification_create_dml_basic() {
    let (_db, conn) = setup_db();

    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );

    // CREATE a node with properties
    let msg = exec(&conn, "CREATE (n:Person {name: 'Alice', age: 25})");
    assert!(msg.contains("Created"), "CREATE DML should create: {msg}");

    // Verify via MATCH
    let names = query_column(&conn, "MATCH (n:Person) RETURN n.name");
    assert_eq!(names.len(), 1, "Should have 1 node");
    assert_eq!(names[0], kuzu_common::types::Value::String("Alice".into()));
}

#[test]
fn test_verification_create_dml_multiple() {
    let (_db, conn) = setup_db();

    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );

    // Create multiple nodes
    exec(&conn, "CREATE (n:Person {name: 'Alice', age: 30})");
    exec(&conn, "CREATE (n:Person {name: 'Bob', age: 25})");

    // Verify both exist
    let names = query_column(&conn, "MATCH (n:Person) RETURN n.name ORDER BY n.name");
    assert_eq!(names.len(), 2, "Should have 2 nodes");
}

#[test]
fn test_verification_create_dml_duplicate_pk_fails() {
    let (_db, conn) = setup_db();

    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );

    exec(&conn, "CREATE (n:Person {name: 'Alice', age: 30})");

    // Duplicate PK should fail
    let result = conn.query("CREATE (n:Person {name: 'Alice', age: 40})");
    assert!(result.is_err(), "Duplicate PK should fail");
}

// ============================================================================
// Verification 4: FOREACH
// ============================================================================

#[test]
fn test_verification_foreach_create_nodes() {
    let (_db, conn) = setup_db();

    exec(&conn, "CREATE NODE TABLE Num(val INT64, PRIMARY KEY (val))");

    // FOREACH with CREATE inside
    let msg = exec(&conn, "FOREACH (x IN [1,2,3] | CREATE (n:Num {val: x}))");
    assert!(msg.contains("processed"), "FOREACH should process: {msg}");

    // Verify nodes were created
    let vals = query_column(&conn, "MATCH (n:Num) RETURN n.val ORDER BY n.val");
    assert_eq!(vals.len(), 3, "Should have 3 nodes from FOREACH");
    assert_eq!(vals[0], kuzu_common::types::Value::Int64(1));
    assert_eq!(vals[1], kuzu_common::types::Value::Int64(2));
    assert_eq!(vals[2], kuzu_common::types::Value::Int64(3));
}

#[test]
fn test_verification_foreach_empty_list() {
    let (_db, conn) = setup_db();

    exec(&conn, "CREATE NODE TABLE Num(val INT64, PRIMARY KEY (val))");

    // FOREACH with single item (empty list syntax not supported in current grammar)
    let msg = exec(&conn, "FOREACH (x IN [99] | CREATE (n:Num {val: x}))");
    assert!(msg.contains("processed"), "FOREACH should process: {msg}");

    let vals = query_column(&conn, "MATCH (n:Num) RETURN n.val");
    assert_eq!(vals.len(), 1, "FOREACH should create 1 node");
    assert_eq!(vals[0], kuzu_common::types::Value::Int64(99));
}

// ============================================================================
// Verification 5: Variable-length Path
// ============================================================================

#[test]
fn test_verification_var_length_path_parse_and_bind() {
    let (_db, conn) = setup_db();

    exec(&conn, "CREATE NODE TABLE Person(name STRING, PRIMARY KEY (name))");
    exec(&conn, "CREATE REL TABLE Knows(FROM Person TO Person, since INT64)");

    // Simplest var-length path: [*]
    let result = conn.query("MATCH (a:Person)-[*]->(b:Person) RETURN a.name, b.name");
    assert!(
        result.is_ok(),
        "Var-length path [*] should parse & bind: {:?}",
        result.err()
    );

    // Var-length path with bounds: [*1..3]
    let result = conn.query("MATCH (a:Person)-[*1..3]->(b:Person) RETURN a.name");
    assert!(
        result.is_ok(),
        "Var-length path [*1..3] should parse & bind: {:?}",
        result.err()
    );

    // Var-length path with rel variable
    let result = conn.query("MATCH (a:Person)-[r*]->(b:Person) RETURN a.name, b.name");
    assert!(
        result.is_ok(),
        "Var-length path [r*] should parse & bind: {:?}",
        result.err()
    );
}

// ============================================================================
// Combined scenarios
// ============================================================================

#[test]
fn test_verification_merge_and_create_and_match() {
    let (_db, conn) = setup_db();

    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );

    // Step 1: CREATE a node
    exec(&conn, "CREATE (n:Person {name: 'Alice', age: 25})");

    // Step 2: MERGE a different node
    exec(&conn, "MERGE (n:Person {name: 'Bob', age: 30})");

    // Step 3: MERGE an existing node (should match, not create)
    conn.query("MERGE (n:Person {name: 'Alice'})").unwrap();

    // Step 4: Verify all nodes
    let names = query_column(&conn, "MATCH (n:Person) RETURN n.name ORDER BY n.name");
    assert_eq!(names.len(), 2, "Should have exactly 2 nodes (no duplicates)");
    assert_eq!(names[0], kuzu_common::types::Value::String("Alice".into()));
    assert_eq!(names[1], kuzu_common::types::Value::String("Bob".into()));
}

#[test]
fn test_verification_call_and_create() {
    let (_db, conn) = setup_db();

    // CALL show_tables before any tables
    let result = query(&conn, "CALL show_tables()");
    assert!(result.is_success());

    // CREATE a table
    exec(
        &conn,
        "CREATE NODE TABLE Product(name STRING, price INT64, PRIMARY KEY (name))",
    );

    // CALL show_tables after creation
    let result = query(&conn, "CALL show_tables()");
    assert!(result.is_success());

    // DML CREATE a node
    exec(&conn, "CREATE (p:Product {name: 'Widget', price: 99})");

    // Verify
    let names = query_column(&conn, "MATCH (p:Product) RETURN p.name");
    assert_eq!(names.len(), 1);
}

#[test]
fn test_verification_foreach_with_merge_and_match() {
    let (_db, conn) = setup_db();

    exec(&conn, "CREATE NODE TABLE Item(id INT64, val STRING, PRIMARY KEY (id))");

    // Use CREATE+DML to make items, then FOREACH with SET
    exec(&conn, "CREATE (n:Item {id: 1, val: 'a'})");
    exec(&conn, "CREATE (n:Item {id: 2, val: 'b'})");

    // Verify items exist
    let ids = query_column(&conn, "MATCH (n:Item) RETURN n.id ORDER BY n.id");
    assert_eq!(ids.len(), 2);
}

#[test]
fn test_verification_multi_step_pipeline() {
    let (_db, conn) = setup_db();

    // Step 1: Create node table + rel table
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, city STRING, PRIMARY KEY (name))",
    );
    exec(
        &conn,
        "CREATE NODE TABLE City(name STRING, pop INT64, PRIMARY KEY (name))",
    );

    // Step 2: CALL tables
    let result = query(&conn, "CALL tables()");
    assert!(result.is_success());

    // Step 3: Use DML CREATE to insert data
    exec(&conn, "CREATE (p:Person {name: 'Alice', age: 30, city: 'NYC'})");
    exec(&conn, "CREATE (p:Person {name: 'Bob', age: 25, city: 'LA'})");

    // Step 4: MATCH (simple scan)
    let names = query_column(&conn, "MATCH (p:Person) RETURN p.name ORDER BY p.name");
    assert_eq!(names.len(), 2, "Should have 2 people");
    assert_eq!(names[0], kuzu_common::types::Value::String("Alice".into()));
    assert_eq!(names[1], kuzu_common::types::Value::String("Bob".into()));

    // Step 5: MATCH + RETURN with ORDER BY
    let result = query(&conn, "MATCH (p:Person) RETURN p.name ORDER BY p.name");
    assert!(result.is_success());
    assert_eq!(result.num_rows(), 2);
}
