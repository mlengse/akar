#![cfg(feature = "markdown-extension")]

use akar_common::types::Value;
use akar_main::{Connection, Database, SystemConfig};
use std::sync::Arc;

/// P122.2: `CALL read_markdown_wiki('<folder>')` reads a Markdown wiki from the
/// data lake and surfaces it as the two declared columns `node` and `rel`.
///
/// This also pins the connection-layer behaviour the `CALL` path needs: the
/// chunk's own column names survive, instead of being flattened to `col_0` /
/// `col_1` by the row-based table-function path.
#[test]
fn test_read_markdown_wiki_call() -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let wiki = dir.path().join("wiki");
    std::fs::create_dir_all(wiki.join("notes")).map_err(|e| e.to_string())?;
    std::fs::write(
        wiki.join("notes").join("a.md"),
        "---\ntitle: Alpha\ntags: [x, y]\n---\nsee [[b]] and [[c|See]]\n",
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(wiki.join("notes").join("b.md"), "no links here\n").map_err(|e| e.to_string())?;

    let db = Arc::new(Database::new(dir.path().to_str().unwrap(), SystemConfig::default()).map_err(|e| e.to_string())?);
    let conn = Connection::new(&db);

    // The path reaches the binder as a string literal; forward slashes keep
    // Windows backslashes from being read as escape sequences.
    let root = wiki.to_string_lossy().replace('\\', "/");
    let res = conn.query(&format!("CALL read_markdown_wiki('{root}')"))?;
    let chunk = res.chunks.first().ok_or("read_markdown_wiki returned no chunk")?;

    assert_eq!(
        chunk.field_names,
        vec!["node".to_string(), "rel".to_string()],
        "the declared column names must survive the CALL path"
    );
    assert_eq!(chunk.size, 4, "one note row plus one row per link, per note");

    // `get_value`/`is_null` take (field index, row index).
    assert!(chunk.is_null(1, 0), "the note's own row carries a NULL rel");
    assert_eq!(chunk.get_value(0, 1), Some(Value::String("notes/a".into())));
    assert_eq!(chunk.get_value(1, 1), Some(Value::String("b".into())));
    assert_eq!(chunk.get_value(1, 2), Some(Value::String("c".into())));
    assert!(chunk.is_null(1, 3), "the note's own row carries a NULL rel");

    Ok(())
}
