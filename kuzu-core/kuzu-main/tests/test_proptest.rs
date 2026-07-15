use kuzu_main::{Connection, Database, SystemConfig};
use proptest::prelude::*;

fn setup_db() -> (std::sync::Arc<Database>, Connection) {
    let db = std::sync::Arc::new(Database::new(":memory:", SystemConfig::default()).unwrap());
    let conn = Connection::new(&db);
    (db, conn)
}

fn extract_count(conn: &Connection, query: &str) -> i64 {
    let res = conn.query(query).unwrap();
    let mut count = 0;
    for chunk in &res.chunks {
        for row in chunk.iter_rows() {
            if let Some(kuzu_common::types::Value::Int64(c)) = chunk.get_value(0, row) {
                count = c;
            }
        }
    }
    count
}

proptest! {
    #[test]
    fn test_roundtrip_i64(val in 0..i64::MAX) {
        let (_db, conn) = setup_db();
        conn.query("CREATE NODE TABLE T(id INT64, val INT64, PRIMARY KEY(id))").unwrap();
        conn.query(&format!("CREATE (t:T {{id: 1, val: {}}})", val)).unwrap();
        
        let res = conn.query("MATCH (t:T) RETURN t.val").unwrap();
        let mut out = String::new();
        for chunk in &res.chunks {
            for row in chunk.iter_rows() {
                for (col_idx, _) in chunk.fields.iter().enumerate() {
                    if let Some(v) = chunk.get_value(col_idx, row) {
                        out.push_str(&format!("{:?}", v));
                    }
                }
            }
        }
        let expected = format!("Int64({})", val);
        assert!(out.contains(&expected));
    }

    #[test]
    fn test_join_associativity(
        a_nodes in 1..20i64,
        b_nodes in 1..20i64,
        c_nodes in 1..20i64,
        ab_edges in 1..40i64,
        bc_edges in 1..40i64
    ) {
        let (_db, conn) = setup_db();
        conn.query("CREATE NODE TABLE A(id INT64, PRIMARY KEY(id))").unwrap();
        conn.query("CREATE NODE TABLE B(id INT64, PRIMARY KEY(id))").unwrap();
        conn.query("CREATE NODE TABLE C(id INT64, PRIMARY KEY(id))").unwrap();
        conn.query("CREATE REL TABLE AB(FROM A TO B)").unwrap();
        conn.query("CREATE REL TABLE BC(FROM B TO C)").unwrap();

        // Create nodes
        for i in 0..a_nodes { conn.query(&format!("CREATE (a:A {{id: {}}})", i)).unwrap(); }
        for i in 0..b_nodes { conn.query(&format!("CREATE (b:B {{id: {}}})", i)).unwrap(); }
        for i in 0..c_nodes { conn.query(&format!("CREATE (c:C {{id: {}}})", i)).unwrap(); }

        // Create edges (deterministic pseudorandom based on properties)
        for i in 0..ab_edges {
            let from = (i * 7) % a_nodes;
            let to = (i * 11) % b_nodes;
            conn.query(&format!("MATCH (a:A {{id: {}}}), (b:B {{id: {}}}) CREATE (a)-[:AB]->(b)", from, to)).unwrap();
        }
        for i in 0..bc_edges {
            let from = (i * 13) % b_nodes;
            let to = (i * 17) % c_nodes;
            conn.query(&format!("MATCH (b:B {{id: {}}}), (c:C {{id: {}}}) CREATE (b)-[:BC]->(c)", from, to)).unwrap();
        }

        // Query 1: (A JOIN B) JOIN C (implied by left-to-right evaluation)
        let q1 = "MATCH (a:A)-[:AB]->(b:B)-[:BC]->(c:C) RETURN COUNT(*)";
        let c1 = extract_count(&conn, q1);

        // Query 2: A JOIN (B JOIN C)
        // We force associativity change using WITH
        let q2 = "MATCH (b:B)-[:BC]->(c:C) WITH b MATCH (a:A)-[:AB]->(b) RETURN COUNT(*)";
        let c2 = extract_count(&conn, q2);

        prop_assert_eq!(c1, c2);
    }

    #[test]
    fn test_filter_pushdown_equivalence(
        nodes in 5..30i64,
        edges in 5..50i64,
        filter_val in 2..15i64
    ) {
        let (_db, conn) = setup_db();
        conn.query("CREATE NODE TABLE A(id INT64, prop INT64, PRIMARY KEY(id))").unwrap();
        conn.query("CREATE NODE TABLE B(id INT64, prop INT64, PRIMARY KEY(id))").unwrap();
        conn.query("CREATE REL TABLE E(FROM A TO B)").unwrap();

        for i in 0..nodes {
            conn.query(&format!("CREATE (a:A {{id: {}, prop: {}}})", i, i % 20)).unwrap();
            conn.query(&format!("CREATE (b:B {{id: {}, prop: {}}})", i, i % 20)).unwrap();
        }

        for i in 0..edges {
            let from = (i * 3) % nodes;
            let to = (i * 5) % nodes;
            conn.query(&format!("MATCH (a:A {{id: {}}}), (b:B {{id: {}}}) CREATE (a)-[:E]->(b)", from, to)).unwrap();
        }

        // Query 1: Filter AFTER join (conceptually, written without where)
        let q1 = format!("MATCH (a:A)-[:E]->(b:B) WHERE a.prop > {} AND b.prop < {} RETURN COUNT(*)", filter_val, filter_val + 5);
        let c1 = extract_count(&conn, &q1);

        // Query 2: Filter BEFORE join (forced via WITH)
        let q2 = format!("MATCH (a:A) WHERE a.prop > {} WITH a MATCH (b:B) WHERE b.prop < {} WITH a, b MATCH (a)-[:E]->(b) RETURN COUNT(*)", filter_val, filter_val + 5);
        let c2 = extract_count(&conn, &q2);

        prop_assert_eq!(c1, c2);
    }
}
