//! Tantivy-backed FTS index construction (P104.1, P104.2 clean break).
//!
//! `build_index` is the single entry point used by the
//! `PhysicalCreateFtsIndex` operator. It builds (or rebuilds) the Tantivy
//! index over a source column and persists it under `<db_path>/fts/<idx>` for
//! disk-backed catalogs.
//!
//! The full and incremental writes go through the same writer path, so the
//! Tantivy directory — not the legacy `fts_{idx}_docs` / `fts_{idx}_terms` /
//! `fts_{idx}_appears_in` macro tables — is the *only* FTS representation
//! (P104.2/P104.4 decision). [`apply_doc_writes`] is the incremental path used
//! by the commit-time propagation hook (P107.1): the writer deletes the
//! previous document for a `doc_id` and re-adds it when the source value
//! changed, keeping the index in sync with committed DML. The macro tables are
//! gone.

use std::path::Path;

use akar_storage::table::ColumnDefinition;
use tantivy::TantivyDocument;
use tantivy::schema::Term;

use crate::index::TantivyIndex;
use crate::schema::{DOC_ID_FIELD, build_index_schema};

/// Tantivy requires at least 15 MB of heap per indexing thread.
const INDEXER_MEMORY_BUDGET: usize = 15_000_000;

/// Build (or rebuild) the Tantivy FTS index over `text_column` for `rows`.
///
/// The index is persisted under `index_dir` when `Some`, otherwise kept in
/// memory only. All rows are added in a single commit.
///
/// # Errors
/// Returns a message when `text_column` is not indexable, the index directory
/// cannot be created, or the Tantivy writer/commit fails.
pub fn build_index(
    columns: &[ColumnDefinition],
    text_column: &str,
    index_dir: Option<&Path>,
    rows: &[(i64, String)],
) -> Result<(), String> {
    let schema = build_index_schema(columns);
    let index = match index_dir {
        Some(dir) => {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("FTS: cannot create index dir '{}': {e}", dir.display()))?;
            TantivyIndex::create_on_disk(dir, schema).map_err(|e| format!("FTS: {e}"))?
        }
        None => TantivyIndex::create_in_memory(schema),
    };

    append_docs(&index, columns, text_column, rows)
}

/// Append `rows` to an already-created Tantivy index over `text_column` and
/// commit once (P105.3 incremental catch-up — no macro tables to rebuild).
///
/// The schema is derived from `columns`; it must match the schema the index was
/// created with (fields are matched by name).
pub fn append_docs(
    index: &TantivyIndex,
    columns: &[ColumnDefinition],
    text_column: &str,
    rows: &[(i64, String)],
) -> Result<(), String> {
    let schema = build_index_schema(columns);
    let text_field = schema
        .get_field(text_column)
        .map_err(|_| format!("FTS: column '{text_column}' is not indexable"))?;
    let doc_id_field = schema
        .get_field(DOC_ID_FIELD)
        .map_err(|_| "FTS: internal doc_id field missing".to_string())?;

    {
        let mut writer = index
            .writer(1, INDEXER_MEMORY_BUDGET)
            .map_err(|e| format!("FTS: writer: {e}"))?;
        for (doc_id, text) in rows {
            let mut doc = TantivyDocument::default();
            doc.add_i64(doc_id_field, *doc_id);
            doc.add_text(text_field, text);
            writer
                .add_document(doc)
                .map_err(|e| format!("FTS: add_document: {e}"))?;
        }
        writer.commit().map_err(|e| format!("FTS: commit: {e}"))?;
    }

    Ok(())
}

/// Apply committed row writes to an already-created Tantivy FTS index and
/// commit once (P107.1 commit-time propagation).
///
/// For each `(doc_id, text)` entry the previous document for `doc_id` is first
/// removed via `delete_term` on the indexed [`DOC_ID_FIELD`], then the current
/// text is re-added when it is `Some`. `None` (a soft-deleted source row, which
/// surfaces as a NULL column, or a row whose text column was never set) only
/// deletes. This makes updates and deletes idempotent for `doc_id`s that may or
/// may not already be present.
///
/// The schema is read from the opened index itself, so callers pass only the
/// source text column name (which must match the field the index was built on).
///
/// # Errors
/// Returns a message when `text_column` is not indexable, the internal
/// `doc_id` field is missing, or the Tantivy writer/commit fails.
pub fn apply_doc_writes(
    index: &TantivyIndex,
    text_column: &str,
    writes: &[(i64, Option<String>)],
) -> Result<(), String> {
    if writes.is_empty() {
        return Ok(());
    }

    let schema = index.inner().schema();
    let text_field = schema
        .get_field(text_column)
        .map_err(|_| format!("FTS: column '{text_column}' is not indexable"))?;
    let doc_id_field = schema
        .get_field(DOC_ID_FIELD)
        .map_err(|_| "FTS: internal doc_id field missing".to_string())?;

    {
        let mut writer = index
            .writer(1, INDEXER_MEMORY_BUDGET)
            .map_err(|e| format!("FTS: writer: {e}"))?;
        for (doc_id, text) in writes {
            // `delete_term` is infallible (returns an Opstamp; an invalid term
            // for the index — e.g. a `doc_id` field that is not indexed — is a
            // silent no-op, never an error).
            writer.delete_term(Term::from_field_i64(doc_id_field, *doc_id));
            if let Some(text) = text {
                let mut doc = TantivyDocument::default();
                doc.add_i64(doc_id_field, *doc_id);
                doc.add_text(text_field, text);
                writer
                    .add_document(doc)
                    .map_err(|e| format!("FTS: add_document: {e}"))?;
            }
        }
        writer.commit().map_err(|e| format!("FTS: commit: {e}"))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::build_index_schema;

    fn text_col() -> ColumnDefinition {
        ColumnDefinition {
            name: "content".to_string(),
            logical_type: akar_common::types::LogicalTypeID::String,
            is_primary_key: false,
            compression: akar_common::enums::CompressionType::Uncompressed,
        }
    }

    #[test]
    fn test_build_index_in_memory_searchable() {
        let rows = vec![
            (0, "A fast graph database in Rust".to_string()),
            (1, "A systems programming language using Rust".to_string()),
            (2, "A slow scripting language".to_string()),
        ];
        let schema = build_index_schema(&[text_col()]);
        let idx = TantivyIndex::create_in_memory(schema);
        append_docs(&idx, &[text_col()], "content", &rows).unwrap();

        let reader = idx.reader().unwrap();
        reader.reload().unwrap();
        let searcher = reader.searcher();
        let searcher_schema = searcher.schema();
        let content = searcher_schema.get_field("content").unwrap();
        let doc_id = searcher_schema.get_field(DOC_ID_FIELD).unwrap();

        // "rust" (en_stem keeps it) matches docs 0 and 1.
        let hits = TantivyIndex::search_doc_ids(&reader, "rust", vec![content], doc_id, 10).unwrap();
        assert_eq!(hits.len(), 2, "expected 2 hits for 'rust'");
        // "language" matches docs 1 and 2.
        let hits = TantivyIndex::search_doc_ids(&reader, "language", vec![content], doc_id, 10).unwrap();
        assert_eq!(hits.len(), 2, "expected 2 hits for 'language'");
    }

    #[test]
    fn test_build_index_on_disk_creates_directory() {
        let dir = tempfile::tempdir().unwrap();
        let index_dir = dir.path().join("fts").join("doc_idx");
        let rows = vec![(0, "hello tantivy".to_string())];
        build_index(&[text_col()], "content", Some(&index_dir), &rows).unwrap();
        assert!(index_dir.join("meta.json").exists(), "Tantivy index must persist");
    }

    /// P107.1 — `apply_doc_writes` is idempotent: after an initial build a
    /// (doc_id, Some(text)) write replaces the row's text entirely (the old
    /// term stops matching, the delete-of-old-term happens first), and a
    /// (doc_id, None) write removes the document entirely. The schema is
    /// re-read from the opened index, so no caller-side schema is needed.
    #[test]
    fn test_apply_doc_writes_updates_and_deletes() {
        let rows = vec![
            (0, "A fast graph database in Rust".to_string()),
            (1, "A systems programming language using Rust".to_string()),
        ];
        let schema = build_index_schema(&[text_col()]);
        let idx = TantivyIndex::create_in_memory(schema);
        append_docs(&idx, &[text_col()], "content", &rows).unwrap();

        // Update doc 0 (drop "rust", add "katana"); delete doc 1.
        apply_doc_writes(
            &idx,
            "content",
            &[(0, Some("a katana embedded database".to_string())), (1, None)],
        )
        .unwrap();

        let reader = idx.reader().unwrap();
        reader.reload().unwrap();
        let searcher = reader.searcher();
        let search_schema = searcher.schema();
        let content = search_schema.get_field("content").unwrap();
        let doc_id = search_schema.get_field(DOC_ID_FIELD).unwrap();
        let search = |q: &str| TantivyIndex::search_doc_ids(&reader, q, vec![content], doc_id, 10).unwrap();

        // Deleted doc 1 is gone; updated doc 0 no longer matches the old term.
        assert!(search("rust").is_empty(), "deleted/updated docs must not match 'rust'");
        // The updated doc 0 matches its new term.
        let hits = search("katana");
        assert_eq!(hits.len(), 1, "updated doc 0 must match 'katana'");
        assert_eq!(hits[0].0, 0);
    }
}
