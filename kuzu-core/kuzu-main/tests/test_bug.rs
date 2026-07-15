use kuzu_main::{Connection, Database, SystemConfig};

#[test]
fn test_bug() {
    let dir = tempfile::tempdir().unwrap();
    let db = std::sync::Arc::new(Database::new(dir.path().to_str().unwrap(), SystemConfig::default()).unwrap());
    let conn = Connection::new(&db);
    conn.query("CREATE NODE TABLE A(id INT64, prop INT64, PRIMARY KEY(id))").unwrap();
    conn.query("CREATE NODE TABLE B(id INT64, prop INT64, PRIMARY KEY(id))").unwrap();
    conn.query("CREATE REL TABLE E(FROM A TO B, dummy INT64)").unwrap();

    let nodes = 12;
    for i in 0..nodes {
        conn.query(&format!("CREATE (a:A {{id: {}, prop: {}}})", i, i % 20)).unwrap();
        conn.query(&format!("CREATE (b:B {{id: {}, prop: {}}})", i, i % 20)).unwrap();
    }

    let edges = nodes;
    for i in 0..edges {
        let from = (i * 3) % nodes;
        let to = (i * 5) % nodes;
        println!("INSERTING {} -> {}", from, to);
        conn.query(&format!("MATCH (a:A {{id: {}}}), (b:B {{id: {}}}) CREATE (a)-[:E]->(b)", from, to)).unwrap();
    }

    let q1 = "MATCH (a:A)-[:E]->(b:B) RETURN a.id, b.id";
    let res = conn.query(&q1).unwrap();
    println!("QUERY RESULT ROWS: {}", res.chunks.iter().map(|c| c.size).sum::<usize>());
    for chunk in res.chunks {
        for row in chunk.iter_rows() {
            let aid = chunk.get_value(0, row).unwrap();
            let bid = chunk.get_value(1, row).unwrap();
            println!("MATCH: a.id={:?}, b.id={:?}", aid, bid);
        }
    }
}
