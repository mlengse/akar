use akar_common::types::Value;
use akar_main::{Connection, Database, SystemConfig};
use std::sync::Arc;
use tempfile::tempdir;

#[test]
fn test_create_and_query_fts_index() -> Result<(), String> {
    let dir = tempdir().map_err(|e| e.to_string())?;
    let db = Arc::new(Database::new(dir.path().to_str().unwrap(), SystemConfig::default()).map_err(|e| e.to_string())?);
    let conn = Connection::new(&db);

    conn.query("CREATE NODE TABLE Document (id INT64, title STRING, content STRING, PRIMARY KEY(id))")?;

    conn.query("CREATE (d:Document {id: 1, title: 'Akar DB', content: 'A fast graph database in Rust'})")?;
    conn.query(
        "CREATE (d:Document {id: 2, title: 'Rust Language', content: 'A systems programming language using Rust'})",
    )?;
    conn.query("CREATE (d:Document {id: 3, title: 'Python Language', content: 'A slow scripting language'})")?;

    // Create native FTS index
    conn.query("CREATE FTS INDEX doc_idx ON (Document.content)")?;

    // The macro tables: fts_doc_idx_docs, fts_doc_idx_terms, fts_doc_idx_appears_in should be created implicitly
    let docs_res = conn.query("MATCH (d:fts_doc_idx_docs) RETURN d.text")?;
    assert!(
        docs_res.chunks.first().unwrap().size > 0,
        "FTS docs table should be populated"
    );

    let terms_res = conn.query("MATCH (t:fts_doc_idx_terms) RETURN t.term")?;
    assert!(
        terms_res.chunks.first().unwrap().size > 0,
        "FTS terms table should be populated"
    );

    let appears_in_res = conn.query("MATCH ()-[r:fts_doc_idx_appears_in]->() RETURN r.term_freq")?;
    assert!(
        appears_in_res.chunks.first().unwrap().size > 0,
        "FTS appears_in table should be populated"
    );

    // Query using native MATCH ... USING FTS INDEX
    let search_res = conn.query("MATCH (d:Document) USING FTS INDEX doc_idx('Rust') RETURN d.id, d.title")?;
    let chunk = search_res.chunks.first().unwrap();
    assert_eq!(chunk.size, 2, "Should return exactly 2 matches for 'Rust'");

    // Verify scores order: document 2 has "using Rust" and "Rust Language" in context (more matches or higher density/length factors)
    // Wait, let's verify that the values returned are correct.
    println!("Returned chunk fields: {:?}", chunk.fields);
    let title1 = match chunk.get_value(1, 0).unwrap() {
        Value::String(s) => s,
        _ => panic!("Expected string for title1, got {:?}", chunk.get_value(1, 0).unwrap()),
    };
    let title2 = match chunk.get_value(1, 1).unwrap() {
        Value::String(s) => s,
        _ => panic!("Expected string for title2, got {:?}", chunk.get_value(1, 1).unwrap()),
    };

    // Rust is in "Akar DB" and "Rust Language". Both should be returned.
    assert!(title1 == "Akar DB" || title1 == "Rust Language");
    assert!(title2 == "Akar DB" || title2 == "Rust Language");

    // Query for "slow"
    let search_res_slow = conn.query("MATCH (d:Document) USING FTS INDEX doc_idx('slow') RETURN d.id, d.title")?;
    let chunk_slow = search_res_slow.chunks.first().unwrap();
    assert_eq!(chunk_slow.size, 1, "Should return exactly 1 match for 'slow'");
    let title_slow = match chunk_slow.get_value(1, 0).unwrap() {
        Value::String(s) => s,
        _ => panic!("Expected string"),
    };
    assert_eq!(title_slow, "Python Language");

    Ok(())
}

#[test]
fn test_fts_with_where_predicate() -> Result<(), String> {
    let dir = tempdir().map_err(|e| e.to_string())?;
    let db = Arc::new(Database::new(dir.path().to_str().unwrap(), SystemConfig::default()).map_err(|e| e.to_string())?);
    let conn = Connection::new(&db);

    conn.query("CREATE NODE TABLE Document (id INT64, title STRING, content STRING, PRIMARY KEY(id))")?;
    conn.query("CREATE (d:Document {id: 1, title: 'Akar DB', content: 'A fast graph database in Rust'})")?;
    conn.query(
        "CREATE (d:Document {id: 2, title: 'Rust Language', content: 'A systems programming language using Rust'})",
    )?;
    conn.query("CREATE (d:Document {id: 3, title: 'Python Language', content: 'A slow scripting language'})")?;
    conn.query("CREATE FTS INDEX doc_idx ON (Document.content)")?;

    // FTS matches 'language' on docs 2 & 3 (rows 1 & 2). WHERE title = 'Python Language'
    // matches the row beyond the FTS-narrowed subset — previously panicked with index OOB
    // in the Arrow fast path (scan.rs), which indexed rows_to_emit[i] by mask position.
    // (Non-PK column so the planner uses scan+filter, not a PK point lookup.)
    let res = conn.query(
        "MATCH (d:Document) USING FTS INDEX doc_idx('language') WHERE d.title = 'Python Language' RETURN d.id, d.title",
    )?;
    let chunk = res.chunks.first().unwrap();
    assert_eq!(chunk.size, 1, "FTS + WHERE should return exactly 1 row");
    let id = match chunk.get_value(0, 0).unwrap() {
        Value::Int64(v) => v,
        _ => panic!("Expected Int64 id, got {:?}", chunk.get_value(0, 0).unwrap()),
    };
    assert_eq!(id, 3, "Only doc id 3 should match 'language' with that title");

    // Also verify the non-matching row is excluded, not just not panicking.
    let res2 = conn.query(
        "MATCH (d:Document) USING FTS INDEX doc_idx('language') WHERE d.title <> 'Python Language' RETURN d.id, d.title",
    )?;
    let chunk2 = res2.chunks.first().unwrap();
    assert_eq!(chunk2.size, 1, "FTS + WHERE (title <> 'Python Language') should return only doc 2");

    Ok(())
}
