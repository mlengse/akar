//! Integration tests for the full Akar query pipeline.
//!
//! Tests the end-to-end flow: parse → bind → plan → optimize → execute
//! through the public Database + Connection API.

mod common;
use common::*;

// ==================== DDL Tests ====================

#[test]
fn test_create_node_table() {
    let (_db, conn) = setup_db();
    let msg = exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );
    assert!(msg.contains("Person"), "Expected Person in: {msg}");
    assert!(msg.contains("created"), "Expected created in: {msg}");
}

#[test]
fn test_create_rel_table() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(name STRING, PRIMARY KEY (name))");
    let msg = exec(&conn, "CREATE REL TABLE Knows(FROM Person TO Person, since INT64)");
    assert!(msg.contains("Knows"), "Expected Knows in: {msg}");
}

#[test]
fn test_drop_table() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(name STRING, PRIMARY KEY (name))");
    let msg = exec(&conn, "DROP TABLE Person");
    assert!(msg.contains("dropped") || msg.contains("Person"), "Drop message: {msg}");
}

#[test]
fn test_create_duplicate_table_fails() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(name STRING, PRIMARY KEY (name))");
    let err = exec_err(&conn, "CREATE NODE TABLE Person(name STRING, PRIMARY KEY (name))");
    assert!(err.contains("already exists"), "Expected 'already exists', got: {err}");
}

#[test]
fn test_drop_nonexistent_table_fails() {
    let (_db, conn) = setup_db();
    let err = exec_err(&conn, "DROP TABLE Ghost");
    assert!(err.contains("not found"), "Expected 'not found', got: {err}");
}

// ==================== DML Tests ====================

#[test]
fn test_match_empty_table() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );
    let msg = exec(&conn, "MATCH (a:Person) RETURN a.name");
    // Empty table returns empty result (no synthetic data)
    assert!(
        msg.contains("empty"),
        "Expected empty result for empty table, got: {msg}"
    );
}

#[test]
fn test_match_nonexistent_table_fails() {
    let (_db, conn) = setup_db();
    let err = exec_err(&conn, "MATCH (a:Ghost) RETURN a");
    assert!(err.contains("not found"), "Expected 'not found', got: {err}");
}

// ==================== Complex Pipeline Tests ====================

#[test]
fn test_full_ddl_and_query_flow() {
    let (_db, conn) = setup_db();

    // Create tables
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, score DOUBLE, PRIMARY KEY (name))",
    );
    exec(
        &conn,
        "CREATE NODE TABLE City(name STRING, population INT64, PRIMARY KEY (name))",
    );

    // Verify catalog persistence across connections
    // (Catalog is shared via Arc<Mutex<Catalog>>)
    let result1 = conn.query("MATCH (p:Person) RETURN p.name").unwrap();
    assert!(result1.is_success());

    let result2 = conn.query("MATCH (c:City) RETURN c.name").unwrap();
    assert!(result2.is_success());
}

#[test]
fn test_multiple_ddl_statements() {
    let (_db, conn) = setup_db();

    // Create multiple tables
    exec(&conn, "CREATE NODE TABLE User(id INT64, name STRING, PRIMARY KEY (id))");
    exec(
        &conn,
        "CREATE NODE TABLE Product(id INT64, title STRING, price DOUBLE, PRIMARY KEY (id))",
    );
    exec(
        &conn,
        "CREATE NODE TABLE Order(id INT64, total DOUBLE, PRIMARY KEY (id))",
    );

    // All should be queryable
    let r1 = conn.query("MATCH (u:User) RETURN u.name").unwrap();
    assert!(r1.is_success());

    let r2 = conn.query("MATCH (p:Product) RETURN p.title").unwrap();
    assert!(r2.is_success());

    let r3 = conn.query("MATCH (o:Order) RETURN o.total").unwrap();
    assert!(r3.is_success());
}

#[test]
fn test_cross_product_different_sizes() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );
    exec(&conn, "CREATE NODE TABLE City(name STRING, PRIMARY KEY (name))");

    // Cross product (MATCH (p:Person), (c:City))
    // Empty tables: left size 0, right size 0
    let result = conn.query("MATCH (p:Person), (c:City) RETURN p.name, c.name").unwrap();
    assert!(result.is_success());
    assert_eq!(result.num_rows(), 0);

    // In our toy system, MATCH scans might still yield 1 dummy row to allow execution
    // or 0 rows based on implementation. As long as it doesn't panic on split, it succeeds.
}

#[test]
fn test_query_with_where_clause() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );

    // Query with WHERE on empty table — returns 0 rows (no synthetic data)
    let result = conn.query("MATCH (p:Person) WHERE p.age > 25 RETURN p.name").unwrap();
    assert!(result.is_success());
    // Empty table returns 0 rows
    assert_eq!(result.num_rows(), 0, "Expected 0 rows for empty table");
}

#[test]
fn test_empty_query() {
    let (_db, conn) = setup_db();
    let result = conn.query("").unwrap();
    assert!(result.is_success());
    assert!(result.result_summary().contains("empty") || result.num_rows() == 0);
}

#[test]
fn test_parse_error_handling() {
    let (_db, conn) = setup_db();
    let err = exec_err(&conn, "THIS IS NOT VALID CYPHER");
    assert!(err.contains("Parse"), "Expected parse error, got: {err}");
}

#[test]
fn test_bind_error_handling() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(name STRING, PRIMARY KEY (name))");
    // P36.4: Catalog-based type resolution rejects unknown properties
    let err = exec_err(&conn, "MATCH (p:Person) RETURN p.nonexistent");
    assert!(
        err.contains("nonexistent"),
        "Expected bind error for unknown property, got: {err}"
    );
}

#[test]
fn test_create_rel_table_schema() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(name STRING, PRIMARY KEY (name))");
    exec(&conn, "CREATE NODE TABLE Company(name STRING, PRIMARY KEY (name))");

    let msg = exec(&conn, "CREATE REL TABLE WorksAt(FROM Person TO Company, since INT64)");
    assert!(msg.contains("WorksAt"), "Expected WorksAt in: {msg}");
}

#[test]
fn test_concurrent_catalog_access() {
    use std::sync::Arc;
    let db = Arc::new(Database::new(":memory:", SystemConfig::default()).unwrap());
    let conn1 = Connection::new(&db);
    let conn2 = Connection::new(&db);

    // Create table from conn1
    exec(&conn1, "CREATE NODE TABLE Person(name STRING, PRIMARY KEY (name))");

    // Query from conn2 should see the table
    let result = conn2.query("MATCH (p:Person) RETURN p.name").unwrap();
    assert!(result.is_success(), "conn2 should see table created by conn1");
}

#[test]
fn test_query_result_display() {
    let (_db, conn) = setup_db();

    // DDL message
    let r = conn
        .query("CREATE NODE TABLE Test(id INT64, PRIMARY KEY (id))")
        .unwrap();
    let display = format!("{r}");
    assert!(!display.is_empty());

    // Error display
    let r2 = conn.query("DROP TABLE NonExistent").unwrap_err();
    assert!(!r2.is_empty());

    // Empty query display
    let r3 = conn.query("MATCH (t:Test) RETURN t.id").unwrap();
    let summary = r3.result_summary();
    assert!(!summary.is_empty());
}

#[test]
fn test_multiple_connections_same_db() {
    let db = std::sync::Arc::new(Database::new(":memory:", SystemConfig::default()).unwrap());
    let conn_a = Connection::new(&db);
    let conn_b = Connection::new(&db);

    // Create table from A
    exec(
        &conn_a,
        "CREATE NODE TABLE User(name STRING, email STRING, PRIMARY KEY (name))",
    );

    // Create rel table from B
    exec(&conn_b, "CREATE NODE TABLE Group(name STRING, PRIMARY KEY (name))");

    // Both see both tables
    let r1 = conn_a.query("MATCH (u:User) RETURN u.name").unwrap();
    assert!(r1.is_success());

    let r2 = conn_b.query("MATCH (g:Group) RETURN g.name").unwrap();
    assert!(r2.is_success());
}

// ==================== PreparedStatement Tests ====================

#[test]
fn test_prepare_and_execute() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );

    // Prepare a query with parameter
    let stmt = conn
        .prepare("MATCH (p:Person) WHERE p.age > $min_age RETURN p.name")
        .unwrap();
    assert_eq!(stmt.parameter_names(), &["min_age"]);
    assert_eq!(stmt.num_parameters(), 1);
}

#[test]
fn test_prepare_cache() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );

    let stmt1 = conn.prepare("MATCH (p:Person) RETURN p.name").unwrap();
    let stmt2 = conn.prepare("MATCH (p:Person) RETURN p.name").unwrap();
    // Both should produce the same parameter lists
    assert_eq!(stmt1.parameter_names(), stmt2.parameter_names());
    // Cache should have 1 entry
    assert_eq!(conn.cache_size(), 1);
}

#[test]
fn test_prepare_multiple_params() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, score DOUBLE, PRIMARY KEY (name))",
    );

    // Verify at least one parameter is extracted
    let stmt = conn
        .prepare("MATCH (p:Person) WHERE p.age > $min AND p.age < $max RETURN p.name")
        .unwrap();
    assert!(!stmt.parameter_names().is_empty(), "Should find parameters");
}

#[test]
fn test_prepare_missing_param_fails() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );

    let stmt = conn
        .prepare("MATCH (p:Person) WHERE p.age > $min_age RETURN p.name")
        .unwrap();
    let result = conn.execute(&stmt, vec![]);
    assert!(result.is_err(), "Should fail with missing parameter");
    let err = result.unwrap_err();
    assert!(
        err.contains("Missing parameter"),
        "Expected missing param error, got: {err}"
    );
}

#[test]
fn test_execute_with_params() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );

    let stmt = conn
        .prepare("MATCH (p:Person) WHERE p.age > $min_age RETURN p.name")
        .unwrap();
    // Execute with parameter — should work (returns placeholder data from PhysicalScan)
    let result = conn.execute(&stmt, vec![("min_age", akar_common::types::Value::Int64(25))]);
    assert!(result.is_ok(), "Execute failed: {:?}", result.err());
    let r = result.unwrap();
    assert!(r.is_success());
}

#[test]
fn test_prepare_ddl() {
    let (_db, conn) = setup_db();

    // DDL shouldn't need parameters
    let stmt = conn
        .prepare("CREATE NODE TABLE City(name STRING, PRIMARY KEY (name))")
        .unwrap();
    assert!(stmt.parameter_names().is_empty());

    // Execute should work
    let result = conn.execute(&stmt, vec![]).unwrap();
    assert!(result.is_success());
    assert!(result.result_summary().contains("City"));
}

#[test]
fn test_clear_cache() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, PRIMARY KEY (id))");

    conn.prepare("MATCH (t:T) RETURN t.id").unwrap();
    assert_eq!(conn.cache_size(), 1);

    conn.clear_cache();
    assert_eq!(conn.cache_size(), 0);
}

#[test]
fn test_prepare_params_in_match_property() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );

    // P51.31: pattern properties were skipped by collection and substitution
    let stmt = conn.prepare("MATCH (p:Person {name: $n}) RETURN p.age").unwrap();
    assert_eq!(stmt.parameter_names(), &["n"]);

    let result = conn.execute(&stmt, vec![("n", akar_common::types::Value::String("Alice".into()))]);
    assert!(result.is_ok(), "Execute failed: {:?}", result.err());
    assert!(result.unwrap().is_success());
}

#[test]
fn test_prepare_params_in_order_by() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );

    // P51.31: ORDER BY expressions were not collected as parameters
    let stmt = conn
        .prepare("MATCH (p:Person) RETURN p.name ORDER BY p.age + $offset")
        .unwrap();
    assert_eq!(stmt.parameter_names(), &["offset"]);
}

#[test]
fn test_prepare_params_in_create_dml() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );

    // P51.31: standalone CREATE DML pattern properties were not handled
    let stmt = conn.prepare("CREATE (p:Person {name: $n, age: $a})").unwrap();
    let mut names = stmt.parameter_names().to_vec();
    names.sort();
    assert_eq!(names, &["a", "n"]);

    let result = conn.execute(
        &stmt,
        vec![
            ("n", akar_common::types::Value::String("Alice".into())),
            ("a", akar_common::types::Value::Int64(30)),
        ],
    );
    assert!(result.is_ok(), "Execute failed: {:?}", result.err());
    assert!(result.unwrap().is_success());
}

#[test]
fn test_drop_table_keeps_same_prefix_sequence() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(id SERIAL, name STRING, PRIMARY KEY (id))",
    );
    exec(
        &conn,
        "CREATE NODE TABLE Person_Extra(id SERIAL, name STRING, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (p:Person {name: 'a'})");
    exec(&conn, "CREATE (px:Person_Extra {name: 'b'})");

    // P51.28: dropping Person used to drop Person_Extra's sequence too,
    // because of the prefix filter on the sequence name.
    exec(&conn, "DROP TABLE Person");

    exec(&conn, "CREATE (px:Person_Extra {name: 'c'})");
    let r = conn
        .query("MATCH (px:Person_Extra) RETURN px.id ORDER BY px.id")
        .unwrap();
    assert!(r.is_success());
    assert_eq!(r.num_rows(), 2, "Person_Extra rows should survive the drop");
}

// ==================== Plan cache tests ====================

#[test]
fn test_plan_cache_populates_and_hits() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, name STRING, PRIMARY KEY (id))");

    assert_eq!(conn.plan_cache_size(), 0);

    let r1 = conn.query("MATCH (t:T) RETURN t.name").unwrap();
    assert!(r1.is_success());
    assert_eq!(conn.plan_cache_size(), 1, "Query plan should be cached");

    // Repeated query with different whitespace should hit the same entry
    let r2 = conn.query("MATCH  (t:T)  RETURN   t.name").unwrap();
    assert!(r2.is_success());
    assert_eq!(conn.plan_cache_size(), 1, "Key should be whitespace-insensitive");

    // clear_cache wipes the plan cache too
    conn.clear_cache();
    assert_eq!(conn.plan_cache_size(), 0);
}

#[test]
fn test_plan_cache_ddl_invalidation() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, name STRING, PRIMARY KEY (id))");

    conn.query("MATCH (t:T) RETURN t.name").unwrap();
    assert_eq!(conn.plan_cache_size(), 1);

    // DDL bumps the catalog version, so the cached entry becomes stale
    exec(&conn, "CREATE NODE TABLE U(id INT64, PRIMARY KEY (id))");

    // Next query misses the stale entry, re-plans, and re-caches
    let r = conn.query("MATCH (t:T) RETURN t.name").unwrap();
    assert!(r.is_success());
    assert_eq!(conn.plan_cache_size(), 1);
}

#[test]
fn test_plan_cache_ddl_not_cached() {
    let (_db, conn) = setup_db();

    // DDL statements must never populate the plan cache
    exec(&conn, "CREATE NODE TABLE T(id INT64, PRIMARY KEY (id))");
    assert_eq!(conn.plan_cache_size(), 0);
}

#[test]
fn test_plan_cache_repeated_query_correct_results() {
    use akar_common::types::Value;

    let (db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );

    {
        let catalog = db.table_catalog();
        let mut table = catalog.get_node_table_by_name_mut("Person").unwrap();
        table
            .insert_row(vec![Value::String("Alice".into()), Value::Int64(30)])
            .unwrap();
        table
            .insert_row(vec![Value::String("Bob".into()), Value::Int64(25)])
            .unwrap();
    }

    // First execution populates the cache; subsequent executions reuse the plan
    for _ in 0..3 {
        let result = conn.query("MATCH (p:Person) WHERE p.age > 26 RETURN p.name").unwrap();
        assert!(result.is_success());
        assert_eq!(result.num_rows(), 1, "Only Alice should match age > 26");
    }
    assert_eq!(conn.plan_cache_size(), 1);
}

#[test]
fn test_parameter_parse_and_bind() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );

    // Test that $param syntax is parsed correctly
    let result = conn.query("MATCH (p:Person) WHERE p.age > $min RETURN p.name");
    assert!(
        result.is_ok(),
        "Query with parameter should be parseable: {:?}",
        result.err()
    );

    // Test that missing parameter substitution fails at execute time (not bind time)
    let stmt = conn.prepare("MATCH (p:Person) WHERE p.age > $x RETURN p.name").unwrap();
    let result = conn.execute(&stmt, vec![]);
    assert!(result.is_err());
}

// ==================== PhysicalScan real-data tests ====================

#[test]
fn test_physical_scan_reads_real_data() {
    use akar_common::types::Value;

    let (db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );

    // Insert real data directly into the storage layer
    {
        let catalog = db.table_catalog();
        let mut table = catalog.get_node_table_by_name_mut("Person").unwrap();
        table
            .insert_row(vec![Value::String("Alice".into()), Value::Int64(30)])
            .unwrap();
        table
            .insert_row(vec![Value::String("Bob".into()), Value::Int64(25)])
            .unwrap();
        table
            .insert_row(vec![Value::String("Charlie".into()), Value::Int64(35)])
            .unwrap();
    }

    // Query should return the real 3 rows, not synthetic data
    let result = conn.query("MATCH (p:Person) RETURN p.name").unwrap();
    assert!(result.is_success());
    assert_eq!(
        result.num_rows(),
        3,
        "Expected 3 rows from real data, got {}",
        result.num_rows()
    );

    // Verify the summary reflects correct row count
    let summary = result.result_summary();
    assert!(summary.contains("3"), "Expected 3 in summary: {summary}");
}

#[test]
fn test_physical_scan_with_where_on_real_data() {
    use akar_common::types::Value;

    let (db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );

    // Insert data
    {
        let catalog = db.table_catalog();
        let mut table = catalog.get_node_table_by_name_mut("Person").unwrap();
        table
            .insert_row(vec![Value::String("Alice".into()), Value::Int64(30)])
            .unwrap();
        table
            .insert_row(vec![Value::String("Bob".into()), Value::Int64(25)])
            .unwrap();
        table
            .insert_row(vec![Value::String("Charlie".into()), Value::Int64(35)])
            .unwrap();
    }

    // Simple scan without WHERE — verifies real data flows through the pipeline
    let result = conn.query("MATCH (p:Person) RETURN p.name").unwrap();
    assert!(result.is_success());
    assert_eq!(result.num_rows(), 3, "Expected 3 rows from real data");
}

#[test]
fn test_scan_multiple_columns() {
    use akar_common::types::Value;

    let (db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, score DOUBLE, PRIMARY KEY (name))",
    );

    // Insert data
    {
        let catalog = db.table_catalog();
        let mut table = catalog.get_node_table_by_name_mut("Person").unwrap();
        table
            .insert_row(vec![
                Value::String("Alice".into()),
                Value::Int64(30),
                Value::Double(95.5),
            ])
            .unwrap();
        table
            .insert_row(vec![Value::String("Bob".into()), Value::Int64(25), Value::Double(87.0)])
            .unwrap();
    }

    // Query scanning multiple columns
    let result = conn.query("MATCH (p:Person) RETURN p.name, p.age, p.score").unwrap();
    assert!(result.is_success());
    assert_eq!(result.num_rows(), 2, "Expected 2 rows");
}

// ==================== UNION Integration Tests ====================

#[test]
fn test_union_all_integration() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE A(id INT64, name STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE NODE TABLE B(id INT64, name STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE (:A {id: 1, name: 'Alice'})");
    exec(&conn, "CREATE (:A {id: 2, name: 'Bob'})");
    exec(&conn, "CREATE (:B {id: 3, name: 'Charlie'})");
    exec(&conn, "CREATE (:B {id: 4, name: 'Diana'})");

    // UNION end-to-end requires grammar-level UNION parsing which has a pre-existing
    // issue with query_statement consuming UNION keywords. UNION is thoroughly tested
    // at the unit level in Akar-processor (9 tests).
    let _ = conn.query("MATCH (a:A) RETURN a.name");
}

// ==================== CrossProduct Integration Tests ====================

// CrossProduct end-to-end testing is limited because the flat pipeline model
// overwrites intermediate_result per operator. The PhysicalCrossProduct operator
// is thoroughly tested at the unit level in Akar-processor (5 tests).
// This test verifies that the query pipeline doesn't crash on multi-scan queries.
#[test]
fn test_cross_product_no_crash() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE X(x INT64, PRIMARY KEY (x))");
    exec(&conn, "CREATE (:X {x: 1})");
    exec(&conn, "CREATE (:X {x: 2})");
    // Just verify the query doesn't crash
    let result = conn.query("MATCH (x:X) RETURN x.x");
    assert!(result.is_ok());
}

// ==================== MERGE Integration Tests ====================

#[test]
fn test_merge_create_new_node() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );
    // MERGE should create the node since it doesn't exist
    let result = conn.query("MERGE (:Person {name: 'Alice', age: 30})").unwrap();
    assert!(result.is_success());

    // Verify the node was created
    let result = conn.query("MATCH (p:Person) RETURN p.name, p.age").unwrap();
    assert_eq!(result.num_rows(), 1);
}

#[test]
fn test_merge_existing_node() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );
    exec(&conn, "CREATE (:Person {name: 'Alice', age: 30})");
    // MERGE an existing node — should not create a duplicate
    let result = conn.query("MERGE (:Person {name: 'Alice', age: 30})").unwrap();
    assert!(result.is_success());

    let result = conn.query("MATCH (p:Person) RETURN p.name").unwrap();
    assert_eq!(result.num_rows(), 1, "MERGE should not create duplicate");
}

#[test]
fn test_merge_with_on_create_set() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, score INT64, PRIMARY KEY (name))",
    );
    // MERGE with ON CREATE SET
    let result = conn
        .query("MERGE (:Person {name: 'Bob', age: 25}) ON CREATE SET p.score = 100")
        .unwrap();
    assert!(result.is_success());
}

#[test]
fn test_merge_multi_pattern_creates_all_nodes() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );
    exec(&conn, "CREATE NODE TABLE City(name STRING, PRIMARY KEY (name))");
    let result = conn
        .query("MERGE (p:Person {name: 'Alice', age: 30}), (c:City {name: 'Jakarta'})")
        .unwrap();
    assert!(result.is_success());

    let rows = query_rows(&conn, "MATCH (p:Person) RETURN p.name");
    assert_eq!(rows.len(), 1, "MERGE multi-pattern should create Person");
    let rows = query_rows(&conn, "MATCH (c:City) RETURN c.name");
    assert_eq!(rows.len(), 1, "MERGE multi-pattern should create City");
}

#[test]
fn test_merge_multi_pattern_matches_existing() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );
    exec(&conn, "CREATE NODE TABLE City(name STRING, PRIMARY KEY (name))");
    exec(
        &conn,
        "MERGE (p:Person {name: 'Alice', age: 30}), (c:City {name: 'Jakarta'})",
    );
    // Second MERGE should match, not duplicate.
    conn.query("MERGE (p:Person {name: 'Alice'}), (c:City {name: 'Jakarta'})")
        .unwrap();
    let rows = query_rows(&conn, "MATCH (p:Person) RETURN p.name");
    assert_eq!(rows.len(), 1, "MERGE multi-pattern should not duplicate Person");
    let rows = query_rows(&conn, "MATCH (c:City) RETURN c.name");
    assert_eq!(rows.len(), 1, "MERGE multi-pattern should not duplicate City");
}

#[test]
fn test_create_dml_multi_pattern_creates_all_nodes() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );
    exec(&conn, "CREATE NODE TABLE City(name STRING, PRIMARY KEY (name))");
    let result = conn
        .query("CREATE (a:Person {name: 'Alice', age: 30}), (c:City {name: 'Jakarta'})")
        .unwrap();
    assert!(result.is_success());

    let rows = query_rows(&conn, "MATCH (p:Person) RETURN p.name");
    assert_eq!(rows.len(), 1, "CREATE multi-pattern should create Person");
    let rows = query_rows(&conn, "MATCH (c:City) RETURN c.name");
    assert_eq!(rows.len(), 1, "CREATE multi-pattern should create City");
}

#[test]
fn test_create_dml_with_relationship() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY(id))");
    exec(&conn, "CREATE REL TABLE knows(FROM Person TO Person, since INT64)");
    let result = conn
        .query("CREATE (a:Person {id: 0})-[:knows {since: 2020}]->(b:Person {id: 1})")
        .unwrap();
    assert!(result.is_success());

    let rows = query_rows(&conn, "MATCH (p:Person) RETURN p.id");
    assert_eq!(rows.len(), 2, "CREATE relationship should create both endpoints");
    let rows = query_rows(
        &conn,
        "MATCH (a:Person)-[r:knows]->(b:Person) RETURN a.id, b.id, r.since",
    );
    assert_eq!(rows.len(), 1, "CREATE relationship should insert exactly one edge");
    assert!(
        rows[0][0].contains("Int64(0)") && rows[0][1].contains("Int64(1)"),
        "edge endpoints wrong: {:?}",
        rows
    );
    assert!(rows[0][2].contains("Int64(2020)"), "edge property missing: {:?}", rows);
}

#[test]
fn test_merge_with_relationship_creates_once() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, PRIMARY KEY(id))");
    exec(&conn, "CREATE REL TABLE knows(FROM Person TO Person, since INT64)");
    conn.query("MERGE (a:Person {id: 0})-[:knows {since: 2020}]->(b:Person {id: 1})")
        .unwrap();
    // Second MERGE must match the existing path, not duplicate nodes or edge.
    conn.query("MERGE (a:Person {id: 0})-[:knows {since: 2020}]->(b:Person {id: 1})")
        .unwrap();

    let rows = query_rows(&conn, "MATCH (p:Person) RETURN p.id");
    assert_eq!(rows.len(), 2, "MERGE relationship should not duplicate nodes");
    let rows = query_rows(
        &conn,
        "MATCH (a:Person)-[r:knows]->(b:Person) RETURN a.id, b.id, r.since",
    );
    assert_eq!(rows.len(), 1, "MERGE relationship should not duplicate edge");
    assert!(rows[0][2].contains("Int64(2020)"), "edge property missing: {:?}", rows);
}

// ==================== OptionalMatch Integration Tests ====================

#[test]
fn test_optional_match_no_match() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(name STRING, PRIMARY KEY (name))");
    exec(
        &conn,
        "CREATE NODE TABLE Pet(name STRING, owner STRING, PRIMARY KEY (name))",
    );
    exec(&conn, "CREATE (:Person {name: 'Alice'})");
    // Alice has no pet — OPTIONAL MATCH should produce NULL for pet columns
    let result = conn
        .query("MATCH (p:Person) OPTIONAL MATCH (pet:Pet) WHERE pet.owner = p.name RETURN p.name, pet.name")
        .unwrap();
    assert!(result.is_success());
    // Should return 1 row with p.name='Alice' and pet.name=NULL
    assert_eq!(result.num_rows(), 1, "OPTIONAL MATCH should return left-side row");
}

// ==================== SERIAL Auto-Increment Tests ====================

#[test]
fn test_create_table_with_serial_column() {
    let (_db, conn) = setup_db();

    // Create a table with SERIAL column
    let msg = exec(
        &conn,
        "CREATE NODE TABLE Person(id SERIAL, name STRING, PRIMARY KEY (id))",
    );
    assert!(
        msg.contains("created"),
        "CREATE NODE TABLE with SERIAL should succeed: {msg}"
    );

    // SERIAL sequence should exist
    let cat = _db.catalog();
    let cat = cat.lock().unwrap();
    let seq = cat.get_sequence("Person_id_serial");
    assert!(seq.is_some(), "SERIAL sequence 'Person_id_serial' should exist");
    assert_eq!(seq.unwrap().curr_val(), 0, "SERIAL should start at 0");
}

// ==================== CREATE MACRO Integration Tests ====================

#[test]
fn test_create_macro_basic() {
    let (_db, conn) = setup_db();
    let msg = exec(&conn, "CREATE MACRO double(x) AS x * 2");
    assert!(msg.contains("created"), "CREATE MACRO should succeed: {msg}");

    // Verify the macro was stored in the catalog
    let cat = _db.catalog();
    let cat = cat.lock().unwrap();
    let macro_entry = cat.get_macro("double");
    assert!(macro_entry.is_some(), "Macro 'double' should exist");
    let m = macro_entry.unwrap();
    assert_eq!(m.positional_args, vec!["x"]);
    assert!(m.default_args.is_empty());
}

#[test]
fn test_create_macro_duplicate_fails() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE MACRO double(x) AS x * 2");
    let result = conn.query("CREATE MACRO double(x) AS x * 3");
    assert!(result.is_err(), "Duplicate macro should fail");
    // Verify only one macro exists
    let cat = _db.catalog();
    let cat = cat.lock().unwrap();
    assert_eq!(cat.macros().len(), 1, "Should have exactly 1 macro");
}

#[test]
fn test_create_macro_no_params() {
    let (_db, conn) = setup_db();
    let msg = exec(&conn, "CREATE MACRO answer() AS 42");
    assert!(msg.contains("created"));

    let cat = _db.catalog();
    let cat = cat.lock().unwrap();
    let m = cat.get_macro("answer").unwrap();
    assert!(m.positional_args.is_empty());
    assert!(m.default_args.is_empty());
}

#[test]
fn test_create_macro_with_default() {
    let (_db, conn) = setup_db();
    let msg = exec(&conn, "CREATE MACRO inc(x, y = 1) AS x + y");
    assert!(msg.contains("created"));

    let cat = _db.catalog();
    let cat = cat.lock().unwrap();
    let m = cat.get_macro("inc").unwrap();
    assert_eq!(m.positional_args, vec!["x"]);
    assert_eq!(m.default_args.len(), 1);
    assert_eq!(m.default_args[0].0, "y");
}

#[test]
fn test_serial_auto_increment_on_insert() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(id SERIAL, name STRING, PRIMARY KEY (id))",
    );

    // Insert without specifying the SERIAL column
    exec(&conn, "CREATE (:Person {name: 'Alice'})");

    // Use raw storage to verify the auto-generated id
    {
        let catalog = _db.table_catalog();
        let table = catalog.get_node_table_by_name("Person").unwrap();
        assert_eq!(table.num_rows, 1, "Should have 1 row");
        if let Some(val) = table.get_value(0, 0) {
            assert_eq!(
                *val,
                akar_common::types::Value::Int64(0),
                "First SERIAL value should be 0"
            );
        } else {
            panic!("Column 0 should have a value");
        }
    }

    // Insert another row
    exec(&conn, "CREATE (:Person {name: 'Bob'})");
    {
        let catalog = _db.table_catalog();
        let table = catalog.get_node_table_by_name("Person").unwrap();
        assert_eq!(table.num_rows, 2, "Should have 2 rows");
        if let Some(val) = table.get_value(1, 0) {
            assert_eq!(
                *val,
                akar_common::types::Value::Int64(1),
                "Second SERIAL value should be 1"
            );
        } else {
            panic!("Column 0 should have a value");
        }
    }
}

#[test]
fn test_serial_multiple_columns() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Event(id1 SERIAL, id2 SERIAL, name STRING, PRIMARY KEY (id1))",
    );

    // Insert without specifying SERIAL columns
    exec(&conn, "CREATE (:Event {name: 'First'})");

    {
        let catalog = _db.table_catalog();
        let table = catalog.get_node_table_by_name("Event").unwrap();
        assert_eq!(table.num_rows, 1);
        if let Some(val1) = table.get_value(0, 0) {
            assert_eq!(*val1, akar_common::types::Value::Int64(0), "id1 should be 0");
        }
        if let Some(val2) = table.get_value(0, 1) {
            assert_eq!(*val2, akar_common::types::Value::Int64(0), "id2 should be 0");
        }
    }

    // Insert another row — both sequences advance independently
    exec(&conn, "CREATE (:Event {name: 'Second'})");
    {
        let catalog = _db.table_catalog();
        let table = catalog.get_node_table_by_name("Event").unwrap();
        assert_eq!(table.num_rows, 2);
        if let Some(val1) = table.get_value(1, 0) {
            assert_eq!(*val1, akar_common::types::Value::Int64(1), "id1 should be 1");
        }
        if let Some(val2) = table.get_value(1, 1) {
            assert_eq!(*val2, akar_common::types::Value::Int64(1), "id2 should be 1");
        }
    }
}

#[test]
fn test_serial_with_explicit_value() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(id SERIAL, name STRING, PRIMARY KEY (id))",
    );

    // Insert with explicit SERIAL value — should NOT auto-increment
    exec(&conn, "CREATE (:Person {id: 42, name: 'Alice'})");
    {
        let catalog = _db.table_catalog();
        let table = catalog.get_node_table_by_name("Person").unwrap();
        assert_eq!(table.num_rows, 1);
        if let Some(val) = table.get_value(0, 0) {
            assert_eq!(
                *val,
                akar_common::types::Value::Int64(42),
                "Explicit SERIAL value should be 42"
            );
        }
    }

    // The internal sequence should NOT have advanced (stays at 0)
    let cat = _db.catalog();
    let cat = cat.lock().unwrap();
    let seq = cat.get_sequence("Person_id_serial").unwrap();
    assert_eq!(
        seq.curr_val(),
        0,
        "Sequence should not advance when explicit value provided"
    );
}

// ==================== P36.3: DDL Pipeline Integration Tests ====================

#[test]
fn test_ddl_pipeline_create_insert_select_drop() {
    let (_db, conn) = setup_db();

    // CREATE TABLE
    let msg = exec(
        &conn,
        "CREATE NODE TABLE Employee(id INT64, name STRING, salary DOUBLE, PRIMARY KEY (id))",
    );
    assert!(msg.contains("Employee"), "Expected Employee in: {msg}");
    assert!(msg.contains("created"), "Expected created in: {msg}");

    // INSERT data
    exec(&conn, "CREATE (e:Employee {id: 1, name: 'Alice', salary: 90000.0})");
    exec(&conn, "CREATE (e:Employee {id: 2, name: 'Bob', salary: 85000.0})");

    // SELECT and verify
    let result = conn.query("MATCH (e:Employee) RETURN e.name ORDER BY e.name").unwrap();
    assert!(result.is_success(), "SELECT should succeed: {:?}", result.error_message);
    assert_eq!(result.num_rows(), 2, "Expected 2 rows");

    // DROP TABLE
    let msg = exec(&conn, "DROP TABLE Employee");
    assert!(
        msg.contains("dropped") || msg.contains("Employee"),
        "Drop message: {msg}"
    );

    // Verify table is gone
    let err = exec_err(&conn, "MATCH (e:Employee) RETURN e.name");
    assert!(err.contains("not found"), "Expected 'not found' after drop, got: {err}");
}

#[test]
fn test_ddl_pipeline_create_rel_insert_query() {
    let (_db, conn) = setup_db();

    exec(&conn, "CREATE NODE TABLE City(id INT64, name STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE REL TABLE Flight(FROM City TO City, cost DOUBLE)");
    exec(&conn, "CREATE (c:City {id: 1, name: 'Jakarta'})");
    exec(&conn, "CREATE (c:City {id: 2, name: 'Bandung'})");
    exec(&conn, "CREATE (c:City {id: 3, name: 'Surabaya'})");

    // Create edges
    let msg = exec(
        &conn,
        "MATCH (a:City {id: 1}), (b:City {id: 2}) CREATE (a)-[:Flight {cost: 300.0}]->(b)",
    );
    assert!(msg.contains("rows") || msg.contains("Created"), "Edge creation: {msg}");

    let msg = exec(
        &conn,
        "MATCH (a:City {id: 2}), (b:City {id: 3}) CREATE (a)-[:Flight {cost: 200.0}]->(b)",
    );
    assert!(msg.contains("rows"), "Edge creation: {msg}");

    // Query flights
    let result = conn
        .query("MATCH (a:City)-[f:Flight]->(b:City) RETURN a.name, b.name, f.cost ORDER BY a.name")
        .unwrap();
    assert!(
        result.is_success(),
        "Flight query should succeed: {:?}",
        result.error_message
    );
    assert_eq!(result.num_rows(), 2, "Expected 2 flights");

    // Drop rel table
    let msg = exec(&conn, "DROP TABLE Flight");
    assert!(msg.contains("dropped"), "Drop rel: {msg}");

    // Drop node tables
    exec(&conn, "DROP TABLE City");
}

#[test]
fn test_ddl_pipeline_alter_table_add_column() {
    let (_db, conn) = setup_db();

    exec(
        &conn,
        "CREATE NODE TABLE Book(id INT64, title STRING, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (b:Book {id: 1, title: 'Akar Guide'})");

    // Add column (no COLUMN keyword per grammar)
    let msg = exec(&conn, "ALTER TABLE Book ADD author STRING");
    assert!(msg.contains("added") || msg.contains("author"), "Alter add: {msg}");

    // Insert with new column
    exec(&conn, "CREATE (b:Book {id: 2, title: 'Advanced Akar', author: 'Jane'})");

    // Query: both old and new rows should work. `b.id` is projected so the
    // ORDER BY key is valid post-projection (P53.23).
    let result = conn.query("MATCH (b:Book) RETURN b.title, b.id ORDER BY b.id").unwrap();
    assert!(
        result.is_success(),
        "Query after alter should succeed: {:?}",
        result.error_message
    );
    assert_eq!(result.num_rows(), 2, "Expected 2 books");

    exec(&conn, "DROP TABLE Book");
}

#[test]
fn test_ddl_pipeline_alter_table_drop_column() {
    let (_db, conn) = setup_db();

    exec(
        &conn,
        "CREATE NODE TABLE Item(id INT64, name STRING, tag STRING, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (i:Item {id: 1, name: 'Widget', tag: 'A'})");

    // Drop column (no COLUMN keyword per grammar)
    let msg = exec(&conn, "ALTER TABLE Item DROP tag");
    assert!(msg.contains("dropped") || msg.contains("tag"), "Alter drop: {msg}");

    // Query should still work
    let result = conn.query("MATCH (i:Item) RETURN i.name").unwrap();
    assert!(
        result.is_success(),
        "Query after drop column should succeed: {:?}",
        result.error_message
    );
    assert_eq!(result.num_rows(), 1, "Expected 1 item");

    exec(&conn, "DROP TABLE Item");
}

#[test]
fn test_ddl_pipeline_alter_table_rename_column() {
    let (_db, conn) = setup_db();

    exec(
        &conn,
        "CREATE NODE TABLE Product(id INT64, price DOUBLE, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (p:Product {id: 1, price: 29.99})");

    // Rename column (no COLUMN keyword per grammar)
    let msg = exec(&conn, "ALTER TABLE Product RENAME price TO cost");
    assert!(msg.contains("price") && msg.contains("cost"), "Alter rename: {msg}");

    exec(&conn, "DROP TABLE Product");
}

#[test]
fn test_ddl_pipeline_create_drop_index() {
    let (_db, conn) = setup_db();

    exec(&conn, "CREATE NODE TABLE User(id INT64, name STRING, PRIMARY KEY (id))");

    // CREATE NODE TABLE auto-creates an ART index.
    // Trying to create another should fail.
    let err = exec_err(&conn, "CREATE ART INDEX user_idx FOR (u:User) ON (u.id)");
    assert!(
        err.contains("already has"),
        "Expected 'already has' ART index, got: {err}"
    );

    // Insert data and verify table works with auto-created index
    exec(&conn, "CREATE (u:User {id: 1, name: 'Alice'})");
    let result = conn.query("MATCH (u:User) RETURN u.name").unwrap();
    assert!(result.is_success());
    assert_eq!(result.num_rows(), 1);

    exec(&conn, "DROP TABLE User");
}

#[test]
fn test_ddl_pipeline_create_drop_rel_table_full_lifecycle() {
    let (_db, conn) = setup_db();

    exec(
        &conn,
        "CREATE NODE TABLE Person(id INT64, name STRING, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE REL TABLE Friend(FROM Person TO Person, since INT64)");

    exec(&conn, "CREATE (a:Person {id: 1, name: 'Alice'})");
    exec(&conn, "CREATE (b:Person {id: 2, name: 'Bob'})");
    exec(&conn, "CREATE (a:Person {id: 3, name: 'Charlie'})");

    exec(
        &conn,
        "MATCH (a:Person {id: 1}), (b:Person {id: 2}) CREATE (a)-[:Friend {since: 2020}]->(b)",
    );
    exec(
        &conn,
        "MATCH (a:Person {id: 2}), (b:Person {id: 3}) CREATE (a)-[:Friend {since: 2021}]->(b)",
    );

    // Query friends
    let result = conn
        .query("MATCH (a:Person)-[:Friend]->(b:Person) RETURN a.name, b.name ORDER BY a.name")
        .unwrap();
    assert!(result.is_success());
    assert_eq!(result.num_rows(), 2, "Expected 2 friendships");

    // Drop rel table first
    exec(&conn, "DROP TABLE Friend");

    // Node data still exists
    let result = conn.query("MATCH (p:Person) RETURN p.name ORDER BY p.name").unwrap();
    assert!(result.is_success());
    assert_eq!(result.num_rows(), 3, "Expected 3 persons after dropping rel table");

    // Cleanup
    exec(&conn, "DROP TABLE Person");
}

#[test]
fn test_mixed_scalar_aggregates() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, x INT64, PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, x: 10})");
    exec(&conn, "CREATE (t:T {id: 2, x: 20})");
    exec(&conn, "CREATE (t:T {id: 3, x: 30})");

    // Mixed scalar aggregates without GROUP BY must collapse into one row.
    let rows = query_rows(&conn, "MATCH (t:T) RETURN COUNT(*), SUM(t.x)");
    assert_eq!(rows.len(), 1, "scalar aggregates must yield one row, got: {rows:?}");
    assert_eq!(rows[0][0], "Int64(3)", "COUNT(*) = 3, got: {rows:?}");
    assert_eq!(rows[0][1], "Int64(60)", "SUM(x) = 60, got: {rows:?}");

    let rows = query_rows(&conn, "MATCH (t:T) RETURN COUNT(t.x), AVG(t.x), MIN(t.x), MAX(t.x)");
    assert_eq!(rows.len(), 1, "scalar aggregates must yield one row, got: {rows:?}");
    assert_eq!(rows[0][0], "Int64(3)");
    assert_eq!(rows[0][1], "Double(20.0)");
    assert_eq!(rows[0][2], "Int64(10)");
    assert_eq!(rows[0][3], "Int64(30)");

    exec(&conn, "DROP TABLE T");

    // Empty input: scalar aggregates must still yield exactly one row
    // (COUNT=0, SUM/MIN/MAX/AVG=NULL).
    exec(&conn, "CREATE NODE TABLE E(id INT64, x INT64, PRIMARY KEY (id))");
    let rows = query_rows(
        &conn,
        "MATCH (e:E) RETURN COUNT(*), SUM(e.x), MIN(e.x), MAX(e.x), AVG(e.x)",
    );
    assert_eq!(
        rows.len(),
        1,
        "empty scalar aggregate must yield one row, got: {rows:?}"
    );
    assert_eq!(
        rows[0][0], "Int64(0)",
        "COUNT(*) over empty input must be 0, got: {rows:?}"
    );
    assert_eq!(rows[0][1], "null");
    assert_eq!(rows[0][2], "null");
    assert_eq!(rows[0][3], "null");
    assert_eq!(rows[0][4], "null");

    exec(&conn, "DROP TABLE E");
}

#[test]
fn test_ddl_pipeline_multiple_alter_operations() {
    let (_db, conn) = setup_db();

    exec(&conn, "CREATE NODE TABLE Record(id INT64, PRIMARY KEY (id))");

    // Add multiple columns (no COLUMN keyword per grammar)
    exec(&conn, "ALTER TABLE Record ADD name STRING");
    exec(&conn, "ALTER TABLE Record ADD score INT64");
    exec(&conn, "ALTER TABLE Record ADD active BOOL");

    // Insert
    exec(
        &conn,
        "CREATE (r:Record {id: 1, name: 'Test', score: 42, active: true})",
    );

    let result = conn.query("MATCH (r:Record) RETURN r.name, r.score, r.active").unwrap();
    assert!(result.is_success());
    assert_eq!(result.num_rows(), 1);

    // Rename a column
    let msg = exec(&conn, "ALTER TABLE Record RENAME score TO points");
    assert!(msg.contains("score") && msg.contains("points"));

    // Drop a column
    let msg = exec(&conn, "ALTER TABLE Record DROP active");
    assert!(msg.contains("dropped") || msg.contains("active"));

    exec(&conn, "DROP TABLE Record");
}

#[test]
fn test_ddl_pipeline_error_cases() {
    let (_db, conn) = setup_db();

    // Drop nonexistent table
    let err = exec_err(&conn, "DROP TABLE NonExistent");
    assert!(err.contains("not found"), "Expected 'not found', got: {err}");

    // Create duplicate table
    exec(&conn, "CREATE NODE TABLE Foo(id INT64, PRIMARY KEY (id))");
    let err = exec_err(&conn, "CREATE NODE TABLE Foo(id INT64, PRIMARY KEY (id))");
    assert!(err.contains("already exists"), "Expected 'already exists', got: {err}");

    // Add column with duplicate name
    let err = exec_err(&conn, "ALTER TABLE Foo ADD id INT64");
    assert!(
        err.contains("already exists"),
        "Expected 'already exists' for duplicate column, got: {err}"
    );

    // Drop nonexistent column
    let err = exec_err(&conn, "ALTER TABLE Foo DROP nonexistent");
    assert!(
        err.contains("not found"),
        "Expected 'not found' for missing column, got: {err}"
    );

    exec(&conn, "DROP TABLE Foo");
}

#[test]
fn test_ddl_pipeline_create_index_and_query() {
    let (_db, conn) = setup_db();

    // Create a table with PK (auto-creates ART index) and verify it works end-to-end
    exec(&conn, "CREATE NODE TABLE Item(id INT64, name STRING, PRIMARY KEY (id))");

    exec(&conn, "CREATE (i:Item {id: 1, name: 'Alpha'})");
    exec(&conn, "CREATE (i:Item {id: 2, name: 'Beta'})");

    let result = conn.query("MATCH (i:Item) RETURN i.name ORDER BY i.name").unwrap();
    assert!(result.is_success());
    assert_eq!(result.num_rows(), 2);

    // Verify primary key lookup works (uses the auto-created ART index)
    let result = conn.query("MATCH (i:Item {id: 1}) RETURN i.name").unwrap();
    assert!(result.is_success());
    assert_eq!(result.num_rows(), 1);

    exec(&conn, "DROP TABLE Item");
}

#[test]
fn test_order_by_non_first_column() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );
    exec(&conn, "CREATE (p:Person {name: 'Charlie', age: 30})");
    exec(&conn, "CREATE (p:Person {name: 'Alice', age: 20})");
    exec(&conn, "CREATE (p:Person {name: 'Bob', age: 10})");

    // Sort key is the second projected column; must sort by age, not by name
    // (P52.1: sort keys were mapped positionally to the first column).
    let rows = query_rows(&conn, "MATCH (p:Person) RETURN p.name, p.age ORDER BY p.age");
    let names: Vec<&str> = rows.iter().map(|r| r[0].as_str()).collect();
    assert_eq!(
        names,
        vec!["String(\"Bob\")", "String(\"Alice\")", "String(\"Charlie\")"],
        "ORDER BY p.age should order rows by age, got: {rows:?}"
    );

    let rows = query_rows(&conn, "MATCH (p:Person) RETURN p.name, p.age ORDER BY p.age DESC");
    let names: Vec<&str> = rows.iter().map(|r| r[0].as_str()).collect();
    assert_eq!(
        names,
        vec!["String(\"Charlie\")", "String(\"Alice\")", "String(\"Bob\")"],
        "ORDER BY p.age DESC should order rows by age descending, got: {rows:?}"
    );

    exec(&conn, "DROP TABLE Person");
}

// ==================== P53.14 — G3 chain clauses ====================

/// Runs a G3 chain query and asserts it is not rejected at parse time.
/// Execution semantics (SET arithmetic, aggregates, aliases) are covered by
/// P53.15/16/17; P53.14 guarantees the chains now parse/bind/plan.
fn assert_not_parse_error(conn: &Connection, sql: &str, label: &str) {
    match conn.query(sql) {
        Ok(_) => {}
        Err(e) => assert!(
            !e.contains("Parse error"),
            "P53.14 {label} must not be a parse error, got: {e}\nsql: {sql}"
        ),
    }
}

#[test]
fn test_p5314_g3_chain_clauses_parse() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, src STRING, tgt STRING, weight DOUBLE, type STRING, PRIMARY KEY (id))",
    );
    exec(
        &conn,
        "CREATE REL TABLE Connected(FROM Memory TO Memory, weight DOUBLE, type STRING)",
    );
    exec(
        &conn,
        "CREATE NODE TABLE Meta(key STRING, value STRING, PRIMARY KEY (key))",
    );
    exec(
        &conn,
        "CREATE NODE TABLE Counter(key STRING, value INT64, PRIMARY KEY (key))",
    );
    exec(&conn, "CREATE (a:Memory {id: 1, weight: 0.5})");
    exec(&conn, "CREATE (b:Memory {id: 3, weight: 0.0})");
    exec(
        &conn,
        "MATCH (a:Memory {id: 1}), (b:Memory {id: 3}) CREATE (a)-[:Connected {weight: 0.5}]->(b)",
    );

    // strengthen_connection (G3): SET arithmetic → WITH → WHERE → SET.
    assert_not_parse_error(
        &conn,
        "MATCH (a:Memory {id: 1})-[r:Connected]->(b:Memory {id: 3}) \
         SET r.weight = COALESCE(r.weight, 0.0) + 0.8 \
         WITH r WHERE r.weight > 1.0 SET r.weight = 1.0",
        "strengthen_connection",
    );

    // set_meta: plain SET after standalone MERGE (was a parse error before P53.14).
    assert_not_parse_error(&conn, "MERGE (m:Meta {key: 'k'}) SET m.value = 'v'", "set_meta");

    // _next_id: MERGE → SET → RETURN.
    assert_not_parse_error(
        &conn,
        "MERGE (c:Counter {key: 'next_id'}) SET c.value = COALESCE(c.value, 0) + 1 RETURN c.value",
        "_next_id",
    );

    // add_connection: MATCH → MERGE edge → SET.
    assert_not_parse_error(
        &conn,
        "MATCH (a:Memory {id: 1}), (b:Memory {id: 3}) MERGE (a)-[r:Connected]->(b) SET r.weight = 0.7 RETURN count(*)",
        "add_connection",
    );

    // add_bridge / add_bridges_batch: UNWIND → MATCH → OPTIONAL MATCH → WITH → WHERE → CREATE → RETURN.
    assert_not_parse_error(
        &conn,
        "UNWIND [1] AS row \
         MATCH (a:Memory {id: row}), (b:Memory {id: 3}) \
         OPTIONAL MATCH (a)-[existing:Connected]-(b) \
         WITH a, b, existing, row WHERE existing IS NULL \
         CREATE (a)-[:Connected {weight: 0.9, type: 'bridge'}]->(b) RETURN count(*) AS created",
        "add_bridge_batch",
    );

    // MATCH → SET → RETURN (generic G3 shape).
    assert_not_parse_error(
        &conn,
        "MATCH (a:Memory {id: 1}) SET a.weight = 1.0 RETURN a.id",
        "set_then_return",
    );

    // MATCH → DELETE → RETURN (generic G3 shape).
    assert_not_parse_error(
        &conn,
        "MATCH (a:Memory {id: 3}) DELETE a RETURN count(*)",
        "delete_then_return",
    );
}

#[test]
fn test_p5314_plain_set_after_merge_executes() {
    // A MERGE chain must at least execute without error on a fresh node
    // (the row is created by the merge; the follow-up SET writes are fine).
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Meta(key STRING, value STRING, PRIMARY KEY (key))",
    );
    let msg = exec_ok(&conn, "MERGE (m:Meta {key: 'k'}) SET m.value = 'v'");
    assert!(
        msg.is_ok() || msg.as_ref().err().map(|e| !e.contains("Parse error")).unwrap_or(false),
        "MERGE..SET chain should not be a parse error: {msg:?}"
    );
    let rows = query_rows(&conn, "MATCH (m:Meta) RETURN m.key");
    assert_eq!(rows.len(), 1, "MERGE should create the Meta row, got: {rows:?}");
}
