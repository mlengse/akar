//! P132 — per-edge addressing for pattern-matched relationships (#36I).
//!
//! A scan of a relationship pattern used to surface only the endpoint node
//! `_id`s, so `DELETE r` / `SET r.…` addressed the rel-table row at the *source
//! node's* offset instead of the matched edge. These tests pin the correct
//! semantics: a scoped delete/update must affect exactly the matched edge.

use akar_main::{Connection, Database, SystemConfig};
use std::sync::Arc;
use tempfile::tempdir;

fn setup() -> Result<(Arc<Database>, Connection, tempfile::TempDir), String> {
    let dir = tempdir().map_err(|e| e.to_string())?;
    let db = Arc::new(Database::new(dir.path().to_str().unwrap(), SystemConfig::default()).map_err(|e| e.to_string())?);
    let conn = Connection::new(&db);
    conn.query("CREATE NODE TABLE Memory (id INT64, content STRING, PRIMARY KEY(id))")?;
    conn.query("CREATE REL TABLE Connected (FROM Memory TO Memory, weight DOUBLE, type STRING)")?;
    for i in 1..=3 {
        conn.query(&format!("CREATE (:Memory {{id: {i}, content: 'm{i}'}})"))?;
    }
    Ok((db, conn, dir))
}

fn edge_count(conn: &Connection, src: i64, dst: Option<i64>) -> Result<i64, String> {
    let q = match dst {
        Some(d) => format!("MATCH (a:Memory {{id: {src}}})-[r:Connected]->(b:Memory {{id: {d}}}) RETURN count(r) AS n"),
        None => format!("MATCH (a:Memory {{id: {src}}})-[r:Connected]->(b:Memory) RETURN count(r) AS n"),
    };
    let res = conn.query(&q)?;
    let chunk = res.chunks.first().ok_or("no chunk")?;
    Ok(chunk.get_i64(0, 0).unwrap_or(0))
}

/// A scoped DELETE must remove exactly the edges the pattern matched — not the
/// edge that happens to sit at the source node's row offset.
#[test]
fn scoped_delete_targets_the_matched_edge() -> Result<(), String> {
    let (_db, conn, _dir) = setup()?;
    // Three edges out of node 1: two to node 2, one to node 3.
    conn.query("MATCH (a:Memory {id: 1}), (b:Memory {id: 2}) CREATE (a)-[:Connected {weight: 1.0, type: 'dup'}]->(b)")?;
    conn.query(
        "MATCH (a:Memory {id: 1}), (b:Memory {id: 3}) CREATE (a)-[:Connected {weight: 2.0, type: 'solo'}]->(b)",
    )?;
    conn.query("MATCH (a:Memory {id: 1}), (b:Memory {id: 2}) CREATE (a)-[:Connected {weight: 3.0, type: 'dup'}]->(b)")?;

    assert_eq!(edge_count(&conn, 1, Some(2))?, 2, "two 1->2 edges before delete");
    assert_eq!(edge_count(&conn, 1, Some(3))?, 1, "one 1->3 edge before delete");

    let res = conn.query("MATCH (a:Memory {id: 1})-[r:Connected]->(b:Memory {id: 2}) DELETE r")?;
    let deleted = res.chunks.first().ok_or("no chunk")?.get_i64(0, 0).unwrap_or(0);
    assert_eq!(deleted, 2, "DELETE r must remove both matched 1->2 edges");

    assert_eq!(edge_count(&conn, 1, Some(2))?, 0, "both matched edges gone");
    assert_eq!(edge_count(&conn, 1, Some(3))?, 1, "unmatched 1->3 edge survives");
    Ok(())
}

/// A predicate-scoped DELETE removes only the edges the predicate matches — the
/// primitive a "collapse N copies to the strongest" dedupe needs
/// (`... WHERE r.weight < $keep DELETE r`).
#[test]
fn predicate_scoped_delete_keeps_the_strongest_edge() -> Result<(), String> {
    let (_db, conn, _dir) = setup()?;
    for w in [0.5_f64, 0.9, 0.3] {
        conn.query(&format!(
            "MATCH (a:Memory {{id: 1}}), (b:Memory {{id: 2}}) CREATE (a)-[:Connected {{weight: {w}, type: 'dup'}}]->(b)"
        ))?;
    }
    assert_eq!(edge_count(&conn, 1, Some(2))?, 3, "three parallel edges before dedupe");

    let res = conn.query("MATCH (a:Memory {id: 1})-[r:Connected]->(b:Memory {id: 2}) WHERE r.weight < 0.9 DELETE r")?;
    let deleted = res.chunks.first().ok_or("no chunk")?.get_i64(0, 0).unwrap_or(0);
    assert_eq!(deleted, 2, "only the two weaker edges are removed");
    assert_eq!(edge_count(&conn, 1, Some(2))?, 1, "one edge survives");

    let kept = conn.query("MATCH (a:Memory {id: 1})-[r:Connected]->(b:Memory {id: 2}) RETURN r.weight")?;
    let chunk = kept.chunks.first().ok_or("no chunk")?;
    assert_eq!(
        chunk.get_value(0, 0),
        Some(akar_common::types::Value::Double(0.9)),
        "the strongest edge is the survivor"
    );
    Ok(())
}

/// A scoped SET must update exactly the matched edges, leaving neighbours at
/// their original values (the previous path silently no-op'd or wrote one row).
#[test]
fn scoped_set_targets_the_matched_edges() -> Result<(), String> {
    let (_db, conn, _dir) = setup()?;
    conn.query("MATCH (a:Memory {id: 1}), (b:Memory {id: 2}) CREATE (a)-[:Connected {weight: 1.0, type: 'dup'}]->(b)")?;
    conn.query(
        "MATCH (a:Memory {id: 1}), (b:Memory {id: 3}) CREATE (a)-[:Connected {weight: 2.0, type: 'solo'}]->(b)",
    )?;
    conn.query("MATCH (a:Memory {id: 1}), (b:Memory {id: 2}) CREATE (a)-[:Connected {weight: 3.0, type: 'dup'}]->(b)")?;

    conn.query("MATCH (a:Memory {id: 1})-[r:Connected]->(b:Memory {id: 2}) SET r.weight = 9.0")?;

    let rows =
        conn.query("MATCH (a:Memory {id: 1})-[r:Connected]->(b:Memory) RETURN b.id, r.weight ORDER BY b.id, r.weight")?;
    let chunk = rows.chunks.first().ok_or("no chunk")?;
    let mut got: Vec<(i64, f64)> = Vec::new();
    for row in 0..chunk.size {
        let dst = chunk.get_i64(0, row).unwrap_or(-1);
        let w = match chunk.get_value(1, row) {
            Some(akar_common::types::Value::Double(d)) => d,
            other => return Err(format!("unexpected weight: {other:?}")),
        };
        got.push((dst, w));
    }
    assert_eq!(
        got,
        vec![(2, 9.0), (2, 9.0), (3, 2.0)],
        "both matched 1->2 edges become 9.0; the 1->3 edge stays 2.0"
    );
    Ok(())
}
