use kuzu_main::{Connection, Database, SystemConfig};

#[test]
fn test_bug() {
    let db = std::sync::Arc::new(Database::new(":memory:", SystemConfig::default()).unwrap());
    let conn = Connection::new(&db);
    
    conn.query("CREATE NODE TABLE A(id INT64, PRIMARY KEY(id))").unwrap();
    conn.query("CREATE NODE TABLE B(id INT64, PRIMARY KEY(id))").unwrap();
    conn.query("CREATE NODE TABLE C(id INT64, PRIMARY KEY(id))").unwrap();
    conn.query("CREATE REL TABLE AB(FROM A TO B, dummy INT64)").unwrap();
    conn.query("CREATE REL TABLE BC(FROM B TO C, dummy INT64)").unwrap();

    conn.query("CREATE (a:A {id: 0})").unwrap();
    conn.query("CREATE (b:B {id: 0})").unwrap();
    conn.query("CREATE (c:C {id: 0})").unwrap();
    conn.query("CREATE (c:C {id: 1})").unwrap();

    conn.query("MATCH (a:A {id: 0}), (b:B {id: 0}) CREATE (a)-[:AB]->(b)").unwrap();
    conn.query("MATCH (b:B {id: 0}), (c:C {id: 0}) CREATE (b)-[:BC]->(c)").unwrap();

    let res = conn.query("MATCH (a:A)-[:AB]->(b:B)-[:BC]->(c:C) RETURN a.id, b.id, c.id").unwrap();
    println!("RESULT:");
    for chunk in &res.chunks {
        for row in chunk.iter_rows() {
            println!("{:?} {:?} {:?}", chunk.get_value(0, row), chunk.get_value(1, row), chunk.get_value(2, row));
        }
    }
}
