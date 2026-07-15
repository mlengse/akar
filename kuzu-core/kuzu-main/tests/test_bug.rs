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
    let filter_val = 6;
    let q1 = format!("MATCH (a:A)-[:E]->(b:B) WHERE a.prop > {} AND b.prop < {} RETURN a.id, a.prop, b.id, b.prop", filter_val, filter_val + 5);
    let res = conn.query(&q1).unwrap();
    println!("QUERY RESULT ROWS: {}", res.chunks.iter().map(|c| c.size).sum::<usize>());
    for chunk in res.chunks {
        // Look up column indices correctly
        let aid_idx = chunk.field_names.iter().position(|n| n == "a.id").unwrap();
        let aprop_idx = chunk.field_names.iter().position(|n| n == "a.prop").unwrap();
        let bid_idx = chunk.field_names.iter().position(|n| n == "b.id").unwrap();
        let bprop_idx = chunk.field_names.iter().position(|n| n == "b.prop").unwrap();
        for row in chunk.iter_rows() {
            let aid = chunk.get_value(aid_idx, row).unwrap();
            let aprop = chunk.get_value(aprop_idx, row).unwrap();
            let bid = chunk.get_value(bid_idx, row).unwrap();
            let bprop = chunk.get_value(bprop_idx, row).unwrap();
            println!("MATCH: a.id={:?}, a.prop={:?}, b.id={:?}, b.prop={:?}", aid, aprop, bid, bprop);
        }
    }
}
