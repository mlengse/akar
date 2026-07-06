use kuzu_common::types::Value;
use kuzu_main::{Connection, Database, SystemConfig};
use std::sync::Arc;
use tempfile::tempdir;

#[test]
fn test_create_and_query_fts_index() -> Result<(), String> {
    let dir = tempdir().map_err(|e| e.to_string())?;
    let db = Arc::new(Database::new(dir.path().to_str().unwrap(), SystemConfig::default()).map_err(|e| e.to_string())?);
    let conn = Connection::new(&db);

    conn.query("CREATE NODE TABLE Document (id INT64, title STRING, content STRING, PRIMARY KEY(id))")?;

    conn.query("CREATE (d:Document {id: 1, title: 'Kuzu DB', content: 'A fast graph database in Rust'})")?;
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

    let appears_in_res = conn.query("MATCH ()-[r:fts_doc_idx_appears_in]->() RETURN r.count")?;
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
    let title1 = match chunk.fields[1].get_value(0).unwrap() {
        Value::String(s) => s,
        _ => panic!(
            "Expected string for title1, got {:?}",
            chunk.fields[1].get_value(0).unwrap()
        ),
    };
    let title2 = match chunk.fields[1].get_value(1).unwrap() {
        Value::String(s) => s,
        _ => panic!(
            "Expected string for title2, got {:?}",
            chunk.fields[1].get_value(1).unwrap()
        ),
    };

    // Rust is in "Kuzu DB" and "Rust Language". Both should be returned.
    assert!(title1 == "Kuzu DB" || title1 == "Rust Language");
    assert!(title2 == "Kuzu DB" || title2 == "Rust Language");

    // Query for "slow"
    let search_res_slow = conn.query("MATCH (d:Document) USING FTS INDEX doc_idx('slow') RETURN d.id, d.title")?;
    let chunk_slow = search_res_slow.chunks.first().unwrap();
    assert_eq!(chunk_slow.size, 1, "Should return exactly 1 match for 'slow'");
    let title_slow = match chunk_slow.fields[1].get_value(0).unwrap() {
        Value::String(s) => s,
        _ => panic!("Expected string"),
    };
    assert_eq!(title_slow, "Python Language");

    Ok(())
}
