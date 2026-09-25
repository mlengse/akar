//! P131 — Repro: parametrized UNWIND variable is lost inside a MATCH write
//! pattern, so `insert_edges` (akar-main/src/bulk.rs) cannot use the batched
//! form:
//!   UNWIND $rows AS r MATCH (a:M {id: r.source}), (b:M {id: r.target})
//!   MERGE (a)-[e:R]->(b) SET {...} RETURN count(e) AS written
//! Every shape fails with `Variable 'r' not found in chunk field_names`.

mod common;
use common::*;

fn setup() -> (std::sync::Arc<Database>, Connection) {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE M(id INT64, content STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE REL TABLE R(FROM M TO M, weight DOUBLE, type STRING)");
    exec(&conn, "CREATE (:M {id: 1, content: 'a'})");
    exec(&conn, "CREATE (:M {id: 2, content: 'b'})");
    (_db, conn)
}

fn two_edge_payload() -> Value {
    Value::List(vec![
        Value::Struct(vec![
            ("source".into(), Value::Int64(1)),
            ("target".into(), Value::Int64(2)),
            ("weight".into(), Value::Double(1.0)),
            ("type".into(), Value::String("similar".into())),
        ]),
        Value::Struct(vec![
            ("source".into(), Value::Int64(2)),
            ("target".into(), Value::Int64(1)),
            ("weight".into(), Value::Double(1.0)),
            ("type".into(), Value::String("bridge".into())),
        ]),
    ])
}

/// The `insert_edges` batch target: UNWIND $rows + two-MATCH + MERGE edge.
#[test]
fn p131_param_unwind_two_match_merge_batch() {
    let (_db, conn) = setup();
    let prepared = conn
        .prepare(
            "UNWIND $rows AS r \
             MATCH (a:M {id: r.source}), (b:M {id: r.target}) \
             MERGE (a)-[e:R]->(b) SET e.weight = r.weight, e.type = r.type \
             RETURN count(e) AS written",
        )
        .unwrap();
    let res = conn.execute(&prepared, vec![("rows", two_edge_payload())]);
    match res {
        Ok(_result) => {}
        Err(e) => panic!("P131 batch form errored: {e}"),
    }
    let rows = query_rows(&conn, "MATCH (a:M)-[e:R]->(b:M) RETURN e.type ORDER BY e.type");
    assert_eq!(
        rows,
        vec![
            vec!["String(\"bridge\")".to_string()],
            vec!["String(\"similar\")".to_string()],
        ],
        "each UNWIND row must create its own edge, got: {rows:?}"
    );
}

/// The WHERE-variant of the batch shape (second failing form in bulk.rs docs).
#[test]
fn p131_param_unwind_where_two_match_create() {
    let (_db, conn) = setup();
    let prepared = conn
        .prepare(
            "UNWIND $rows AS r \
             MATCH (a:M) WHERE a.id = r.source \
             MATCH (b:M) WHERE b.id = r.target \
             CREATE (a)-[e:R {weight: r.weight, type: r.type}]->(b)",
        )
        .unwrap();
    let res = conn.execute(&prepared, vec![("rows", two_edge_payload())]);
    match res {
        Ok(_result) => {}
        Err(e) => panic!("P131 WHERE form errored: {e}"),
    }
    let rows = query_rows(&conn, "MATCH (a:M)-[e:R]->(b:M) RETURN e.type ORDER BY e.type");
    assert_eq!(
        rows,
        vec![
            vec!["String(\"bridge\")".to_string()],
            vec!["String(\"similar\")".to_string()],
        ],
        "each UNWIND row must create its own edge, got: {rows:?}"
    );
}
