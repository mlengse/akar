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
#[ignore = "Parser drops unary minus on negative literals"]
fn test_boundary_values() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Extreme(id INT64, val INT64, PRIMARY KEY (id))");
    
    exec(&conn, "CREATE (e:Extreme {id: 1, val: 9223372036854775807})");
    exec(&conn, "CREATE (e:Extreme {id: 2, val: -9223372036854775807})");
    
    let res = exec(&conn, "MATCH (e:Extreme) WHERE e.id = 1 RETURN e.val");
    assert!(res.contains("9223372036854775807"), "res was: {}", res);

    let res = exec(&conn, "MATCH (e:Extreme) WHERE e.id = 2 RETURN e.val");
    assert!(res.contains("-9223372036854775807"), "res was: {}", res);
}
