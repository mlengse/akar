//! P53.37 — `ALTER TABLE ADD` must mirror the new column into the storage
//! table, not just the DDL catalog.
//!
//! kairos `_ensure_schema` widens Memory with dae_* columns via ALTER ADD.
//! Before this fix the storage scan never emitted the added column: `MATCH
//! ... RETURN <added>` fell back to positional projection (garbage/_id leak),
//! CREATE dropped the added values, and `EXPORT DATABASE` produced a parquet
//! whose rows did not match the recreated schema — `repair_schema` lost all
//! rows on import.

mod common;
use common::*;

/// ALTER ADD a column, read it back (NULL for existing rows), then CREATE a
/// row that stores a real value in it.
#[test]
fn test_p5337_alter_add_column_visible_to_scan_and_write() {
    let (_db, conn) = setup_db();
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, PRIMARY KEY (id))",
    );
    exec(&conn, "CREATE (:Memory {id: 1, content: 'a'})");

    // ALTER ADD — previously the storage table kept the old 2-column schema,
    // so this column was invisible to scans.
    let msg = exec(&conn, "ALTER TABLE Memory ADD dae_self_weight DOUBLE");
    assert!(msg.contains("dae_self_weight"), "alter failed: {msg}");

    // Existing rows read NULL in the new column (no error, no positional garbage).
    let rows = query_rows(&conn, "MATCH (m:Memory) RETURN m.id, m.dae_self_weight");
    assert_eq!(
        rows,
        vec![vec!["Int64(1)".to_string(), "null".to_string()]],
        "existing row must see NULL in the added column, got: {rows:?}"
    );

    // New rows can write real values into the added column.
    exec(&conn, "CREATE (:Memory {id: 2, content: 'b', dae_self_weight: 0.5})");
    let rows = query_rows(&conn, "MATCH (m:Memory) RETURN m.id, m.dae_self_weight ORDER BY m.id");
    assert_eq!(
        rows,
        vec![
            vec!["Int64(1)".to_string(), "null".to_string()],
            vec!["Int64(2)".to_string(), "Double(0.5)".to_string()],
        ],
        "new row must store the added column value, got: {rows:?}"
    );
}

/// ALTER ADD a rel-table property column — edges see NULL, new edges store it.
#[test]
fn test_p5337_alter_add_rel_column_visible() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE Memory(id INT64, PRIMARY KEY (id))");
    exec(
        &conn,
        "CREATE REL TABLE Connected(FROM Memory TO Memory, weight DOUBLE)",
    );
    exec(&conn, "CREATE (:Memory {id: 1})");
    exec(&conn, "CREATE (:Memory {id: 2})");
    exec(
        &conn,
        "MATCH (a:Memory {id: 1}), (b:Memory {id: 2}) CREATE (a)-[r:Connected {weight: 0.9}]->(b)",
    );

    exec(&conn, "ALTER TABLE Connected ADD type STRING");
    let rows = query_rows(
        &conn,
        "MATCH (a:Memory)-[r:Connected]->(b:Memory) RETURN r.weight, r.type",
    );
    assert_eq!(
        rows,
        vec![vec!["Double(0.9)".to_string(), "null".to_string()]],
        "existing edge must see NULL in the added rel column, got: {rows:?}"
    );

    exec(
        &conn,
        "MATCH (a:Memory {id: 1}), (b:Memory {id: 2}) CREATE (a)-[r:Connected {weight: 0.1, type: 'similar'}]->(b)",
    );
    let rows = query_rows(
        &conn,
        "MATCH (a:Memory)-[r:Connected]->(b:Memory) RETURN r.weight, r.type ORDER BY r.weight",
    );
    assert_eq!(
        rows,
        vec![
            vec!["Double(0.1)".to_string(), "String(\"similar\")".to_string()],
            vec!["Double(0.9)".to_string(), "null".to_string()],
        ],
        "new edge must store the added rel column value, got: {rows:?}"
    );
}
