//! P53.34 regression tests — EXPORT/IMPORT DATABASE option parsing (kairos E2).
//!
//! kairos `repair_schema()` sends `IMPORT DATABASE "...path" (format="parquet")`.
//! The grammar only accepted `IMPORT DATABASE <string>`, so the trailing options
//! clause failed with `expected EOI` — the exact harness failure. This suite
//! locks the parse fix AND the hidden sibling bug: EXPORT's options clause was
//! also silently dropped (the `export_options` parent pair was never descended),
//! so `(format="parquet")` never reached the AST and copy.cypher fell back to CSV.

mod common;
use common::*;

#[cfg(feature = "parquet-export")]
#[test]
fn test_p5334_export_import_format_options_roundtrip() {
    let (dir, _db, conn) = setup_db_on_disk();
    // kairos-style schema: Memory node + Connected rel (dependent on Memory) +
    // Counter node.
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, embedding FLOAT[], PRIMARY KEY (id))",
    );
    exec(
        &conn,
        "CREATE REL TABLE Connected(FROM Memory TO Memory, weight DOUBLE)",
    );
    exec(
        &conn,
        "CREATE NODE TABLE Counter(key STRING, value INT64, PRIMARY KEY (key))",
    );
    exec(
        &conn,
        "CREATE (:Memory {id: 1, content: 'hello', embedding: [1.0, 0.0, 0.0]})",
    );
    exec(
        &conn,
        "CREATE (:Memory {id: 2, content: 'world', embedding: [0.0, 1.0, 0.0]})",
    );
    exec(
        &conn,
        "MATCH (a:Memory {id: 1}), (b:Memory {id: 2}) CREATE (a)-[r:Connected {weight: 0.9}]->(b)",
    );

    let export_dir = dir.path().join("backup");
    let p = export_dir.to_string_lossy().replace('\\', "/");

    // EXPORT with Kuzu options clause — format must survive into copy.cypher.
    let msg = exec(&conn, &format!(r#"EXPORT DATABASE "{p}" (format="parquet")"#));
    assert!(msg.contains("exported"), "export failed: {msg}");
    let copy = std::fs::read_to_string(export_dir.join("copy.cypher")).expect("copy.cypher written");
    assert!(
        copy.contains(".parquet"),
        "format option must reach copy.cypher: {copy}"
    );
    assert!(export_dir.join("schema.cypher").exists(), "schema.cypher written");

    // kairos repair flow: DROP the node table while a dependent rel table exists.
    // (kairos sends `DROP TABLE IF EXISTS Memory`; the akar-python wrapper
    // strips IF EXISTS per P53.1, so the engine test uses the plain form.)
    let msg = exec(&conn, "DROP TABLE Memory");
    assert!(msg.to_lowercase().contains("dropped"), "drop failed: {msg}");
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, embedding FLOAT[], PRIMARY KEY (id))",
    );

    // IMPORT with the same options clause — the EOI parse regression.
    let msg = exec(&conn, &format!(r#"IMPORT DATABASE "{p}" (format="parquet")"#));
    assert!(msg.contains("imported"), "import failed: {msg}");

    // Schema still usable; Memory recreated (CREATE may be re-run by schema.cypher
    // and skipped as duplicate, which is tolerated by the importer).
    let rows = query_rows(&conn, "CALL show_tables()");
    let names: Vec<String> = rows.iter().map(|r| r.join(",")).collect();
    assert!(names.iter().any(|n| n.contains("Memory")), "Memory missing: {names:?}");

    // NOTE: data roundtrip via parquet is not asserted here because the parquet
    // writer serializes List (embedding) as Utf8 — a pre-existing serialization
    // gap. The CSV repair flow test (test_p5334b) covers data persistence fully.
}

/// P53.34b — exact kairos `repair_schema()` flow: EXPORT→DROP→CREATE→IMPORT→count.
/// Uses CSV format (no parquet feature required).
#[test]
fn test_p5334b_repair_schema_flow_data_persists_csv() {
    let (dir, _db, conn) = setup_db_on_disk();
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, embedding FLOAT[], PRIMARY KEY (id))",
    );
    exec(
        &conn,
        "CREATE REL TABLE Connected(FROM Memory TO Memory, weight DOUBLE)",
    );
    exec(
        &conn,
        "CREATE NODE TABLE Counter(key STRING, value INT64, PRIMARY KEY (key))",
    );
    // Seed 3 Memory nodes + 1 edge (matches kairos seeded_store fixture).
    for id in 1..=3 {
        exec(
            &conn,
            &format!("CREATE (:Memory {{id: {id}, content: 'hello {id}', embedding: [1.0, 0.0, 0.0]}})"),
        );
    }
    exec(
        &conn,
        "MATCH (a:Memory {id: 1}), (b:Memory {id: 2}) \
         CREATE (a)-[r:Connected {weight: 0.9}]->(b)",
    );
    let res = query_values(&conn, "MATCH (m:Memory) RETURN count(m)");
    assert_eq!(res.trim(), "Int64(3)", "seed count must be 3");

    // ── EXPORT (CSV, no explicit format option) ──────────────
    let export_dir = dir.path().join("repair_csv");
    let p = export_dir.to_string_lossy().replace('\\', "/");
    let msg = exec(&conn, &format!(r#"EXPORT DATABASE "{p}""#));
    assert!(msg.contains("exported"), "export failed: {msg}");
    assert!(
        export_dir.join("Memory.csv").exists(),
        "Memory.csv must exist in export dir"
    );

    // ── DROP + recreate (kairos repair flow) ─────────────────
    let msg = exec(&conn, "DROP TABLE Memory");
    assert!(msg.to_lowercase().contains("dropped"), "drop failed: {msg}");
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, embedding FLOAT[], PRIMARY KEY (id))",
    );

    // ── IMPORT ───────────────────────────────────────────────
    let msg = exec(&conn, &format!(r#"IMPORT DATABASE "{p}""#));
    assert!(msg.contains("imported"), "import failed: {msg}");

    // ── Verify data persisted through the round-trip ─────────
    let res = query_values(&conn, "MATCH (m:Memory) RETURN count(m)");
    assert_eq!(res.trim(), "Int64(3)", "repair flow must preserve all 3 Memory rows");

    // Verify specific content survived.
    let res = query_values(&conn, "MATCH (m:Memory {id: 2}) RETURN m.content");
    assert_eq!(res.trim(), "String(\"hello 2\")", "content must survive round-trip");
}

#[test]
fn test_p5334_export_without_options_still_works() {
    let (dir, _db, conn) = setup_db_on_disk();
    exec(
        &conn,
        "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))",
    );
    exec(&conn, "CREATE (:Person {name: 'a', age: 1})");

    let export_dir = dir.path().join("plain");
    let p = export_dir.to_string_lossy().replace('\\', "/");
    let msg = exec(&conn, &format!(r#"EXPORT DATABASE "{p}""#));
    assert!(msg.contains("exported"), "export failed: {msg}");
    let copy = std::fs::read_to_string(export_dir.join("copy.cypher")).expect("copy.cypher written");
    assert!(copy.contains(".csv"), "default format is csv: {copy}");

    let msg = exec(&conn, &format!(r#"IMPORT DATABASE "{p}""#));
    assert!(msg.contains("imported"), "import failed: {msg}");

    // P53.34b: verify data survived the round-trip.
    let res = query_values(&conn, "MATCH (p:Person) RETURN count(p)");
    assert_eq!(res.trim(), "Int64(1)", "expected 1 Person after CSV import");
}

/// P53.37 — EXPORT DATABASE must tolerate rel tables with no property columns
/// (metadata relations like kairos HAS_TABLE/HAS_DEFINITION are created with
/// only FROM/TO endpoints). Previously the data-export query degraded to
/// `MATCH ()-[r:T]->() RETURN ` (empty column list) → parse error; the COPY
/// FROM line also referenced a data file that was never written.
#[test]
fn test_p5337_export_with_endpoint_only_rel_table() {
    let (dir, _db, conn) = setup_db_on_disk();
    exec(&conn, "CREATE NODE TABLE Database(name STRING, PRIMARY KEY (name))");
    exec(
        &conn,
        "CREATE NODE TABLE MetadataTable(name STRING, PRIMARY KEY (name))",
    );
    exec(&conn, "CREATE REL TABLE HAS_TABLE(FROM Database TO MetadataTable)");
    exec(&conn, "CREATE (:Database {name: 'db1'})");
    exec(&conn, "CREATE (:MetadataTable {name: 'm1'})");
    exec(
        &conn,
        "MATCH (d:Database {name: 'db1'}), (m:MetadataTable {name: 'm1'}) \
         CREATE (d)-[:HAS_TABLE]->(m)",
    );

    let export_dir = dir.path().join("backup");
    let p = export_dir.to_string_lossy().replace('\\', "/");
    let msg = exec(&conn, &format!(r#"EXPORT DATABASE "{p}""#));
    assert!(msg.contains("exported"), "export failed: {msg}");

    // The endpoint-only rel table must not appear in copy.cypher (no data file).
    let copy = std::fs::read_to_string(export_dir.join("copy.cypher")).expect("copy.cypher written");
    assert!(
        !copy.contains("HAS_TABLE"),
        "endpoint-only rel table must be skipped in copy.cypher: {copy}"
    );
    assert!(
        copy.contains("Database"),
        "node table export must still be listed: {copy}"
    );

    // IMPORT must still succeed (skipped table has nothing to load).
    let msg = exec(&conn, &format!(r#"IMPORT DATABASE "{p}""#));
    assert!(msg.contains("imported"), "import failed: {msg}");
    let res = query_values(&conn, "MATCH (d:Database) RETURN count(d)");
    assert_eq!(res.trim(), "Int64(1)", "Database rows must survive round-trip");
}

/// P53.37 — parquet EXPORT/IMPORT round-trip preserves data AND FLOAT[]
/// embedding lists (harness `repair_schema`: count == 3 after export →
/// drop → recreate → import). The parquet writer previously serialized
/// `Value::List` as Utf8/NULL and the reader rejected Utf8→List, so every
/// row was dropped on import; all-null columns (an untouched FLOAT[] column)
/// must keep their declared Arrow type instead of falling back to Utf8.
#[cfg(feature = "parquet-export")]
#[test]
fn test_p5337_parquet_roundtrip_data_persists() {
    let (dir, _db, conn) = setup_db_on_disk();
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, embedding FLOAT[], dae_embedding FLOAT[], PRIMARY KEY (id))",
    );
    for id in 1..=3 {
        exec(
            &conn,
            &format!("CREATE (:Memory {{id: {id}, content: 'hello {id}', embedding: [{id}.0, 0.0, 0.0]}})"),
        );
    }
    let export_dir = dir.path().join("backup");
    let p = export_dir.to_string_lossy().replace('\\', "/");
    let msg = exec(&conn, &format!(r#"EXPORT DATABASE "{p}" (format="parquet")"#));
    assert!(msg.contains("exported"), "export failed: {msg}");

    exec(&conn, "DROP TABLE Memory");
    exec(
        &conn,
        "CREATE NODE TABLE Memory(id INT64, content STRING, embedding FLOAT[], dae_embedding FLOAT[], PRIMARY KEY (id))",
    );
    let msg = exec(&conn, &format!(r#"IMPORT DATABASE "{p}" (format="parquet")"#));
    assert!(msg.contains("imported"), "import failed: {msg}");

    let res = query_values(&conn, "MATCH (m:Memory) RETURN count(m)");
    assert_eq!(res.trim(), "Int64(3)", "all 3 rows must survive the parquet round-trip");
    // The FLOAT[] embedding must come back as a list of doubles, not NULL/Utf8.
    let res = query_values(&conn, "MATCH (m:Memory {id: 2}) RETURN m.embedding");
    assert!(
        res.contains("List([Double(2.0), Double(0.0), Double(0.0)])"),
        "embedding must round-trip as a FLOAT[] list, got: {res}"
    );
    // The untouched (all-null) FLOAT[] column must import as NULL, not fail
    // the whole COPY with a Utf8→List type mismatch.
    let res = query_values(&conn, "MATCH (m:Memory {id: 2}) RETURN m.dae_embedding");
    assert!(
        res.contains("null") || res.contains("None"),
        "all-null FLOAT[] column must read back as NULL, got: {res}"
    );
}
