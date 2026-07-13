use kuzu_main::{Connection, Database, SystemConfig};

fn setup_db() -> (std::sync::Arc<Database>, Connection) {
    let db = std::sync::Arc::new(Database::new(":memory:", SystemConfig::default()).unwrap());
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
    let mut out = String::new();
    for chunk in &result.chunks {
        for row in chunk.iter_rows() {
            for field in &chunk.fields {
                if field.is_null(row) {
                    out.push_str("null ");
                } else if let Some(v) = field.get_value(row) {
                    out.push_str(&format!("{:?} ", v));
                }
            }
            out.push('\n');
        }
    }
    out
}

#[test]
fn test_null_properties() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, name STRING, age INT64, PRIMARY KEY (id))");
    
    // Insert node with missing optional property (age is null)
    exec(&conn, "CREATE (p:Person {id: 1, name: 'Alice'})");
    
    let res = exec(&conn, "MATCH (p:Person) RETURN p.age");
    assert!(res.contains("null"), "Expected null in output, got: {}", res);
    
    let res = exec(&conn, "MATCH (p:Person) WHERE p.age IS NULL RETURN p.id");
    assert!(res.contains('1'));

    let res = exec(&conn, "MATCH (p:Person) WHERE p.age IS NOT NULL RETURN p.id");
    assert!(!res.contains('1'));
}
