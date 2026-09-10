//! Tantivy index lifecycle: create, write, commit, read.
//!
//! Wraps `tantivy::Index` with Akar-specific convenience:
//! - [`TantivyIndex::create_on_disk`] — persistent index at a directory path
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
use tantivy::{DocAddress, Index, IndexReader, IndexWriter, ReloadPolicy, Score, TantivyDocument};

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
}
