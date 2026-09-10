//! Tantivy-backed FTS index construction (P104.1).
//!
//! [`build_index`] is the single entry point used by the
//! `PhysicalCreateFtsIndex` operator. It builds (or rebuilds) a Tantivy index
//! over a source column and returns the payload needed to populate the legacy
//! `fts_{idx}_docs` / `fts_{idx}_terms` / `fts_{idx}_appears_in` macro tables
//! for backward compatibility.

use std::collections::HashMap;
use std::path::Path;

use akar_storage::table::ColumnDefinition;
use tantivy::TantivyDocument;

use crate::index::TantivyIndex;
use crate::schema::{DOC_ID_FIELD, build_index_schema};

/// Tantivy requires at least 15 MB of heap per indexing thread.
const INDEXER_MEMORY_BUDGET: usize = 15_000_000;

/// Legacy macro-table payload derived while building the Tantivy index.
///
/// - `docs`: `(doc_id, text)` for `fts_{idx}_docs`
/// - `terms`: `(term_id, term, doc_freq)` for `fts_{idx}_terms`
/// - `postings`: `(term_id, doc_id, term_freq)` for `fts_{idx}_appears_in`
#[derive(Debug, Default)]
pub struct FtsIndexData {
    pub docs: Vec<(i64, String)>,
    pub terms: Vec<(i64, String, i64)>,
    pub postings: Vec<(i64, i64, i64)>,
}

/// Build (or rebuild) the Tantivy FTS index over `text_column` for `rows`.
///
/// The index is persisted under `index_dir` when `Some`, otherwise kept in
/// memory only. All rows are added in a single commit.
///
/// Returns the legacy macro-table payload. Terms and postings are derived with
/// the same `en_stem` pipeline the Tantivy index uses, so the macro tables stay
/// semantically identical to the previous from-scratch implementation.
///
/// # Errors
/// Returns a message when `text_column` is not indexable, the index directory
/// cannot be created, or the Tantivy writer/commit fails.
pub fn build_index(
    columns: &[ColumnDefinition],
    text_column: &str,
    index_dir: Option<&Path>,
    rows: &[(i64, String)],
) -> Result<FtsIndexData, String> {
    let schema = build_index_schema(columns);
    let text_field = schema
        .get_field(text_column)
        .map_err(|_| format!("FTS: column '{text_column}' is not indexable"))?;
    let doc_id_field = schema
        .get_field(DOC_ID_FIELD)
        .map_err(|_| "FTS: internal doc_id field missing".to_string())?;

    let index = match index_dir {
        Some(dir) => {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("FTS: cannot create index dir '{}': {e}", dir.display()))?;
            TantivyIndex::create_on_disk(dir, schema).map_err(|e| format!("FTS: {e}"))?
        }
        None => TantivyIndex::create_in_memory(schema),
    };

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

    Ok(derive_macro_tables(rows))
}

/// Tokenize `rows` with the shared `en_stem` pipeline to produce the legacy
/// macro-table payload (P104.1 backward compatibility).
fn derive_macro_tables(rows: &[(i64, String)]) -> FtsIndexData {
    let mut term_map: HashMap<String, (i64, i64)> = HashMap::new();
    let mut docs = Vec::with_capacity(rows.len());
    let mut postings = Vec::new();

    for (doc_id, text) in rows {
        docs.push((*doc_id, text.clone()));
        let mut freq: HashMap<String, i64> = HashMap::new();
        for token in crate::tokenize(text) {
            let stemmed = crate::stem_word(&token);
            if !crate::STOP_WORDS.contains(&stemmed.as_str()) {
                *freq.entry(stemmed).or_insert(0) += 1;
            }
        }
        for (term, count) in freq {
            let next_id = term_map.len() as i64;
            let (term_id, doc_freq) = term_map.entry(term).or_insert((next_id, 0));
            *doc_freq += 1;
            postings.push((*term_id, *doc_id, count));
        }
    }

    let mut terms: Vec<(i64, String, i64)> = term_map.into_iter().map(|(term, (id, df))| (id, term, df)).collect();
    terms.sort_by_key(|(id, _, _)| *id);
    FtsIndexData { docs, terms, postings }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_col() -> ColumnDefinition {
        ColumnDefinition {
            name: "content".to_string(),
            logical_type: akar_common::types::LogicalTypeID::String,
            is_primary_key: false,
            compression: akar_common::enums::CompressionType::Uncompressed,
        }
    }

    #[test]
    fn test_build_index_in_memory_derives_macro_tables() {
        let rows = vec![
            (0, "A fast graph database in Rust".to_string()),
            (1, "A systems programming language using Rust".to_string()),
            (2, "A slow scripting language".to_string()),
        ];
        let data = build_index(&[text_col()], "content", None, &rows).unwrap();
        assert_eq!(data.docs.len(), 3);
        assert!(!data.terms.is_empty(), "terms must be derived");
        assert!(!data.postings.is_empty(), "postings must be derived");
        // "rust" stems to "rust" and appears in docs 0 and 1.
        let rust = data.terms.iter().find(|(_, t, _)| t == "rust").expect("term 'rust'");
        assert_eq!(rust.2, 2, "doc_freq for 'rust'");
        let rust_postings: Vec<_> = data.postings.iter().filter(|(tid, _, _)| *tid == rust.0).collect();
        assert_eq!(rust_postings.len(), 2);
    }

    #[test]
    fn test_build_index_on_disk_creates_directory() {
        let dir = tempfile::tempdir().unwrap();
        let index_dir = dir.path().join("fts").join("doc_idx");
        let rows = vec![(0, "hello tantivy".to_string())];
        let data = build_index(&[text_col()], "content", Some(&index_dir), &rows).unwrap();
        assert!(index_dir.join("meta.json").exists(), "Tantivy index must persist");
        assert_eq!(data.docs.len(), 1);
    }
}
