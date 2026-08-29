//! P80 — `CREATE NODE TABLE` must accept an array-with-dimension column type
//! such as `embedding FLOAT[384]` end-to-end.
//!
//! Before this fix the parser/binder only accepted the empty-bracket form
//! `FLOAT[]`; `FLOAT[384]` (with a capacity) failed at parse with
//! "expected map_type, struct_type, or union_type" / at bind with
//! "Unknown type", so a fresh daemon could not self-bootstrap a `Memory`
//! table whose embedding column is declared `FLOAT[384]`.
//!
//! Path A (P80) makes the engine treat `FLOAT[384]` identically to `FLOAT[]`
//! (a `List` column); the numeric dimension is not tracked by the engine —
//! that mirrors the Python translator, and vector dims continue to be declared
//! separately via `CREATE VECTOR INDEX ... WITH (dims=N)`.
//!
//! This file is intentionally NOT feature-gated (the DDL fix is core).

mod common;

use akar_common::types::Value;
use common::*;

/// The kairos bootstrap column shape: a `FLOAT[384]` embedding column must
/// create a `Memory` table, accept an embedding list literal, and read it back
/// as a `Value::List`.
#[test]
fn create_node_table_float_with_dims_bootstraps_memory() {
    let (_db, conn) = setup_db();

    exec(
        &conn,
        "CREATE NODE TABLE Memory (id INT64, embedding FLOAT[384], salience DOUBLE, PRIMARY KEY (id))",
    );

    // The engine treats `FLOAT[384]` as a List column, so inserting a list and
    // reading it back must round-trip like a plain `FLOAT[]` column.
    exec(
        &conn,
        "CREATE (m:Memory {id: 1, embedding: [0.1, 0.2, 0.3], salience: 0.5})",
    );

    let result = conn.query("MATCH (m:Memory) RETURN m.id, m.embedding").expect("query");
    let chunk = result.chunks.first().expect("one chunk");
    assert_eq!(chunk.size, 1, "one row expected");
    let emb = chunk.get_value(1, 0).expect("embedding must not be null");
    match emb {
        Value::List(items) => {
            assert_eq!(items.len(), 3, "embedding list length");
        }
        other => panic!("expected Value::List embedding, got {other:?}"),
    }
}
