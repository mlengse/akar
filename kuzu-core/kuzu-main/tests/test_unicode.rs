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
fn test_unicode_strings() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Person(id INT64, name STRING, PRIMARY KEY (id))");
    
    // Insert unicode characters (emojis, cyrillic, kanji)
    exec(&conn, "CREATE (p:Person {id: 1, name: 'Alice 🚀'})");
    exec(&conn, "CREATE (p:Person {id: 2, name: 'Боб'})");
    exec(&conn, "CREATE (p:Person {id: 3, name: '太郎'})");
    
    let res = exec(&conn, "MATCH (p:Person) WHERE p.id = 1 RETURN p.name");
    assert!(res.contains("Alice 🚀"));
    
    let res = exec(&conn, "MATCH (p:Person) WHERE p.id = 2 RETURN p.name");
    assert!(res.contains("Боб"));
    
    let res = exec(&conn, "MATCH (p:Person) WHERE p.id = 3 RETURN p.name");
    assert!(res.contains("太郎"));
}
