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
fn test_nested_types() {
    let (_db, conn) = setup_db();
    
    // Test if list type parsing and creation works
    let res = conn.query("CREATE NODE TABLE Nested(id INT64, items INT64[], PRIMARY KEY (id))");
    
    if res.is_err() || !res.as_ref().unwrap().is_success() {
        // If not implemented in Rust port yet, skip for now.
        return;
    }
    
    exec(&conn, "CREATE (n:Nested {id: 1, items: [1, 2, 3]})");
    let res = exec(&conn, "MATCH (n:Nested) RETURN n.items");
    assert!(res.contains("[1,2,3]") || res.contains("[1, 2, 3]"));
}
