//! P123: typed bulk writes, batched traversal and sharing a pool across threads.
//!
//! These tests run against a real on-disk database through the public API, so
//! they pin the statement shapes the helpers generate (batched `UNWIND`, the
//! `RETURN count(...)` write count, and how a missing endpoint is reported)
//! rather than only the Rust-side encoding.

use akar_common::types::Value;
use akar_main::{
    Connection, ConnectionPool, Database, EdgeRow, RelSpec, SystemConfig, TypedNode, insert_edges, insert_nodes,
    neighbors,
};
use std::sync::Arc;

/// The shape Sulur's formation path writes: one memory row per item.
struct MemoryRow {
    id: i64,
    label: String,
    content: String,
    embedding: Vec<f32>,
    salience: f64,
}

impl TypedNode for MemoryRow {
    const TABLE: &'static str = "Memory";
    const COLUMNS: &'static [&'static str] = &["id", "label", "content", "embedding", "salience"];

    fn to_values(&self) -> Vec<Value> {
        vec![
            Value::Int64(self.id),
            Value::String(self.label.clone()),
            Value::String(self.content.clone()),
            Value::List(self.embedding.iter().map(|f| Value::Double(*f as f64)).collect()),
            Value::Double(self.salience),
        ]
    }
}

fn memory(id: i64) -> MemoryRow {
    MemoryRow {
        id,
        label: format!("memory-{id}"),
        content: format!("content of memory {id}"),
        embedding: vec![id as f32, 0.5, 0.25],
        salience: 0.5,
    }
}

fn open() -> (Arc<Database>, Connection, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Arc::new(Database::new(dir.path().to_str().unwrap(), SystemConfig::default()).expect("open db"));
    let conn = Connection::new(&db);
    conn.query(
        "CREATE NODE TABLE Memory (id INT64, label STRING, content STRING, \
         embedding FLOAT[3], salience DOUBLE, PRIMARY KEY (id))",
    )
    .expect("create Memory");
    conn.query("CREATE REL TABLE Connected (FROM Memory TO Memory, weight DOUBLE, type STRING)")
        .expect("create Connected");
    (db, conn, dir)
}

const CONNECTED: RelSpec = RelSpec::new("Connected", "Memory", "Memory");

#[test]
fn typed_batch_insert_writes_every_row_and_reports_the_count() -> Result<(), String> {
    let (_db, conn, _dir) = open();
    let rows: Vec<MemoryRow> = (1..=250).map(memory).collect();

    // 250 rows over chunks of 100 → 3 statements, none of them per-row.
    let written = insert_nodes(&conn, &rows, 100)?;
    assert_eq!(written, 250, "the engine count must equal the rows submitted");

    let count = conn.query("MATCH (m:Memory) RETURN count(m) AS n")?;
    let chunk = count.chunks.first().ok_or("no chunk")?;
    assert_eq!(chunk.get_value(0, 0), Some(Value::Int64(250)));

    // Spot-check the property encoding, including the list-valued embedding.
    let one = conn.query("MATCH (m:Memory {id: 7}) RETURN m.label AS label, m.embedding AS e")?;
    let chunk = one.chunks.first().ok_or("no chunk")?;
    assert_eq!(chunk.get_value(0, 0), Some(Value::String("memory-7".into())));
    match chunk.get_value(1, 0) {
        Some(Value::List(values)) => assert_eq!(values.len(), 3, "embedding list survives the round trip"),
        other => panic!("embedding came back as {other:?}"),
    }
    Ok(())
}

#[test]
fn typed_batch_insert_rejects_a_column_value_mismatch() -> Result<(), String> {
    struct Broken(u8);
    impl TypedNode for Broken {
        const TABLE: &'static str = "Memory";
        const COLUMNS: &'static [&'static str] = &["id", "label"];
        fn to_values(&self) -> Vec<Value> {
            // One value for two columns: must be caught, not silently shifted.
            vec![Value::Int64(self.0 as i64)]
        }
    }

    let (_db, conn, _dir) = open();
    let error = insert_nodes(&conn, &[Broken(1)], 10).expect_err("mismatch must fail");
    assert!(
        error.contains("to_values returned 1 values for 2 columns"),
        "unexpected error: {error}"
    );
    Ok(())
}

#[test]
fn batched_edge_insert_is_idempotent_and_needs_both_endpoints() -> Result<(), String> {
    let (_db, conn, _dir) = open();
    insert_nodes(&conn, &[memory(1), memory(2), memory(3)], 10)?;

    let properties = ["weight", "type"];
    let edges = vec![
        EdgeRow::with_values(1, 2, vec![Value::Double(0.9), Value::String("similar".into())]),
        EdgeRow::with_values(2, 3, vec![Value::Double(0.4), Value::String("similar".into())]),
        // Endpoint 99 does not exist: the MATCH drops the row.
        EdgeRow::with_values(1, 99, vec![Value::Double(1.0), Value::String("similar".into())]),
    ];

    let written = insert_edges(&conn, &CONNECTED, &properties, &edges)?;
    assert_eq!(written, 2, "only rows with both endpoints exist are written");

    // Re-running the same batch must update in place, not duplicate.
    let again = insert_edges(&conn, &CONNECTED, &properties, &edges)?;
    assert_eq!(again, 2);
    let count = conn.query("MATCH (a:Memory)-[c:Connected]->(b:Memory) RETURN count(c) AS n")?;
    let chunk = count.chunks.first().ok_or("no chunk")?;
    assert_eq!(
        chunk.get_value(0, 0),
        Some(Value::Int64(2)),
        "MERGE must not duplicate edges"
    );
    Ok(())
}

#[test]
fn batched_edge_insert_rejects_a_property_value_mismatch() -> Result<(), String> {
    let (_db, conn, _dir) = open();
    insert_nodes(&conn, &[memory(1), memory(2)], 10)?;

    let edges = vec![EdgeRow::new(1, 2)];
    let error = insert_edges(&conn, &CONNECTED, &["weight"], &edges).expect_err("mismatch must fail");
    assert!(
        error.contains("carries 0 values for 1 properties"),
        "unexpected error: {error}"
    );
    Ok(())
}

#[test]
fn neighbors_fetches_many_nodes_in_one_call_and_limits_per_node() -> Result<(), String> {
    let (_db, conn, _dir) = open();
    insert_nodes(&conn, &(1..=4).map(memory).collect::<Vec<_>>(), 10)?;

    let properties = ["weight", "type"];
    let edges = vec![
        EdgeRow::with_values(1, 2, vec![Value::Double(0.1), Value::String("similar".into())]),
        EdgeRow::with_values(1, 3, vec![Value::Double(0.9), Value::String("similar".into())]),
        EdgeRow::with_values(1, 4, vec![Value::Double(0.5), Value::String("similar".into())]),
        EdgeRow::with_values(2, 3, vec![Value::Double(0.7), Value::String("similar".into())]),
    ];
    insert_edges(&conn, &CONNECTED, &properties, &edges)?;

    let all = neighbors(&conn, &CONNECTED, Some("weight"), &[1, 2], 0, 100)?;
    assert_eq!(all.len(), 4, "both nodes' edges come back: {all:?}");
    let node_one: Vec<f64> = all
        .iter()
        .filter(|n| n.source == 1)
        .map(|n| n.weight.expect("weight requested"))
        .collect();
    assert_eq!(
        node_one,
        vec![0.9, 0.5, 0.1],
        "neighbours are ordered by descending weight"
    );

    let limited = neighbors(&conn, &CONNECTED, Some("weight"), &[1, 2], 2, 100)?;
    assert_eq!(limited.len(), 3, "two for node 1 plus one for node 2: {limited:?}");
    assert!(
        limited
            .iter()
            .filter(|n| n.source == 1)
            .all(|n| n.weight.unwrap() >= 0.5)
    );
    Ok(())
}

#[test]
fn neighbors_handles_property_less_rel_tables_and_duplicate_ids() -> Result<(), String> {
    let (_db, conn, _dir) = open();
    insert_nodes(&conn, &[memory(1), memory(2)], 10)?;
    conn.query("CREATE REL TABLE Links (FROM Memory TO Memory)")?;
    conn.query("MATCH (a:Memory {id: 1}), (b:Memory {id: 2}) CREATE (a)-[:Links]->(b)")?;

    let links = RelSpec::new("Links", "Memory", "Memory");
    // Duplicate ids must not multiply the result.
    let found = neighbors(&conn, &links, None, &[1, 1, 1], 0, 100)?;
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].source, 1);
    assert_eq!(found[0].target, 2);
    assert_eq!(found[0].weight, None, "no weight was requested");

    assert!(neighbors(&conn, &links, None, &[], 0, 100)?.is_empty());
    Ok(())
}

#[test]
fn bulk_helpers_reject_a_zero_row_batch_size() -> Result<(), String> {
    let (_db, conn, _dir) = open();
    assert!(
        insert_nodes(&conn, &[memory(1)], 0).is_err(),
        "zero rows per statement is rejected"
    );
    assert!(neighbors(&conn, &CONNECTED, None, &[1], 0, 0).is_err());
    Ok(())
}

#[test]
fn empty_batches_are_a_no_op_not_an_error() -> Result<(), String> {
    let (_db, conn, _dir) = open();
    assert_eq!(insert_nodes::<MemoryRow>(&conn, &[], 100)?, 0);
    assert_eq!(insert_edges(&conn, &CONNECTED, &[], &[])?, 0);
    assert!(neighbors(&conn, &CONNECTED, None, &[], 0, 100)?.is_empty());
    Ok(())
}

#[test]
fn a_pool_serves_concurrent_writers_on_one_database() -> Result<(), String> {
    // The shape `sulur-server` uses: one Database, one pool, N blocking tasks.
    let (db, conn, _dir) = open();
    drop(conn);
    let pool = Arc::new(ConnectionPool::new(db));

    let handles: Vec<_> = (0..4i64)
        .map(|worker| {
            let pool = Arc::clone(&pool);
            std::thread::spawn(move || -> Result<usize, String> {
                let conn = pool.get();
                let rows: Vec<MemoryRow> = (1..=25).map(|n| memory(worker * 100 + n)).collect();
                insert_nodes(&conn, &rows, 100)
            })
        })
        .collect();

    let mut total = 0;
    for handle in handles {
        total += handle.join().expect("thread joins")?;
    }
    assert_eq!(total, 100, "every worker's rows landed");

    let verify = pool.run(|conn| conn.query("MATCH (m:Memory) RETURN count(m) AS n"))?;
    let chunk = verify.chunks.first().ok_or("no chunk")?;
    assert_eq!(chunk.get_value(0, 0), Some(Value::Int64(100)));
    Ok(())
}
