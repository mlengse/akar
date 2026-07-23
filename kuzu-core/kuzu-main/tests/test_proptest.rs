use kuzu_main::{Connection, Database, SystemConfig};
use proptest::prelude::*;

fn setup_db() -> (tempfile::TempDir, std::sync::Arc<Database>, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let db = std::sync::Arc::new(Database::new(dir.path().to_str().unwrap(), SystemConfig::default()).unwrap());
    let conn = Connection::new(&db);
    (dir, db, conn)
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
        let (_dir, _db, conn) = setup_db();
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
        c_nodes in 1..20i64
    ) {
        let (_dir, _db, conn) = setup_db();
        conn.query("CREATE NODE TABLE A(id INT64, PRIMARY KEY(id))").unwrap();
        conn.query("CREATE NODE TABLE B(id INT64, PRIMARY KEY(id))").unwrap();
        conn.query("CREATE NODE TABLE C(id INT64, PRIMARY KEY(id))").unwrap();
        conn.query("CREATE REL TABLE AB(FROM A TO B, dummy INT64)").unwrap();
        conn.query("CREATE REL TABLE BC(FROM B TO C, dummy INT64)").unwrap();

        // Create nodes
        for i in 0..a_nodes { conn.query(&format!("CREATE (a:A {{id: {}}})", i)).unwrap(); }
        for i in 0..b_nodes { conn.query(&format!("CREATE (b:B {{id: {}}})", i)).unwrap(); }
        for i in 0..c_nodes { conn.query(&format!("CREATE (c:C {{id: {}}})", i)).unwrap(); }

        let ab_edges = a_nodes;
        let bc_edges = b_nodes;
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

        // Compare query result with manual join in Rust
        let q1 = "MATCH (a:A)-[:AB]->(b:B)-[:BC]->(c:C) RETURN COUNT(*)";
        let c1 = extract_count(&conn, q1);

        let mut expected_count = 0;
        for i in 0..ab_edges {
            let _ab_from = (i * 7) % a_nodes;
            let ab_to = (i * 11) % b_nodes;
            for j in 0..bc_edges {
                let bc_from = (j * 13) % b_nodes;
                let _bc_to = (j * 17) % c_nodes;
                if ab_to == bc_from {
                    expected_count += 1;
                }
            }
        }

        prop_assert_eq!(c1, expected_count);
    }

    #[test]
    fn test_filter_pushdown_equivalence(
        nodes in 5..30i64,
        filter_val in 2..15i64
    ) {
        let (_dir, _db, conn) = setup_db();
        conn.query("CREATE NODE TABLE A(id INT64, prop INT64, PRIMARY KEY(id))").unwrap();
        conn.query("CREATE NODE TABLE B(id INT64, prop INT64, PRIMARY KEY(id))").unwrap();
        conn.query("CREATE REL TABLE E(FROM A TO B, dummy INT64)").unwrap();

        for i in 0..nodes {
            conn.query(&format!("CREATE (a:A {{id: {}, prop: {}}})", i, i % 20)).unwrap();
            conn.query(&format!("CREATE (b:B {{id: {}, prop: {}}})", i, i % 20)).unwrap();
        }

        let edges = nodes;
        for i in 0..edges {
            let from = (i * 3) % nodes;
            let to = (i * 5) % nodes;
            conn.query(&format!("MATCH (a:A {{id: {}}}), (b:B {{id: {}}}) CREATE (a)-[:E]->(b)", from, to)).unwrap();
        }

        // Compare with manual evaluation
        let q1 = format!("MATCH (a:A)-[:E]->(b:B) WHERE a.prop > {} AND b.prop < {} RETURN COUNT(*)", filter_val, filter_val + 5);
        let c1 = extract_count(&conn, &q1);

        let mut expected_count = 0;
        for i in 0..edges {
            let from = (i * 3) % nodes;
            let to = (i * 5) % nodes;
            let from_prop = from % 20;
            let to_prop = to % 20;
            if from_prop > filter_val && to_prop < filter_val + 5 {
                expected_count += 1;
            }
        }

        prop_assert_eq!(c1, expected_count);
    }
}
