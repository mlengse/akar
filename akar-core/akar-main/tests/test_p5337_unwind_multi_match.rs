//! P53.37a — UNWIND variable propagation through multi-pattern MATCH.
//!
//! The Kairos `add_connections_batch` query uses:
//!   UNWIND $batch AS row MATCH (a:Memory {id: row.src}),(b:Memory {id: row.tgt})
//!   MERGE (a)-[r:Connected]->(b) SET r.weight = 1.0 ...
//!
//! A2: `Variable 'row' not found in chunk field_names` — the UNWIND variable
//!     must survive through the CrossProduct of two node scans.
//! Root cause (2026-08-17): the optimizer's join-reorder pass
//! (`akar-optimizer/src/join_order.rs`) had no `Unwind` arm in
//! `collect_scans_recursive`/`get_scan_alias`, so `reorder_joins_segment`
//! rebuilt the join tree from the node scans only and silently dropped the
//! UNWIND operator — the pipeline lost the `row` binding entirely.

mod common;
use common::*;

fn setup_two_nodes() -> (std::sync::Arc<Database>, Connection) {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, PRIMARY KEY (id))",
    );
    exec(
        &conn,
        "CREATE REL TABLE Connected(FROM Memory TO Memory, weight DOUBLE, type STRING)",
    );
    exec(&conn, "CREATE (:Memory {id: 1, content: 'a'})");
    exec(&conn, "CREATE (:Memory {id: 2, content: 'b'})");
    (_db, conn)
}

/// Minimal reproduction of A2: UNWIND + two MATCH patterns — check field_names.
#[test]
fn test_p5337_unwind_two_match_must_resolve_variable() {
    let (_db, conn) = setup_two_nodes();
    let rows = query_rows(
        &conn,
        "UNWIND [{src: 1, tgt: 2}] AS row \
         MATCH (a:Memory {id: row.src}), (b:Memory {id: row.tgt}) \
         RETURN a.id, b.id",
    );
    assert_eq!(
        rows,
        vec![vec!["Int64(1)".to_string(), "Int64(2)".to_string()]],
        "UNWIND variable must be resolved in two-MATCH pattern, got: {rows:?}"
    );
}

/// A2 end-to-end, kairos `add_connections_batch` shape: UNWIND → two MATCH →
/// MERGE edge (with edge property from the UNWIND row) → SET.
#[test]
fn test_p5337_unwind_two_match_merge_creates_edge() {
    let (_db, conn) = setup_two_nodes();
    exec(
        &conn,
        "UNWIND [{src: 1, tgt: 2, type: 'similar'}] AS row \
         MATCH (a:Memory {id: row.src}), (b:Memory {id: row.tgt}) \
         MERGE (a)-[r:Connected {type: row.type}]->(b) SET r.weight = 1.0",
    );
    let rows = query_rows(
        &conn,
        "MATCH (a:Memory)-[r:Connected]->(b:Memory) RETURN r.weight, r.type",
    );
    assert_eq!(
        rows,
        vec![vec!["Double(1.0)".to_string(), "String(\"similar\")".to_string()]],
        "MERGE in UNWIND pipeline must create edge with weight+type, got: {rows:?}"
    );
}

/// A3: SET in UNWIND→MATCH→MERGE pipeline must apply the value.
#[test]
fn test_p5337_unwind_match_merge_set_weight_applied() {
    let (_db, conn) = setup_two_nodes();
    // Pre-create an edge with weight 0.9
    exec(
        &conn,
        "MATCH (a:Memory {id: 1}), (b:Memory {id: 2}) \
         CREATE (a)-[r:Connected {weight: 0.9, type: 'similar'}]->(b)",
    );
    // UNWIND→MATCH→SET must update the weight
    exec(
        &conn,
        "UNWIND [{src: 1, tgt: 2, type: 'similar'}] AS e \
         MATCH (a:Memory {id: e.src})-[r:Connected]->(b:Memory {id: e.tgt}) \
         SET r.weight = 1.0",
    );
    let rows = query_rows(&conn, "MATCH (a:Memory)-[r:Connected]->(b:Memory) RETURN r.weight");
    assert_eq!(
        rows,
        vec![vec!["Double(1.0)".to_string()]],
        "SET must apply in UNWIND→MATCH→SET pipeline, got: {rows:?}"
    );
}

/// A3c: kairos `batch_strengthen_connections` shape — SET with a CASE
/// expression that reads the UNWIND variable (`e.delta`) and a RETURN that
/// projects UNWIND + edge columns.
#[test]
fn test_p5337_unwind_match_set_case_reads_unwind_var() {
    let (_db, conn) = setup_two_nodes();
    // Pre-create an edge with weight 0.9
    exec(
        &conn,
        "MATCH (a:Memory {id: 1}), (b:Memory {id: 2}) \
         CREATE (a)-[r:Connected {weight: 0.9, type: 'similar'}]->(b)",
    );
    // kairos batch_strengthen_connections: UNWIND → MATCH(rel) → SET CASE
    // (clamp to 1.0) → RETURN e.src/e.tgt/r.weight.
    let rows = query_rows(
        &conn,
        "UNWIND [{src: 1, tgt: 2, delta: 0.05}] AS e \
         MATCH (a:Memory {id: e.src})-[r:Connected]->(b:Memory {id: e.tgt}) \
         SET r.weight = \
           CASE WHEN COALESCE(r.weight, 0.0) + e.delta > 1.0 \
           THEN 1.0 ELSE COALESCE(r.weight, 0.0) + e.delta END \
         RETURN e.src AS src, e.tgt AS tgt, r.weight AS new_w",
    );
    assert_eq!(
        rows,
        vec![vec![
            "Int64(1)".to_string(),
            "Int64(2)".to_string(),
            "Double(0.9500000000000001)".to_string(),
        ]],
        "SET CASE must read e.delta and write the new weight, got: {rows:?}"
    );
}

/// A3b: UNWIND + MATCH + SET with per-row values from UNWIND variable.
#[test]
fn test_p5337_unwind_match_set_per_row_value() {
    let (_db, conn) = setup_two_nodes();
    exec(
        &conn,
        "UNWIND [{id: 1, content: 'v42'}, {id: 2, content: 'v99'}] AS row \
         MATCH (m:Memory {id: row.id}) SET m.content = row.content",
    );
    let rows = query_rows(&conn, "MATCH (m:Memory) RETURN m.id, m.content ORDER BY m.id");
    assert_eq!(
        rows,
        vec![
            vec!["Int64(1)".to_string(), "String(\"v42\")".to_string()],
            vec!["Int64(2)".to_string(), "String(\"v99\")".to_string()],
        ],
        "SET must write per-UNWIND-element values, got: {rows:?}"
    );
}

/// A2/A3 combined: multiple UNWIND rows — each must produce its own edge
/// through the two-MATCH join (kairos `add_connections_batch` with 2 pairs).
#[test]
fn test_p5337_unwind_two_match_merge_multiple_rows() {
    let (_db, conn) = setup_two_nodes();
    exec(
        &conn,
        "UNWIND [{src: 1, tgt: 2, type: 'similar'}, {src: 2, tgt: 1, type: 'bridge'}] AS row \
         MATCH (a:Memory {id: row.src}), (b:Memory {id: row.tgt}) \
         MERGE (a)-[r:Connected {type: row.type}]->(b) SET r.weight = 1.0",
    );
    let rows = query_rows(
        &conn,
        "MATCH (a:Memory)-[r:Connected]->(b:Memory) RETURN r.type ORDER BY r.type",
    );
    assert_eq!(
        rows,
        vec![
            vec!["String(\"bridge\")".to_string()],
            vec!["String(\"similar\")".to_string()],
        ],
        "each UNWIND row must create its own edge, got: {rows:?}"
    );
}
