use kuzu_main::{Connection, Database, SystemConfig};
use tempfile::tempdir;
use std::sync::Arc;
use kuzu_common::types::Value;

#[test]
fn test_create_fts_index() -> Result<(), String> {
    let dir = tempdir().map_err(|e| e.to_string())?;
    let db = Arc::new(Database::new(dir.path().to_str().unwrap(), SystemConfig::default()).map_err(|e| e.to_string())?);
    let conn = Connection::new(&db);

    conn.query("CREATE NODE TABLE Document (id INT64, title STRING, content STRING, PRIMARY KEY(id))")?;
    
    conn.query("CREATE (d:Document {id: 1, title: 'Kuzu DB', content: 'A fast graph database'})")?;
    conn.query("CREATE (d:Document {id: 2, title: 'Rust Language', content: 'A fast systems programming language'})")?;
    
    let res = conn.query("CALL create_fts_index('Document', 'doc_idx', ['title', 'content'])")?;
    
    let chunk = res.chunks.first().unwrap();
    let val = match chunk.fields[0].get_value(0).unwrap() {
        Value::String(s) => s,
        _ => panic!("Expected string"),
    };
    assert_eq!(val, "Success");

    // The tables Document_doc_idx_docs and Document_doc_idx_appears_in should be created implicitly
    // We can query them to see if the index works
    let docs_res = conn.query("MATCH (d:Document_doc_idx_docs) RETURN d.term")?;
    assert!(docs_res.chunks.first().unwrap().size > 0, "FTS docs table should be populated");
    
    let appears_in_res = conn.query("MATCH ()-[r:Document_doc_idx_appears_in]->() RETURN r.count")?;
    assert!(appears_in_res.chunks.first().unwrap().size > 0, "FTS appears_in table should be populated");

    Ok(())
}
