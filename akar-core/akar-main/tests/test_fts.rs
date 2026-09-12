use akar_common::types::Value;
use akar_main::{Connection, Database, QueryResult, SystemConfig};
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

    // P104.2 clean break: the macro tables (fts_doc_idx_docs, fts_doc_idx_terms,
    // fts_doc_idx_appears_in) no longer exist — SELECT FROM them must error.
    assert!(
        conn.query("MATCH (d:fts_doc_idx_docs) RETURN d.text").is_err(),
        "FTS docs macro table must not exist after clean break (P104.2)"
    );
    assert!(
        conn.query("MATCH (t:fts_doc_idx_terms) RETURN t.term").is_err(),
        "FTS terms macro table must not exist after clean break (P104.2)"
    );
    assert!(
        conn.query("MATCH ()-[r:fts_doc_idx_appears_in]->() RETURN r.term_freq")
            .is_err(),
        "FTS appears_in macro table must not exist after clean break (P104.2)"
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
    assert_eq!(
        chunk2.size, 1,
        "FTS + WHERE (title <> 'Python Language') should return only doc 2"
    );

    Ok(())
}

#[test]
fn test_fts_catches_up_rows_after_index() -> Result<(), String> {
    let dir = tempdir().map_err(|e| e.to_string())?;
    let db = Arc::new(Database::new(dir.path().to_str().unwrap(), SystemConfig::default()).map_err(|e| e.to_string())?);
    let conn = Connection::new(&db);

    conn.query("CREATE NODE TABLE Document (id INT64, title STRING, content STRING, PRIMARY KEY(id))")?;
    conn.query("CREATE (d:Document {id: 1, title: 'Akar DB', content: 'A fast graph database in Rust'})")?;
    conn.query("CREATE (d:Document {id: 2, title: 'Rust Language', content: 'A systems programming language'})")?;
    conn.query("CREATE (d:Document {id: 3, title: 'Python', content: 'A slow scripting language'})")?;
    conn.query("CREATE FTS INDEX doc_idx ON (Document.content)")?;

    // Baseline: the term is not present in any indexed row.
    let base = conn.query("MATCH (d:Document) USING FTS INDEX doc_idx('katana') RETURN d.id, d.title")?;
    assert_eq!(base.chunks.first().unwrap().size, 0, "no match before the row exists");

    // Insert a row AFTER the index was created.
    conn.query("CREATE (d:Document {id: 4, title: 'Katana', content: 'katana rust embedded vector database'})")?;

    // P52.39 catch-up: the newly inserted row must be searchable.
    let res = conn.query("MATCH (d:Document) USING FTS INDEX doc_idx('katana') RETURN d.id, d.title")?;
    let chunk = res.chunks.first().unwrap();
    assert_eq!(chunk.size, 1, "row inserted after CREATE FTS INDEX must be searchable");
    let id = match chunk.get_value(0, 0).unwrap() {
        Value::Int64(v) => v,
        _ => panic!("Expected Int64 id, got {:?}", chunk.get_value(0, 0).unwrap()),
    };
    assert_eq!(id, 4, "catch-up must return the post-index row");

    // P52.39 deleted-row filter: a deleted doc must no longer match.
    let del_res = conn.query("MATCH (d:Document) WHERE d.id = 4 DELETE d")?;
    assert_eq!(
        del_res.chunks.first().unwrap().get_i64(0, 0).unwrap(),
        1,
        "DELETE should remove exactly 1 row"
    );
    let after = conn.query("MATCH (d:Document) USING FTS INDEX doc_idx('katana') RETURN d.id, d.title")?;
    let chunk_after = after.chunks.first().unwrap();
    assert_eq!(chunk_after.size, 0, "deleted row must not remain searchable");

    Ok(())
}

/// P104.1: `CREATE FTS INDEX` must build a persistent Tantivy index on disk
/// under `<db_path>/fts/<index_name>` (not just the backward-compat macro
/// tables).
#[test]
fn test_create_fts_index_persists_tantivy_index() -> Result<(), String> {
    let dir = tempdir().map_err(|e| e.to_string())?;
    let db = Arc::new(Database::new(dir.path().to_str().unwrap(), SystemConfig::default()).map_err(|e| e.to_string())?);
    let conn = Connection::new(&db);

    conn.query("CREATE NODE TABLE Document (id INT64, title STRING, content STRING, PRIMARY KEY(id))")?;
    conn.query("CREATE (d:Document {id: 1, title: 'Akar DB', content: 'A fast graph database in Rust'})")?;
    conn.query("CREATE (d:Document {id: 2, title: 'Python', content: 'A slow scripting language'})")?;
    conn.query("CREATE FTS INDEX doc_idx ON (Document.content)")?;

    let index_dir = dir.path().join("fts").join("doc_idx");
    assert!(
        index_dir.join("meta.json").exists(),
        "Tantivy index metadata must be persisted at {}",
        index_dir.display()
    );

    Ok(())
}

/// Read the `id` column of a `MATCH ... RETURN d.id` result as a sorted
/// `Vec<i64>` (hits come back ranked by descending BM25 score, so compare as a
/// sorted set).
fn ids(res: &QueryResult) -> Vec<i64> {
    let chunk = res.chunks.first().unwrap();
    let mut v: Vec<i64> = (0..chunk.size)
        .map(|i| match chunk.get_value(0, i).unwrap() {
            Value::Int64(x) => x,
            other => panic!("expected Int64 id, got {other:?}"),
        })
        .collect();
    v.sort_unstable();
    v
}

/// P106.2 — the `USING FTS INDEX idx('<query>')` grammar exposes Tantivy's
/// advanced query types end-to-end. No SQL grammar change was needed: the query
/// string inside the literal is passed verbatim to Tantivy's `QueryParser`, so
/// the "grammar extension" is Tantivy's own query language. This test freezes
/// the exposed types with a corpus engineered so each type is provably
/// distinct (corpus tokenized with Tantivy's `en_stem`: learn/learning → learn,
/// language → languag, database → databas):
///
/// - **Phrase** `"machine learning"` matches doc 2 but NOT doc 5 (both terms
///   present, not adjacent) — while bare `machine learning` (implicit OR)
///   matches both.
/// - **Boolean** Tantivy's operator syntax: `+rust -python` (must + must-not)
///   matches docs 1 & 4; `+rust +python` only doc 2.
/// - **Regex / wildcard** `content:/rus.*/` matches the rust docs (1, 2, 4)
///   but not "dust" (6). Note: bare-term `*` and `~N` are NOT query operators in
///   Tantivy 0.26.2 (fuzzy exists only via `set_field_fuzzy`); the phrase-level
///   `*` and `~N` operators below are the real syntax gates.
/// - **Phrase-prefix** `"machine learn"*` matches doc 2 (adjacent machine +
///   learning), not doc 5 (terms separated by "vision").
/// - **Phrase-slop** `"machine learning rust"` matches nothing (three terms not
///   adjacent anywhere); `"machine learning rust"~1` matches doc 2 (one
///   interpolated position allowed between "learning" and "rust").
#[test]
fn test_fts_advanced_query_types() -> Result<(), String> {
    let dir = tempdir().map_err(|e| e.to_string())?;
    let db = Arc::new(Database::new(dir.path().to_str().unwrap(), SystemConfig::default()).map_err(|e| e.to_string())?);
    let conn = Connection::new(&db);

    conn.query("CREATE NODE TABLE Document (id INT64, title STRING, content STRING, PRIMARY KEY(id))")?;
    conn.query("CREATE FTS INDEX doc_idx ON (Document.content)")?;

    let insert = |id: i64, text: &str| {
        conn.query(&format!(
            "CREATE (d:Document {{id: {id}, title: 'doc {id}', content: '{text}'}})"
        ))
    };
    insert(1, "A fast graph database in Rust")?;
    insert(2, "machine learning with rust and python")?;
    insert(3, "python is a fun language")?;
    insert(4, "dust and rust on the shelf")?;
    insert(5, "machine vision and learning")?;
    insert(6, "the dust swept away")?;

    // Phrase: adjacent terms only → {2}; doc 5 has the terms separated by
    // "vision" and must NOT match. Bare terms (implicit OR) match both.
    let phrase = conn.query("MATCH (d:Document) USING FTS INDEX doc_idx('\"machine learning\"') RETURN d.id")?;
    let bare = conn.query("MATCH (d:Document) USING FTS INDEX doc_idx('machine learning') RETURN d.id")?;
    assert_eq!(
        ids(&phrase),
        vec![2],
        "phrase must match only the adjacent pair (doc 2)"
    );
    assert_eq!(ids(&bare), vec![2, 5], "bare terms (OR) match docs 2 and 5");

    // Boolean operators: Tantivy uses + (Must) / - (MustNot), not SQL-style
    // AND NOT. These are the exposed query grammar operators.
    //
    // +rust -python → docs containing "rust" AND excluding "python" → {1, 4}.
    let must_not = conn.query("MATCH (d:Document) USING FTS INDEX doc_idx('+rust -python') RETURN d.id")?;
    assert_eq!(
        ids(&must_not),
        vec![1, 4],
        "+rust -python must exclude doc 2 (has python) and doc 3 (no rust)"
    );
    // +rust +python → both must be present → {2}.
    let must_both = conn.query("MATCH (d:Document) USING FTS INDEX doc_idx('+rust +python') RETURN d.id")?;
    assert_eq!(ids(&must_both), vec![2], "+rust +python must match only doc 2");

    // Regex / wildcard: `/rus.*/` matches the "rust" docs but NOT "dust".
    // Tantivy regex queries require an explicit field prefix (`field:/.../`).
    let re = conn.query("MATCH (d:Document) USING FTS INDEX doc_idx('content:/rus.*/') RETURN d.id")?;
    let none = conn.query("MATCH (d:Document) USING FTS INDEX doc_idx('rus') RETURN d.id")?;
    assert_eq!(
        ids(&re),
        vec![1, 2, 4],
        "regex content:/rus.*/ matches the rust docs only"
    );
    assert!(ids(&none).is_empty(), "exact term 'rus' must not match");

    // Phrase-prefix: `"machine learn"*` → doc 2 (machine+learning adjacent);
    // doc 5 has them separated by "vision", so the phrase prefix excludes it.
    let ph_prefix = conn.query("MATCH (d:Document) USING FTS INDEX doc_idx('\"machine learn\"*') RETURN d.id")?;
    assert_eq!(
        ids(&ph_prefix),
        vec![2],
        "phrase-prefix matches the adjacent pair (doc 2)"
    );

    // Phrase-slop: the three terms are never adjacent, so the plain phrase is
    // empty; `~1` allows exactly one interpolated position (doc 2 "with").
    let no_slop = conn.query("MATCH (d:Document) USING FTS INDEX doc_idx('\"machine learning rust\"') RETURN d.id")?;
    let slop = conn.query("MATCH (d:Document) USING FTS INDEX doc_idx('\"machine learning rust\"~1') RETURN d.id")?;
    assert!(ids(&no_slop).is_empty(), "three terms are not adjacent anywhere");
    assert_eq!(
        ids(&slop),
        vec![2],
        "slop ~1 allows the one interpolated position (doc 2)"
    );

    Ok(())
}
