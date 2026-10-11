//! Regression: a leading unary `-` on a numeric literal must be preserved.
//!
//! The grammar is `unary_expr = ("-" | "+")? ~ postfix_expr`, where the sign is
//! an unnamed pest token and therefore absent from `into_inner()`. The AST
//! builder only handled 1- and ≥3-child forms, so the sign was silently dropped
//! before the child `float`/`integer` could see it: `[-1.5, 2.0]` parsed as
//! `[1.5, 2.0]`. That corrupted every negative vector component — including the
//! ANN query literal in `CALL vector_similarity_scan` (P6-RECALL-1 / T025).

use akar_common::types::Value;
use akar_main::{Connection, Database, SystemConfig};
use std::sync::Arc;

fn setup() -> (tempfile::TempDir, Arc<Database>, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Database::new(dir.path().to_str().unwrap(), SystemConfig::default()).unwrap());
    let conn = Connection::new(&db);
    (dir, db, conn)
}

fn first_value(conn: &Connection, sql: &str) -> Value {
    let res = conn.query(sql).expect("query must succeed");
    res.chunks
        .first()
        .expect("one chunk")
        .get_value(0, 0)
        .expect("one value")
}

fn as_f64s(value: &Value) -> Vec<f64> {
    match value {
        Value::List(items) => items
            .iter()
            .map(|v| match v {
                Value::Double(d) => *d,
                Value::Int64(i) => *i as f64,
                other => panic!("unexpected list element {other:?}"),
            })
            .collect(),
        other => panic!("expected List, got {other:?}"),
    }
}

#[test]
fn negative_scalar_literal_preserved() {
    let (_d, _db, conn) = setup();
    assert!(
        matches!(first_value(&conn, "RETURN -3"), Value::Int64(-3)),
        "`RETURN -3` must yield -3"
    );
    match first_value(&conn, "RETURN -1.5") {
        Value::Double(d) => assert!((d + 1.5).abs() < 1e-12, "`RETURN -1.5` must yield -1.5, got {d}"),
        other => panic!("expected Double(-1.5), got {other:?}"),
    }
}

#[test]
fn negative_list_literal_preserved() {
    let (_d, _db, conn) = setup();
    let got = as_f64s(&first_value(&conn, "RETURN [-1.5, 2.0, -0.25]"));
    assert_eq!(got, vec![-1.5, 2.0, -0.25]);
}

#[test]
fn negative_vector_component_roundtrips() {
    let (_d, _db, conn) = setup();
    conn.query("CREATE NODE TABLE Mem(id INT64, vec FLOAT[], PRIMARY KEY (id))")
        .expect("create table");
    conn.query("CREATE (:Mem {id: 1, vec: [-0.5, 0.25, -1.0]})")
        .expect("insert row");
    let got = as_f64s(&first_value(&conn, "MATCH (m:Mem) RETURN m.vec"));
    assert_eq!(got, vec![-0.5, 0.25, -1.0]);
}
