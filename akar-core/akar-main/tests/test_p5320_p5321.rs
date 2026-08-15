//! P53.20 / P53.21 regression tests — MERGE edge + OPTIONAL MATCH label-less nodes.

mod common;
use common::*;

#[test]
fn test_p5320_merge_edge_label_less() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Memory(id INT64, content STRING, PRIMARY KEY (id))");
    exec(
        &conn,
        "CREATE REL TABLE Connected(FROM Memory TO Memory, type STRING, weight DOUBLE, \
         created_at DOUBLE, event_time DOUBLE, ingestion_time DOUBLE, valid_from DOUBLE)",
    );
    exec(&conn, "CREATE (a:Memory {id: 1, content: 'x'})");
    exec(&conn, "CREATE (b:Memory {id: 3, content: 'y'})");

    // add_connection (Kairos): first call creates the edge and applies SET.
    exec(
        &conn,
        "MATCH (a:Memory {id: 1}), (b:Memory {id: 3}) MERGE (a)-[r:Connected {type: 'similar'}]->(b) \
         SET r.weight = 0.7, r.created_at = 1000.0",
    );
    let rows = query_rows(
        &conn,
        "MATCH (a:Memory {id: 1})-[r:Connected]->(b:Memory {id: 3}) RETURN r.type, r.weight, r.created_at",
    );
    assert_eq!(rows.len(), 1, "one edge expected after first MERGE, got: {rows:?}");
    assert!(rows[0][0].contains("similar"), "type should be 'similar', got: {rows:?}");
    assert!(rows[0][1].contains("0.7"), "weight should be 0.7, got: {rows:?}");
    assert!(rows[0][2].contains("1000"), "created_at should be 1000, got: {rows:?}");

    // Second call (same type, different weight/timestamp) must match, not
    // duplicate, and update the existing edge.
    exec(
        &conn,
        "MATCH (a:Memory {id: 1}), (b:Memory {id: 3}) MERGE (a)-[r:Connected {type: 'similar'}]->(b) \
         SET r.weight = 0.9, r.created_at = 2000.0",
    );
    let rows = query_rows(
        &conn,
        "MATCH (a:Memory {id: 1})-[r:Connected]->(b:Memory {id: 3}) RETURN r.type, r.weight, r.created_at",
    );
    assert_eq!(rows.len(), 1, "no duplicate edge expected, got: {rows:?}");
    assert!(rows[0][1].contains("0.9"), "weight should be 0.9 after update, got: {rows:?}");
    assert!(rows[0][2].contains("2000"), "created_at should be 2000 after update, got: {rows:?}");

    // A different type must create a separate edge.
    exec(
        &conn,
        "MATCH (a:Memory {id: 1}), (b:Memory {id: 3}) MERGE (a)-[r:Connected {type: 'bridge'}]->(b) \
         SET r.weight = 0.5",
    );
    let rows = query_rows(
        &conn,
        "MATCH (a:Memory {id: 1})-[r:Connected]->(b:Memory {id: 3}) RETURN r.type, r.weight",
    );
    assert_eq!(rows.len(), 2, "two edges with different types expected, got: {rows:?}");
}

#[test]
fn test_p5321_optional_match_existing_var() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Memory(id INT64, content STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE REL TABLE Connected(FROM Memory TO Memory, type STRING, weight DOUBLE)");
    exec(&conn, "CREATE (a:Memory {id: 1, content: 'x'})");
    exec(&conn, "CREATE (b:Memory {id: 3, content: 'y'})");

    // add_bridge_batch shape: OPTIONAL MATCH reuses bound label-less nodes.
    // P53.21 scope = binding: the query must bind (no "already defined" /
    // "Bind error"). Full execution of the OPTIONAL MATCH → CREATE chain is
    // tracked as a follow-up engine task.
    let msg = exec_ok(
        &conn,
        "UNWIND [1] AS row \
         MATCH (a:Memory {id: row}), (b:Memory {id: 3}) \
         OPTIONAL MATCH (a)-[existing:Connected]-(b) \
         WITH a, b, existing, row WHERE existing IS NULL \
         CREATE (a)-[:Connected {weight: 0.9, type: 'bridge'}]->(b) RETURN count(*) AS created",
    );
    match msg {
        Ok(_) => {}
        Err(e) => {
            assert!(
                !e.contains("already defined")
                    && !e.contains("Bind error")
                    && !e.contains("Parse error"),
                "P53.21 must not be a parse/bind error, got: {e}"
            );
        }
    }
}
