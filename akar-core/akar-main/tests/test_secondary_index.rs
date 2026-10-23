//! Tests for secondary index dedup (FR-002, FR-004).
//!
//! These tests verify index-backed dedup and full-scan fallback.
//! Note: The current storage layer supports one index per table (the PK's ART index),
//! so secondary indexes on non-PK columns are not yet supported by the processor.
//! These tests document the current behavior and the gap.
use akar_common::types::Value;
use akar_main::{Connection, Database, SystemConfig};
use std::sync::Arc;
use tempfile::tempdir;

fn setup() -> (Arc<Database>, Connection) {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test_db");
    let config = SystemConfig::default();
    let database = Arc::new(Database::new(db_path, config).unwrap());
    let conn = Connection::new(&database);
    std::mem::forget(dir);
    (database, conn)
}

fn exec(conn: &Connection, query: &str) {
    let result = conn.query(query).unwrap();
    assert!(
        result.is_success(),
        "Query failed: {query} -> {:?}",
        result.error_message
    );
}

fn query_column_ids(conn: &Connection, sql: &str) -> Vec<i64> {
    let result = conn.query(sql).unwrap();
    let mut ids: Vec<i64> = result
        .chunks
        .iter()
        .flat_map(|c| (0..c.size).filter_map(|i| c.get_value(0, i)))
        .filter_map(|v| match v {
            Value::Int64(x) => Some(x),
            _ => None,
        })
        .collect();
    ids.sort_unstable();
    ids
}

fn count_rows(conn: &Connection, sql: &str) -> usize {
    conn.query(sql).unwrap().chunks.first().map(|c| c.size).unwrap_or(0)
}

/// T046 / FR-004: Query on an unindexed property falls back to a scan
/// without error and returns correct results.
#[test]
fn test_unindexed_property_falls_back_to_scan() {
    let (_db, conn) = setup();

    exec(
        &conn,
        "CREATE NODE TABLE Person (id INT64, email STRING, name STRING, PRIMARY KEY(id))",
    );

    exec(&conn, "CREATE (p:Person {id: 1, email: 'a@example.com', name: 'Alice'})");
    exec(&conn, "CREATE (p:Person {id: 2, email: 'b@example.com', name: 'Bob'})");

    // No index on name — full scan must still return correct results.
    let res = query_column_ids(
        &conn,
        "MATCH (p:Person {name: 'Bob'}) RETURN p.id ORDER BY p.id",
    );
    assert_eq!(res, vec![2], "full scan on unindexed title must return row 2");
}

/// T046 / FR-004: Count all rows via full scan.
#[test]
fn test_no_index_full_scan_correct() {
    let (_db, conn) = setup();

    exec(
        &conn,
        "CREATE NODE TABLE Memory (id INT64, content_hash STRING, title STRING, PRIMARY KEY(id))",
    );

    exec(
        &conn,
        "CREATE (m:Memory {id: 1, content_hash: 'h1', title: 'Alpha'})",
    );
    exec(
        &conn,
        "CREATE (m:Memory {id: 2, content_hash: 'h2', title: 'Beta'})",
    );
    exec(
        &conn,
        "CREATE (m:Memory {id: 3, content_hash: 'h3', title: 'Gamma'})",
    );

    // Count all rows via full scan.
    let count = count_rows(&conn, "MATCH (m:Memory) RETURN m.id");
    assert_eq!(count, 3, "full scan count must return all 3 rows");
}

/// T046 / FR-002 (pending): Content-hash dedup uses a declared secondary index.
///
/// NOTE: This test is currently blocked because the storage layer only allows
/// one index per table (the PK's ART index). Creating a secondary index on a
/// non-PK column fails with "already has an ART index". The processor's
/// CREATE INDEX handler always calls `create_art_index()` regardless of the
/// requested index type, so HASH indexes on non-PK columns are not yet
/// supported. Once the processor supports secondary indexes (calling
/// `TableCatalog::create_secondary_index` for HASH indexes), this test should
/// verify that dedup by content_hash uses the index rather than a full scan.
#[test]
fn test_dedup_uses_secondary_index() {
    let (_db, conn) = setup();

    exec(
        &conn,
        "CREATE NODE TABLE Memory (id INT64, content_hash STRING, title STRING, PRIMARY KEY(id))",
    );

    exec(
        &conn,
        "CREATE (m:Memory {id: 1, content_hash: 'hash-aaa', title: 'First'})",
    );
    exec(
        &conn,
        "CREATE (m:Memory {id: 2, content_hash: 'hash-aaa', title: 'First-dup'})",
    );

    // Declare a secondary index on content_hash — this currently fails because
    // the table already has an ART index on the PK column (id).
    // Once secondary indexes on non-PK columns are supported, this should
    // succeed and dedup queries should use the index.
    let result = conn.query("CREATE HASH INDEX idx_hash FOR (m:Memory) ON (m.content_hash)");
    assert!(
        result.is_err(),
        "Creating a secondary index on a table with an existing PK index should fail \
         until the storage layer supports multiple indexes per table"
    );
}
