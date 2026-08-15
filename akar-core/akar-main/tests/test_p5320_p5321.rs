//! P53.20 / P53.21 / P53.25 regression tests — MERGE edge, OPTIONAL MATCH
//! label-less nodes, and the Kairos `add_bridge_batch` UNWIND→MATCH→OPTIONAL
//! MATCH→CREATE pipeline.

mod common;
use common::*;

#[test]
fn test_p5320_merge_edge_label_less() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, PRIMARY KEY (id))",
    );
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
    assert!(
        rows[0][0].contains("similar"),
        "type should be 'similar', got: {rows:?}"
    );
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
    assert!(
        rows[0][1].contains("0.9"),
        "weight should be 0.9 after update, got: {rows:?}"
    );
    assert!(
        rows[0][2].contains("2000"),
        "created_at should be 2000 after update, got: {rows:?}"
    );

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
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, PRIMARY KEY (id))",
    );
    exec(
        &conn,
        "CREATE REL TABLE Connected(FROM Memory TO Memory, type STRING, weight DOUBLE)",
    );
    exec(&conn, "CREATE (a:Memory {id: 1, content: 'x'})");
    exec(&conn, "CREATE (b:Memory {id: 3, content: 'y'})");

    // add_bridge_batch shape: OPTIONAL MATCH reuses bound label-less nodes.
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
                !e.contains("already defined") && !e.contains("Bind error") && !e.contains("Parse error"),
                "P53.21 must not be a parse/bind error, got: {e}"
            );
        }
    }
}

#[test]
fn test_p5325_add_bridge_batch_creates_edge_once() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, PRIMARY KEY (id))",
    );
    exec(
        &conn,
        "CREATE REL TABLE Connected(FROM Memory TO Memory, type STRING, weight DOUBLE)",
    );
    exec(&conn, "CREATE (a:Memory {id: 1, content: 'x'})");
    exec(&conn, "CREATE (b:Memory {id: 3, content: 'y'})");

    // Kairos add_bridge_batch shape: UNWIND → MATCH both bound nodes → OPTIONAL
    // MATCH the undirected edge → CREATE it only when missing → RETURN the
    // number of edges created. P53.25: previously this silently executed as a
    // no-op (0 edges) or errored on the UNWIND/MATCH join.
    let q = "UNWIND [1] AS row \
         MATCH (a:Memory {id: row}), (b:Memory {id: 3}) \
         OPTIONAL MATCH (a)-[existing:Connected]-(b) \
         WITH a, b, existing, row WHERE existing IS NULL \
         CREATE (a)-[:Connected {weight: 0.9, type: 'bridge'}]->(b) RETURN count(*) AS created";

    // First call: no edge yet → CREATE runs once, count = 1.
    let rows1 = query_rows(&conn, q);
    assert_eq!(
        rows1,
        vec![vec!["Int64(1)".to_string()]],
        "first call should create 1 edge, got {rows1:?}"
    );
    let edges = query_rows(
        &conn,
        "MATCH (a:Memory {id: 1})-[r:Connected]->(b:Memory {id: 3}) RETURN r.type, r.weight",
    );
    assert_eq!(edges.len(), 1, "one edge expected after first call, got {edges:?}");
    assert!(edges[0][0].contains("bridge"), "type should be 'bridge', got {edges:?}");
    assert!(edges[0][1].contains("0.9"), "weight should be 0.9, got {edges:?}");

    // Second call: edge exists → WHERE existing IS NULL drops the row, no
    // duplicate CREATE, count = 0.
    let rows2 = query_rows(&conn, q);
    assert_eq!(
        rows2,
        vec![vec!["Int64(0)".to_string()]],
        "second call should create 0 edges, got {rows2:?}"
    );
    let edges2 = query_rows(
        &conn,
        "MATCH (a:Memory {id: 1})-[r:Connected]->(b:Memory {id: 3}) RETURN r.type, r.weight",
    );
    assert_eq!(edges2.len(), 1, "no duplicate edge expected, got {edges2:?}");
}

#[test]
fn test_p5325_optional_match_probe_sees_existing_edge() {
    // Positive probe path: when the edge already exists, `existing` is bound
    // and `existing IS NOT NULL` keeps the row (the OptionalExtend adjacency
    // probe emits the edge property columns instead of NULLs).
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, PRIMARY KEY (id))",
    );
    exec(
        &conn,
        "CREATE REL TABLE Connected(FROM Memory TO Memory, type STRING, weight DOUBLE)",
    );
    exec(&conn, "CREATE (a:Memory {id: 1, content: 'x'})");
    exec(&conn, "CREATE (b:Memory {id: 3, content: 'y'})");
    exec(
        &conn,
        "MATCH (a:Memory {id: 1}), (b:Memory {id: 3}) CREATE (a)-[:Connected {type: 'similar', weight: 0.5}]->(b)",
    );

    let rows = query_rows(
        &conn,
        "UNWIND [1] AS row \
         MATCH (a:Memory {id: row}), (b:Memory {id: 3}) \
         OPTIONAL MATCH (a)-[existing:Connected]-(b) \
         WITH a, b, existing, row WHERE existing IS NOT NULL \
         RETURN existing.type, existing.weight",
    );
    assert_eq!(rows.len(), 1, "existing edge should be visible, got {rows:?}");
    assert!(rows[0][0].contains("similar"), "type should be 'similar', got {rows:?}");
    assert!(rows[0][1].contains("0.5"), "weight should be 0.5, got {rows:?}");
}

#[test]
fn test_p5325_optional_match_probe_undirected_both_directions() {
    // Undirected probe (`-[existing:Connected]-`) must find edges created in
    // either direction: a forward edge (a)->(b) matches the probe of (a)-(b)
    // AND the probe of (b)-(a).
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, PRIMARY KEY (id))",
    );
    exec(
        &conn,
        "CREATE REL TABLE Connected(FROM Memory TO Memory, type STRING, weight DOUBLE)",
    );
    exec(&conn, "CREATE (a:Memory {id: 1, content: 'x'})");
    exec(&conn, "CREATE (b:Memory {id: 3, content: 'y'})");
    exec(
        &conn,
        "MATCH (a:Memory {id: 1}), (b:Memory {id: 3}) CREATE (a)-[:Connected {type: 'forward', weight: 0.4}]->(b)",
    );

    // Probe from b's perspective (reverse lookup).
    let rows = query_rows(
        &conn,
        "UNWIND [3] AS row \
         MATCH (a:Memory {id: row}), (b:Memory {id: 1}) \
         OPTIONAL MATCH (a)-[existing:Connected]-(b) \
         WITH a, b, existing, row WHERE existing IS NOT NULL \
         RETURN existing.type",
    );
    assert_eq!(rows.len(), 1, "reverse-direction edge should be visible, got {rows:?}");
    assert!(rows[0][0].contains("forward"), "type should be 'forward', got {rows:?}");
}
