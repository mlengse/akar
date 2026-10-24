//! Typed multi-edge regression tests (FR-013 – FR-017, SC-006).
//!
//! A relationship is identified by `(source, target, type)`: two different
//! types on one pair coexist, re-writing one type is idempotent, type/time
//! round-trip unchanged, and traversal can filter by type.
mod common;
use akar_common::types::Value;
use akar_main::{EdgeRow, RelSpec, insert_edges};
use common::*;

/// Sulur's `Connected` shape: endpoints keyed by `id`, typed edges with a
/// creation time and validity window.
const CONNECTED: RelSpec = RelSpec::new("Connected", "Memory", "Memory");
const PROPERTIES: &[&str] = &["weight", "type", "created_at", "valid_from", "valid_to"];

fn setup() -> (std::sync::Arc<Database>, Connection) {
    let (db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Memory (id INT64, content STRING, PRIMARY KEY (id))",
    );
    exec(
        &conn,
        "CREATE REL TABLE Connected (FROM Memory TO Memory, weight DOUBLE, type STRING, \
         created_at DOUBLE, valid_from DOUBLE, valid_to DOUBLE)",
    );
    exec(&conn, "CREATE (:Memory {id: 1, content: 'A'})");
    exec(&conn, "CREATE (:Memory {id: 2, content: 'B'})");
    (db, conn)
}

fn edge(weight: f64, kind: &str, created_at: f64) -> EdgeRow {
    EdgeRow::with_values(
        1,
        2,
        vec![
            Value::Double(weight),
            Value::String(kind.to_string()),
            Value::Double(created_at),
            Value::Double(1000.0),
            Value::Double(2000.0),
        ],
    )
}

fn edge_count(conn: &Connection) -> usize {
    conn.query("MATCH (a:Memory)-[r:Connected]->(b:Memory) RETURN r.type")
        .unwrap()
        .chunks
        .first()
        .map(|chunk| chunk.size)
        .unwrap_or(0)
}

fn text(value: &str) -> String {
    value.to_string()
}

/// FR-013: both types on one ordered pair persist.
#[test]
fn two_types_on_the_same_pair_both_persist() {
    let (_db, conn) = setup();
    let written = insert_edges(
        &conn,
        &CONNECTED,
        PROPERTIES,
        &[edge(0.5, "similar", 100.0), edge(0.9, "contradicts", 200.0)],
    )
    .unwrap();
    assert_eq!(written, 2, "both edges must be reported written");
    assert_eq!(edge_count(&conn), 2, "both typed edges must be stored");

    let rows = query_rows(
        &conn,
        "MATCH (a:Memory)-[r:Connected]->(b:Memory) RETURN r.type ORDER BY r.type",
    );
    assert_eq!(
        rows,
        vec![vec![text("String(\"contradicts\")")], vec![text("String(\"similar\")")]]
    );
}

/// FR-014: re-writing the same endpoints and type updates in place.
#[test]
fn rewriting_the_same_type_is_idempotent() {
    let (_db, conn) = setup();
    insert_edges(&conn, &CONNECTED, PROPERTIES, &[edge(0.5, "similar", 100.0)]).unwrap();
    insert_edges(&conn, &CONNECTED, PROPERTIES, &[edge(0.8, "similar", 300.0)]).unwrap();

    assert_eq!(edge_count(&conn), 1, "the rewrite must not create a second edge");
    let rows = query_rows(
        &conn,
        "MATCH (a:Memory)-[r:Connected]->(b:Memory) RETURN r.weight, r.created_at",
    );
    assert_eq!(
        rows,
        vec![vec![text("Double(0.8)"), text("Double(100.0)")]],
        "the update path applies the new weight and preserves the original creation time"
    );
}

/// FR-016: traversal filters by relationship type while the other type stays
/// stored.
#[test]
fn traversal_can_filter_by_type() {
    let (_db, conn) = setup();
    insert_edges(
        &conn,
        &CONNECTED,
        PROPERTIES,
        &[edge(0.5, "similar", 100.0), edge(0.9, "contradicts", 200.0)],
    )
    .unwrap();

    let filtered = query_rows(
        &conn,
        "MATCH (a:Memory)-[r:Connected]->(b:Memory) WHERE r.type = 'contradicts' RETURN r.type",
    );
    assert_eq!(filtered.len(), 1, "only the requested type is returned");
    assert_eq!(edge_count(&conn), 2, "the other type remains stored");
}

/// FR-017: type, creation time and validity window round-trip unchanged.
#[test]
fn type_and_temporal_fields_round_trip() {
    let (_db, conn) = setup();
    insert_edges(&conn, &CONNECTED, PROPERTIES, &[edge(0.5, "similar", 123.0)]).unwrap();

    let rows = query_rows(
        &conn,
        "MATCH (a:Memory)-[r:Connected]->(b:Memory) RETURN r.type, r.created_at, r.valid_from, r.valid_to",
    );
    assert_eq!(
        rows,
        vec![vec![
            text("String(\"similar\")"),
            text("Double(123.0)"),
            text("Double(1000.0)"),
            text("Double(2000.0)"),
        ]]
    );
}

/// FR-015 / SC-006: MERGE on an existing primary key takes the update path and
/// never raises a duplicate-primary-key error.
#[test]
fn merge_on_existing_pk_updates_without_a_duplicate_error() {
    let (_db, conn) = setup();
    exec(&conn, "MERGE (m:Memory {id: 1}) SET m.content = 'A-updated'");

    let rows = query_rows(&conn, "MATCH (m:Memory {id: 1}) RETURN m.content");
    assert_eq!(rows, vec![vec![text("String(\"A-updated\")")]]);
    assert_eq!(
        query_rows(&conn, "MATCH (m:Memory) RETURN m.id").len(),
        2,
        "no duplicate node was created"
    );
}

/// FR-015 / SC-006: concurrent merges for the same key leave exactly one node.
#[test]
fn concurrent_merges_leave_one_node() {
    let (db, conn) = setup();
    let handles: Vec<_> = (0..8)
        .map(|worker| {
            let database = std::sync::Arc::clone(&db);
            std::thread::spawn(move || {
                let connection = Connection::new(&database);
                connection
                    .query(&format!("MERGE (m:Memory {{id: 99}}) SET m.content = 'w{worker}'"))
                    .map(|result| result.is_success())
                    .unwrap_or(false)
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }

    let rows = query_rows(&conn, "MATCH (m:Memory {id: 99}) RETURN m.id");
    assert_eq!(rows.len(), 1, "exactly one node must exist for the key");
}
