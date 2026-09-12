//! Tantivy index lifecycle: create, write, commit, read.
//!
//! Wraps `tantivy::Index` with Akar-specific convenience:
//! - [`TantivyIndex::create_on_disk`] — persistent index at a directory path
//! - [`TantivyIndex::open_on_disk`] — reopen an existing index (no schema needed)
//! - [`TantivyIndex::create_in_memory`] — ephemeral index for tests
//! - [`TantivyIndex::writer`] — `IndexWriter` with configurable threads / memory
//! - [`TantivyIndex::reader`] — `IndexReader` with [`ReloadPolicy::Manual`]
//! - [`TantivyIndex::search`] — convenience search returning `(Score, DocAddress)`
//!
//! Every index registers Akar's `en_stem` tokenizer (via
//! [`crate::tokenizer::manager`]) so `TEXT` fields built by
//! [`crate::schema::build_tantivy_schema`] resolve the same pipeline at index
//! and query time.

use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::Schema;
use tantivy::{DocAddress, Index, IndexWriter, ReloadPolicy, Score, TantivyDocument};

/// Re-exported so consumers of [`TantivyIndex::reader`] can name the reader
/// type without depending on `tantivy` directly (e.g. the physical FTS scan in
/// `akar-processor`).
pub use tantivy::IndexReader;

/// Minimum memory budget per thread accepted by Tantivy (15 MB).
#[cfg(test)]
const MIN_MEMORY_PER_THREAD: usize = 15_000_000;

/// Wrapper around a Tantivy [`Index`] with Akar-convenience constructors and
/// search helpers.
pub struct TantivyIndex {
    index: Index,
}

impl TantivyIndex {
    /// Open or create a persistent on-disk index at `index_dir`.
    ///
    /// If the directory does not exist it is created; if it already contains a
    /// valid Tantivy index it is opened.
    pub fn create_on_disk(index_dir: impl AsRef<Path>, schema: Schema) -> tantivy::Result<Self> {
        let mmap_dir = tantivy::directory::MmapDirectory::open(index_dir.as_ref())?;
        let mut index = Index::open_or_create(mmap_dir, schema)?;
        index.set_tokenizers(crate::tokenizer::manager());
        Ok(Self { index })
    }

    /// Create a fully in-memory index (no disk I/O).  Ideal for unit tests.
    pub fn create_in_memory(schema: Schema) -> Self {
        let mut index = Index::create_in_ram(schema);
        index.set_tokenizers(crate::tokenizer::manager());
        Self { index }
    }

    /// Open an existing on-disk index at `index_dir` without providing the
    /// schema (the schema is read back from the index's `meta.json`).
    ///
    /// Fails if the directory does not contain a Tantivy index. The caller is
    /// responsible for the index having been created first (e.g. via
    /// [`TantivyIndex::create_on_disk`]).
    pub fn open_on_disk(index_dir: impl AsRef<Path>) -> tantivy::Result<Self> {
        let mmap_dir = tantivy::directory::MmapDirectory::open(index_dir.as_ref())?;
        let mut index = Index::open(mmap_dir)?;
        index.set_tokenizers(crate::tokenizer::manager());
        Ok(Self { index })
    }

    /// Return the underlying [`Index`] for advanced use-cases.
    pub fn inner(&self) -> &Index {
        &self.index
    }

    /// Build an [`IndexWriter`] with `num_threads` worker threads and a shared
    /// memory budget of `overall_mem_budget` bytes (split evenly across threads).
    ///
    /// The minimum per-thread budget is 15 MB ([`MIN_MEMORY_PER_THREAD`]).
    pub fn writer(
        &self,
        num_threads: usize,
        overall_mem_budget: usize,
    ) -> tantivy::Result<IndexWriter<TantivyDocument>> {
        self.index.writer_with_num_threads(num_threads, overall_mem_budget)
    }

    /// Build an [`IndexReader`] with [`ReloadPolicy::Manual`].
    ///
    /// The caller **must** call [`IndexReader::reload()`] explicitly after each
    /// [`IndexWriter::commit()`] to observe the newly committed data.
    pub fn reader(&self) -> tantivy::Result<IndexReader> {
        self.index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
    }

    /// Convenience search: parse `query_str` against `search_fields`, collect
    /// the top `limit` results, and return `(DocAddress, Score)` pairs sorted
    /// by descending relevance.
    pub fn search(
        reader: &IndexReader,
        query_str: &str,
        search_fields: Vec<tantivy::schema::Field>,
        limit: usize,
    ) -> tantivy::Result<Vec<(Score, DocAddress)>> {
        let searcher = reader.searcher();
        let query_parser = QueryParser::for_index(searcher.index(), search_fields);
        let query = query_parser.parse_query(query_str)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(limit).order_by_score())?;
        Ok(top_docs)
    }

    /// Search `query_str` and resolve each hit to its stored `doc_id` (the
    /// `doc_id_field` stored on every document at index time), returning
    /// `(doc_id, score)` pairs sorted by descending relevance.
    ///
    /// This is what the physical FTS scan consumes (P105.2): hits come back as
    /// the source-table row index plus the Tantivy BM25 score.
    pub fn search_doc_ids(
        reader: &IndexReader,
        query_str: &str,
        search_fields: Vec<tantivy::schema::Field>,
        doc_id_field: tantivy::schema::Field,
        limit: usize,
    ) -> tantivy::Result<Vec<(i64, Score)>> {
        let searcher = reader.searcher();
        let query_parser = QueryParser::for_index(searcher.index(), search_fields);
        let query = query_parser.parse_query(query_str)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(limit).order_by_score())?;
        let mut results = Vec::with_capacity(top_docs.len());
        for (score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc(doc_address)?;
            if let Some(value) = doc.get_first(doc_id_field) {
                if let tantivy::schema::OwnedValue::I64(doc_id) = tantivy::schema::OwnedValue::from(value) {
                    results.push((doc_id, score));
                }
            }
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantivy::schema::{STORED, TEXT};

    fn test_schema() -> Schema {
        let mut builder = Schema::builder();
        let _ = builder.add_text_field("title", TEXT | STORED);
        let _ = builder.add_text_field("body", TEXT | STORED);
        builder.build()
    }

    #[test]
    fn test_create_in_memory_commit_search() {
        let schema = test_schema();
        let idx = TantivyIndex::create_in_memory(schema.clone());
        let title = schema.get_field("title").unwrap();
        let body = schema.get_field("body").unwrap();

        // Write 3 documents
        {
            let mut writer = idx.writer(1, MIN_MEMORY_PER_THREAD).unwrap();
            writer
                .add_document(tantivy::doc!(
                    title => "The Rust Programming Language",
                    body => "Rust is a systems language focused on safety and performance."
                ))
                .unwrap();
            writer
                .add_document(tantivy::doc!(
                    title => "Learning Python",
                    body => "Python is great for data science and scripting."
                ))
                .unwrap();
            writer
                .add_document(tantivy::doc!(
                    title => "Advanced Rust Patterns",
                    body => "Ownership, borrowing, and lifetimes in Rust."
                ))
                .unwrap();
            writer.commit().unwrap();
        }

        // Read
        let reader = idx.reader().unwrap();
        reader.reload().unwrap();
        assert_eq!(reader.searcher().num_docs(), 3);

        // Search for "rust" — should match documents 1 and 3
        let results = TantivyIndex::search(&reader, "rust", vec![title, body], 10).unwrap();
        assert_eq!(results.len(), 2, "expected 2 hits for 'rust'");

        // Search for "python" — should match document 2 only
        let results = TantivyIndex::search(&reader, "python", vec![title, body], 10).unwrap();
        assert_eq!(results.len(), 1, "expected 1 hit for 'python'");
    }

    #[test]
    fn test_create_on_disk_and_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let schema = test_schema();
        let title = schema.get_field("title").unwrap();

        // Create, write, commit
        {
            let idx = TantivyIndex::create_on_disk(dir.path(), schema.clone()).unwrap();
            let mut writer = idx.writer(1, MIN_MEMORY_PER_THREAD).unwrap();
            writer.add_document(tantivy::doc!(title => "Hello Tantivy")).unwrap();
            writer.commit().unwrap();
        }

        // Reopen from the same directory
        let idx = TantivyIndex::create_on_disk(dir.path(), schema).unwrap();
        let reader = idx.reader().unwrap();
        reader.reload().unwrap();
        assert_eq!(reader.searcher().num_docs(), 1);

        let results = TantivyIndex::search(&reader, "tantivy", vec![title], 10).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_empty_search_returns_nothing() {
        let schema = test_schema();
        let idx = TantivyIndex::create_in_memory(schema.clone());
        let title = schema.get_field("title").unwrap();

        let reader = idx.reader().unwrap();
        reader.reload().unwrap();

        let results = TantivyIndex::search(&reader, "anything", vec![title], 10).unwrap();
        assert!(results.is_empty());
    }

    fn bm25_parity_schema() -> Schema {
        let mut builder = Schema::builder();
        let _ = builder.add_text_field("title", TEXT | STORED);
        let _ = builder.add_i64_field("doc_id", STORED);
        builder.build()
    }

    /// P106.1 — BM25 scoring parity, plan test case.
    ///
    /// Verifies the score surfaced through [`TantivyIndex::search_doc_ids`] is
    /// Tantivy's own BM25 (k1=1.2, b=0.75) against the plan's closed-form case:
    /// `bm25(2.0, 100.0, 80.0, 5.0, 1000.0, 1.2, 0.75)` (tf, dl, avgdl, df, N,
    /// k1, b). The corpus is engineered to those statistics: 1,000 documents,
    /// "rust" in exactly 5 (df=5), the target document carrying it twice (tf=2)
    /// at length 100, every other document length 80.
    ///
    /// Two effects make the naive literal imprecise — documented here because
    /// plan P106.1 asks for the divergence to be recorded:
    ///
    /// 1. **dl is a quantized fieldnorm.** Tantivy stores doc length as a byte
    ///    indexing `FIELD_NORMS_TABLE`; a length of 100 rounds *down* to 96.
    /// 2. **avgdl is the raw mean of token counts** — (100 + 999·80) / 1000 =
    ///    80.02 here — not a mean over quantized lengths.
    ///
    /// The engine-consistent closed form therefore uses `dl=96, avgdl=80.02` and
    /// equals the real score exactly. akar's legacy scorer (`crate::bm25`)
    /// normalised with caller-supplied raw lengths and could not reproduce real
    /// engine numbers for this reason; the new pure-Tantivy scan can.
    #[test]
    fn test_bm25_scoring_parity_plan_case() {
        let schema = bm25_parity_schema();
        let idx = TantivyIndex::create_in_memory(schema.clone());
        let title = schema.get_field("title").unwrap();
        let doc_id = schema.get_field("doc_id").unwrap();

        let mut writer = idx.writer(1, MIN_MEMORY_PER_THREAD).unwrap();
        for d in 0..1000 {
            let text = if d == 0 {
                format!("rust rust {}", "token ".repeat(98))
            } else if d < 5 {
                format!("rust {}", "token ".repeat(79))
            } else {
                "token ".repeat(80)
            };
            writer
                .add_document(tantivy::doc!(title => text.as_str(), doc_id => d as i64))
                .unwrap();
        }
        writer.commit().unwrap();

        let reader = idx.reader().unwrap();
        reader.reload().unwrap();
        let hits = TantivyIndex::search_doc_ids(&reader, "rust", vec![title], doc_id, 10).unwrap();
        assert_eq!(hits.len(), 5, "expected exactly 5 hits for 'rust'");

        let (_, actual) = *hits
            .iter()
            .find(|(id, _)| *id == 0)
            .expect("target document 0 must be among the hits");

        let naive = crate::bm25(2.0, 100.0, 80.0, 5.0, 1000.0, 1.2, 0.75);
        let engine_consistent = crate::bm25(2.0, 96.0, 80.02, 5.0, 1000.0, 1.2, 0.75);

        assert!(
            (actual as f64 - engine_consistent).abs() < 1e-3,
            "engine score {actual} != engine-consistent closed form {engine_consistent}"
        );
        assert!(
            ((actual as f64 - naive).abs() / naive) < 0.02,
            "engine score {actual} drifted >2% from the naive literal {naive}"
        );
    }

    /// P106.1 — BM25 length normalization.
    ///
    /// For equal term frequency, the shorter document must outscore the longer
    /// one (b=0.75 penalises length beyond avgdl). Field lengths 64 and 128 are
    /// both lossless in `FIELD_NORMS_TABLE`, so dl/avgdl is exact here and the
    /// closed form reproduces the engine scores precisely.
    #[test]
    fn test_bm25_length_normalization() {
        let schema = bm25_parity_schema();
        let idx = TantivyIndex::create_in_memory(schema.clone());
        let title = schema.get_field("title").unwrap();
        let doc_id = schema.get_field("doc_id").unwrap();

        let mut writer = idx.writer(1, MIN_MEMORY_PER_THREAD).unwrap();
        let texts = [
            format!("rust rust {}", "token ".repeat(62)),
            format!("rust rust {}", "token ".repeat(126)),
            format!("rust rust {}", "token ".repeat(126)),
        ];
        for (d, text) in texts.iter().enumerate() {
            writer
                .add_document(tantivy::doc!(title => text.as_str(), doc_id => d as i64))
                .unwrap();
        }
        writer.commit().unwrap();

        let reader = idx.reader().unwrap();
        reader.reload().unwrap();
        let hits = TantivyIndex::search_doc_ids(&reader, "rust", vec![title], doc_id, 10).unwrap();
        assert_eq!(hits.len(), 3);
        let score_of = |id: i64| hits.iter().find(|(i, _)| *i == id).map(|(_, s)| *s).unwrap();

        let avgdl = (64.0 + 128.0 + 128.0) / 3.0;
        let expected_short = crate::bm25(2.0, 64.0, avgdl, 3.0, 3.0, 1.2, 0.75);
        let expected_long = crate::bm25(2.0, 128.0, avgdl, 3.0, 3.0, 1.2, 0.75);

        assert!(
            (score_of(0) as f64 - expected_short).abs() < 1e-3,
            "short doc score != closed form ({} != {expected_short})",
            score_of(0)
        );
        assert!(
            (score_of(1) as f64 - expected_long).abs() < 1e-3,
            "long doc score != closed form ({} != {expected_long})",
            score_of(1)
        );
        assert!(
            score_of(0) > score_of(1),
            "shorter doc must score higher (length normalization)"
        );
    }
}
