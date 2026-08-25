//! P53.12 — Rel-scan target binding (G1).
//!
//! `MATCH (a:Memory)-[r:Connected]->(b:Memory)` must bind `b` to the
//! *destination* node of the relationship, not the source node's row.
//!
//! Uses primary keys that deliberately differ from row offsets (PKs start
//! at 1 while row offsets start at 0) so the buggy PK→row indirection in
//! `PhysicalExtend` would return `b.id == a.id`.

mod common;

use common::*;

/// Build a small graph where PK != row offset:
///   Memory nodes: id 1 (row 0), id 2 (row 1), id 3 (row 2)
///   Connected:    1 -[weight 0.5]-> 2,  2 -[weight 0.7]-> 3
fn setup(conn: &Connection) {
    exec(conn, "CREATE NODE TABLE Memory(id INT64, PRIMARY KEY(id))");
    exec(conn, "CREATE REL TABLE Connected(FROM Memory TO Memory, weight DOUBLE)");
    exec(conn, "CREATE (a:Memory {id: 1})");
    exec(conn, "CREATE (a:Memory {id: 2})");
    exec(conn, "CREATE (a:Memory {id: 3})");
    exec(
        conn,
        "MATCH (a:Memory {id: 1}), (b:Memory {id: 2}) CREATE (a)-[:Connected {weight: 0.5}]->(b)",
    );
    exec(
        conn,
        "MATCH (a:Memory {id: 2}), (b:Memory {id: 3}) CREATE (a)-[:Connected {weight: 0.7}]->(b)",
    );
}

#[test]
fn test_rel_scan_binds_destination_node() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // G1 probe: b must be the destination node, not a copy of the source row.
    let mut got = query_rows(&conn, "MATCH (a:Memory)-[r:Connected]->(b:Memory) RETURN a.id, b.id");
    got.sort_unstable();
    assert_eq!(
        got,
        vec![
            vec!["Int64(1)".to_string(), "Int64(2)".to_string()],
            vec!["Int64(2)".to_string(), "Int64(3)".to_string()]
        ],
        "destination node binding"
    );
}

#[test]
fn test_rel_scan_carries_dest_properties_and_rel_props() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // r.weight was already correct before the fix; assert it stays correct
    // together with the destination node properties.
    let mut got = query_rows(
        &conn,
        "MATCH (a:Memory)-[r:Connected]->(b:Memory) RETURN a.id, b.id, r.weight",
    );
    got.sort_unstable();
    assert_eq!(
        got,
        vec![
            vec![
                "Int64(1)".to_string(),
                "Int64(2)".to_string(),
                "Double(0.5)".to_string()
            ],
            vec![
                "Int64(2)".to_string(),
                "Int64(3)".to_string(),
                "Double(0.7)".to_string()
            ],
        ]
    );
}

#[test]
fn test_rel_scan_reverse_direction() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // Backward direction: `b` is the source, `a` the destination.
    let mut got = query_rows(&conn, "MATCH (a:Memory)<-[r:Connected]-(b:Memory) RETURN a.id, b.id");
    got.sort_unstable();
    assert_eq!(
        got,
        vec![
            vec!["Int64(2)".to_string(), "Int64(1)".to_string()],
            vec!["Int64(3)".to_string(), "Int64(2)".to_string()]
        ],
        "reverse direction binding"
    );
}

#[test]
fn test_rel_scan_multi_hop() {
    let (_db, conn) = setup_db();
    setup(&conn);

    // Two-hop chain through a non-trivial dst binding.
    let mut got = query_rows(
        &conn,
        "MATCH (a:Memory)-[:Connected]->(b:Memory)-[:Connected]->(c:Memory) RETURN a.id, b.id, c.id",
    );
    got.sort_unstable();
    assert_eq!(
        got,
        vec![vec![
            "Int64(1)".to_string(),
            "Int64(2)".to_string(),
            "Int64(3)".to_string()
        ]]
    );
}

/// P63 — rel-scan must not crash when a destination node carries a string
/// property longer than the legacy 255-byte inline ValueVector cap.
///
/// Reproduced via kairos legacy import (Finding #8): any
/// `MATCH (a)-[r]->(b)` over a Memory node with `content` > 255 bytes made
/// `connection_count()` / `get_connections()` fail with
/// `Cannot store string of N bytes: inline string storage limit is 255 bytes`.
/// The fix routes String columns through the Arrow builder path in
/// PhysicalExtend / PhysicalOptionalExtend / PhysicalPackedExtend.
#[test]
fn test_rel_scan_long_string_dest_property() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, PRIMARY KEY(id))",
    );
    exec(
        &conn,
        "CREATE REL TABLE Connected(FROM Memory TO Memory, weight DOUBLE)",
    );
    let long = "x".repeat(600);
    let create = format!("CREATE (a:Memory {{id: 1, content: '{long}'}})");
    exec(&conn, &create);
    exec(&conn, "CREATE (a:Memory {id: 2, content: 'short'})");
    exec(
        &conn,
        "MATCH (a:Memory {id: 1}), (b:Memory {id: 2}) CREATE (a)-[:Connected {weight: 0.5}]->(b)",
    );

    // The 600-byte content must survive the rel-scan (was: crash). The long
    // content lives on the SOURCE node `a` (id 1); `b` (id 2) is short.
    let rows = query_rows(
        &conn,
        "MATCH (a:Memory)-[r:Connected]->(b:Memory) RETURN a.id, a.content, b.id, b.content",
    );
    assert_eq!(rows.len(), 1, "one edge");
    assert_eq!(rows[0][0], "Int64(1)");
    assert_eq!(rows[0][1], format!("String({long:?})"), "long content preserved");
    assert_eq!(rows[0][2], "Int64(2)");
    assert_eq!(rows[0][3], "String(\"short\")");

    // And a pure count over the rel table must not crash either.
    let got = query_rows(&conn, "MATCH (a:Memory)-[r:Connected]->(b:Memory) RETURN count(r)");
    assert_eq!(got, vec![vec!["Int64(1)".to_string()]]);
}
