use kuzu_main::{Connection, Database, SystemConfig};
use proptest::prelude::*;

fn setup_db() -> (std::sync::Arc<Database>, Connection) {
    let db = std::sync::Arc::new(Database::new(":memory:", SystemConfig::default()).unwrap());
    let conn = Connection::new(&db);
    (db, conn)
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
                for field in &chunk.fields {
                    if let Some(v) = field.get_value(row) {
                        out.push_str(&format!("{:?}", v));
                    }
                }
            }
        }
        // The expected value format depends on Value representation (e.g., Int64(X))
        let expected = format!("Int64({})", val);
        assert!(out.contains(&expected));
    }
}
