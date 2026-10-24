//! Secondary-index tests (FR-001 – FR-006).
//!
//! These cover declaration on a populated table, equality and set-membership
//! lookups, index maintenance on later inserts, `DROP INDEX`, scan fallback and
//! survival across a restart.
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

/// Render the serialized physical plan for a statement.
fn explain_plan(conn: &Connection, sql: &str) -> String {
    let result = conn.query(&format!("EXPLAIN {sql}")).unwrap();
    assert!(result.is_success(), "EXPLAIN failed: {:?}", result.error_message);
    let mut lines = Vec::new();
    for chunk in &result.chunks {
        for row in 0..chunk.size {
            for column in 0..chunk.fields.len() {
                match chunk.get_value(column, row) {
                    Some(Value::String(text)) => lines.push(text),
                    Some(other) => lines.push(format!("{other:?}")),
                    None => {}
                }
            }
        }
    }
    lines.join("\n")
}

fn create_memory_table(conn: &Connection) {
    exec(
        conn,
        "CREATE NODE TABLE Memory (id INT64, content_hash STRING, title STRING, PRIMARY KEY(id))",
    );
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

    exec(
        &conn,
        "CREATE (p:Person {id: 1, email: 'a@example.com', name: 'Alice'})",
    );
    exec(&conn, "CREATE (p:Person {id: 2, email: 'b@example.com', name: 'Bob'})");

    // No index on name — full scan must still return correct results.
    let res = query_column_ids(&conn, "MATCH (p:Person {name: 'Bob'}) RETURN p.id ORDER BY p.id");
    assert_eq!(res, vec![2], "full scan on unindexed property must return row 2");
}

/// T046 / FR-004: Count all rows via full scan.
#[test]
fn test_no_index_full_scan_correct() {
    let (_db, conn) = setup();
    create_memory_table(&conn);

    exec(&conn, "CREATE (m:Memory {id: 1, content_hash: 'h1', title: 'Alpha'})");
    exec(&conn, "CREATE (m:Memory {id: 2, content_hash: 'h2', title: 'Beta'})");
    exec(&conn, "CREATE (m:Memory {id: 3, content_hash: 'h3', title: 'Gamma'})");

    let count = count_rows(&conn, "MATCH (m:Memory) RETURN m.id");
    assert_eq!(count, 3, "full scan count must return all 3 rows");
}

/// FR-001 / FR-003: a HASH index can be declared on a populated table, indexes
/// the existing rows, and repeating the declaration is a no-op.
#[test]
fn test_index_creation_indexes_existing_rows_and_is_repeatable() {
    let (_db, conn) = setup();
    create_memory_table(&conn);

    exec(
        &conn,
        "CREATE (m:Memory {id: 1, content_hash: 'hash-aaa', title: 'First'})",
    );
    exec(
        &conn,
        "CREATE (m:Memory {id: 2, content_hash: 'hash-aaa', title: 'First-dup'})",
    );
    exec(
        &conn,
        "CREATE (m:Memory {id: 3, content_hash: 'hash-bbb', title: 'Second'})",
    );

    exec(&conn, "CREATE HASH INDEX idx_hash FOR (m:Memory) ON (m.content_hash)");
    // Repeatable without data loss (FR-003).
    exec(
        &conn,
        "CREATE HASH INDEX IF NOT EXISTS idx_hash FOR (m:Memory) ON (m.content_hash)",
    );

    let ids = query_column_ids(
        &conn,
        "MATCH (m:Memory) WHERE m.content_hash = 'hash-aaa' RETURN m.id ORDER BY m.id",
    );
    assert_eq!(ids, vec![1, 2], "index must cover rows written before it was declared");
    assert_eq!(count_rows(&conn, "MATCH (m:Memory) RETURN m.id"), 3);
}

/// FR-002 / FR-006: equality and set-membership predicates are answered from
/// the index, and the plan reports the index scan.
#[test]
fn test_dedup_uses_secondary_index() {
    let (_db, conn) = setup();
    create_memory_table(&conn);

    exec(
        &conn,
        "CREATE (m:Memory {id: 1, content_hash: 'hash-aaa', title: 'First'})",
    );
    exec(
        &conn,
        "CREATE (m:Memory {id: 2, content_hash: 'hash-aaa', title: 'First-dup'})",
    );
    exec(
        &conn,
        "CREATE (m:Memory {id: 3, content_hash: 'hash-bbb', title: 'Second'})",
    );
    exec(&conn, "CREATE HASH INDEX idx_hash FOR (m:Memory) ON (m.content_hash)");

    // Equality.
    let equal = query_column_ids(
        &conn,
        "MATCH (m:Memory) WHERE m.content_hash = 'hash-aaa' RETURN m.id ORDER BY m.id",
    );
    assert_eq!(equal, vec![1, 2]);

    // Set membership, parameterised exactly as the dedup path sends it.
    let result = conn
        .execute_params(
            "MATCH (m:Memory) WHERE m.content_hash IN $hashes RETURN m.id AS id ORDER BY id",
            vec![(
                "hashes",
                Value::List(vec![
                    Value::String("hash-aaa".into()),
                    Value::String("hash-bbb".into()),
                    Value::String("hash-none".into()),
                ]),
            )],
        )
        .unwrap();
    let mut ids: Vec<i64> = result
        .chunks
        .iter()
        .flat_map(|chunk| (0..chunk.size).filter_map(|row| chunk.get_value(0, row)))
        .filter_map(|value| match value {
            Value::Int64(id) => Some(id),
            _ => None,
        })
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2, 3], "set-membership must return every matching row");

    // The plan must show the index scan rather than a full scan (FR-002).
    let plan = explain_plan(
        &conn,
        "MATCH (m:Memory) WHERE m.content_hash IN ['hash-aaa'] RETURN m.id",
    );
    assert!(
        plan.contains("IndexScan"),
        "EXPLAIN must report index use for an indexed equality predicate:\n{plan}"
    );
}

/// FR-006: rows written after the index exists are findable through it.
#[test]
fn test_index_is_maintained_on_later_inserts() {
    let (_db, conn) = setup();
    create_memory_table(&conn);
    exec(&conn, "CREATE HASH INDEX idx_hash FOR (m:Memory) ON (m.content_hash)");

    exec(
        &conn,
        "CREATE (m:Memory {id: 10, content_hash: 'late-1', title: 'Late'})",
    );

    let ids = query_column_ids(&conn, "MATCH (m:Memory) WHERE m.content_hash IN ['late-1'] RETURN m.id");
    assert_eq!(ids, vec![10], "an insert after CREATE INDEX must be indexed");
}

/// FR-001: dropping an index keeps queries correct and releases the index.
#[test]
fn test_drop_index_keeps_queries_correct() {
    let (_db, conn) = setup();
    create_memory_table(&conn);

    exec(
        &conn,
        "CREATE (m:Memory {id: 1, content_hash: 'hash-aaa', title: 'First'})",
    );
    exec(&conn, "CREATE HASH INDEX idx_hash FOR (m:Memory) ON (m.content_hash)");
    exec(&conn, "DROP INDEX idx_hash ON Memory");

    let ids = query_column_ids(&conn, "MATCH (m:Memory) WHERE m.content_hash = 'hash-aaa' RETURN m.id");
    assert_eq!(ids, vec![1], "scan fallback must stay correct after a drop");

    let plan = explain_plan(&conn, "MATCH (m:Memory) WHERE m.content_hash = 'hash-aaa' RETURN m.id");
    assert!(
        !plan.contains("IndexScan"),
        "a dropped index must not be used by the planner:\n{plan}"
    );
}

/// FR-005: a declared index is discoverable through the introspection surface.
#[test]
fn test_show_indexes_lists_the_secondary_index() {
    let (_db, conn) = setup();
    create_memory_table(&conn);
    exec(&conn, "CREATE HASH INDEX idx_hash FOR (m:Memory) ON (m.content_hash)");

    let result = conn.query("CALL show_indexes()").unwrap();
    assert!(result.is_success(), "show_indexes failed: {:?}", result.error_message);
    let listing = result
        .chunks
        .iter()
        .flat_map(|chunk| {
            (0..chunk.size).flat_map(move |row| {
                (0..chunk.fields.len()).filter_map(move |column| match chunk.get_value(column, row) {
                    Some(Value::String(text)) => Some(text),
                    _ => None,
                })
            })
        })
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(
        listing.contains("idx_hash") && listing.contains("content_hash"),
        "show_indexes must list the declared secondary index: {listing}"
    );
}

/// FR-005: a declared index survives a reopen without being re-declared.
#[test]
fn test_index_survives_restart() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test_db");
    {
        let database = Arc::new(Database::new(db_path.clone(), SystemConfig::default()).unwrap());
        let conn = Connection::new(&database);
        create_memory_table(&conn);
        exec(
            &conn,
            "CREATE (m:Memory {id: 1, content_hash: 'hash-aaa', title: 'First'})",
        );
        exec(&conn, "CREATE HASH INDEX idx_hash FOR (m:Memory) ON (m.content_hash)");
    }

    let database = Arc::new(Database::new(db_path, SystemConfig::default()).unwrap());
    let conn = Connection::new(&database);
    let ids = query_column_ids(&conn, "MATCH (m:Memory) WHERE m.content_hash = 'hash-aaa' RETURN m.id");
    assert_eq!(ids, vec![1], "restored index must answer the same predicate");
}
