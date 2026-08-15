//! P53.13 — LIMIT/ORDER BY must preserve complex (List) column values (G2).
//!
//! The copy-chunk path of `PhysicalLimit` (partial chunk), `PhysicalOrderBy`,
//! and `PhysicalTopK` used to round-trip through the legacy `ValueVector`,
//! whose `store_value_in_vector` silently drops `Value::List` to NULL. Scalar
//! columns survived; complex columns (e.g. `FLOAT[]` embeddings) collapsed.
//!
//! Fix: those operators slice source fields via Arrow `take`
//! (`take_global_rows`), which preserves List/Struct columns.
//!
//! Regression test uses 3 Memory rows in a single scan chunk so the
//! partial-chunk path of LIMIT and the ORDER BY materialization both trigger.

mod common;

use common::*;

/// Insert three Memory rows; embeddings differ per id so we can verify that
/// the list travels with the right row.
fn setup(conn: &Connection) {
    exec(
        conn,
        "CREATE NODE TABLE Memory (id INT64, embedding FLOAT[], PRIMARY KEY (id))",
    );
    exec(conn, "CREATE (m:Memory {id: 1, embedding: [0.1, 0.2, 0.3]})");
    exec(conn, "CREATE (m:Memory {id: 2, embedding: [0.4, 0.5, 0.6]})");
    exec(conn, "CREATE (m:Memory {id: 3, embedding: [0.7, 0.8, 0.9]})");
}

/// Extract (id, embedding_len, first_element) triples from a query result.
fn embedding_rows(conn: &Connection, sql: &str) -> Vec<(i64, usize, f64)> {
    let result = conn.query(sql).unwrap();
    let mut out = Vec::new();
    for chunk in &result.chunks {
        for row in chunk.iter_rows() {
            let id = chunk.get_value(0, row).expect("id must not be null");
            let id = match id {
                Value::Int64(n) => n,
                other => panic!("expected Int64 id, got {other:?}"),
            };
            let emb = chunk
                .get_value(1, row)
                .expect("embedding must not be null (was NULL before fix)");
            let (len, first) = match emb {
                Value::List(items) => {
                    assert_eq!(items.len(), 3, "embedding must have 3 elements, got {items:?}");
                    let first = match &items[0] {
                        Value::Float(f) => *f as f64,
                        Value::Double(d) => *d,
                        other => panic!("expected numeric embedding element, got {other:?}"),
                    };
                    (items.len(), first)
                }
                other => panic!("expected Value::List, got {other:?}"),
            };
            out.push((id, len, first));
        }
    }
    out
}

#[test]
fn limit_partial_chunk_preserves_list_values() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // G2 probe: LIMIT 1 on a 3-row scan chunk must truncate the chunk without
    // collapsing the FLOAT[] column to NULL.
    let got = embedding_rows(&conn, "MATCH (m:Memory) RETURN m.id, m.embedding LIMIT 1");
    assert_eq!(got, vec![(1, 3, 0.1)], "LIMIT 1 keeps the embedding list");
}

#[test]
fn limit_with_offset_preserves_list_values() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // Partial-chunk path with a non-zero start offset inside the chunk.
    let got = embedding_rows(&conn, "MATCH (m:Memory) RETURN m.id, m.embedding LIMIT 2 SKIP 1");
    assert_eq!(
        got,
        vec![(2, 3, 0.4), (3, 3, 0.7)],
        "OFFSET+LIMIT keeps the embedding lists"
    );
}

#[test]
fn order_by_preserves_list_values() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // ORDER BY re-materializes all output chunks; complex columns must survive.
    let got = embedding_rows(&conn, "MATCH (m:Memory) RETURN m.id, m.embedding ORDER BY m.id DESC");
    assert_eq!(
        got,
        vec![(3, 3, 0.7), (2, 3, 0.4), (1, 3, 0.1)],
        "ORDER BY DESC keeps the embedding lists"
    );
}

#[test]
fn order_by_asc_preserves_list_values() {
    let (_db, conn) = setup_db();
    setup(&conn);

    let got = embedding_rows(&conn, "MATCH (m:Memory) RETURN m.id, m.embedding ORDER BY m.id ASC");
    assert_eq!(
        got,
        vec![(1, 3, 0.1), (2, 3, 0.4), (3, 3, 0.7)],
        "ORDER BY ASC keeps the embedding lists"
    );
}

#[test]
fn order_by_limit_preserves_list_values() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // Fused ORDER BY + LIMIT (TopK) output path.
    let got = embedding_rows(
        &conn,
        "MATCH (m:Memory) RETURN m.id, m.embedding ORDER BY m.id ASC LIMIT 2",
    );
    assert_eq!(got, vec![(1, 3, 0.1), (2, 3, 0.4)], "TOP-K keeps the embedding lists");
}
