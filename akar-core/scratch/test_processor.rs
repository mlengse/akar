use akar_main::{Connection, Database, SystemConfig};

fn main() {
    let db = std::sync::Arc::new(Database::new(":memory:", SystemConfig::default()).unwrap());
    let conn = Connection::new(&db);
    conn.query("CREATE NODE TABLE User(id INT64, name STRING, PRIMARY KEY (id))").unwrap();
    conn.query("CREATE NODE TABLE Post(id INT64, content STRING, PRIMARY KEY (id))").unwrap();
    conn.query("CREATE REL TABLE Likes(FROM User TO Post, since INT64)").unwrap();
    conn.query("CREATE (u:User {id: 1, name: 'Alice'})").unwrap();
    conn.query("CREATE (u:User {id: 2, name: 'Bob'})").unwrap();
    conn.query("CREATE (p:Post {id: 10, content: 'Hello'})").unwrap();
    conn.query("CREATE (p:Post {id: 20, content: 'World'})").unwrap();
    
    println!("=== RUNNING MATCH CREATE ===");
    let result = conn.query("MATCH (u:User {id: 1}), (p:Post {id: 10}) CREATE (u)-[:Likes]->(p)");
    if let Err(e) = result {
        println!("ERROR: {}", e);
    } else {
        println!("SUCCESS");
    }
}
