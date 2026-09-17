# Deep Exploration — akar-fts

Full-text search on Akar is backed by a Tantivy-derived inverted index. `CREATE FTS INDEX` builds the index synchronously at DDL time; subsequent inserts are caught up automatically so the index never goes stale; queries with `MATCH ... USING FTS INDEX` score results with BM25 or LETOR. The crate ships with stemmer (Porter), tokenizer, stop-words, and the TF-IDF/BM25 primitives required for ranking.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `InvertedIndex` | Tantivy-backed inverted index | `akar-core/akar-fts/src/lib.rs` |
| `FtsMetadata` | Per-table FTS index config (stemmer, stop words, tokenizer) | `akar-core/akar-fts/src/` |
| `Stemmer` (Porter) / `Tokenizer` | Normalization pipeline | `akar-core/akar-fts/src/` |
| `TfIdf` / `Bm25` | Scoring models | `akar-core/akar-fts/src/` |
| registered functions | `stemmer`, `tokenizer`, `tf_idf`, `bm25`, `query`, `stop_words` | `akar-core/akar-fts/src/lib.rs` |
| FTS DDL + sync | `PhysicalCreateFtsIndex`, row catch-up | `akar-core/akar-processor/src/physical/write_ops/ddl_fts.rs`, `fts_sync.rs` |

## Design Decisions

- **Synchronous catch-up (verified).** `test_fts_catches_up_rows_after_index` proves rows inserted after `CREATE FTS INDEX` are searchable, and soft-deleted rows stop matching. The design chose synchronous sync over a lazy background rebuild to keep query semantics deterministic.
- **Scoring exposed as functions.** `tf_idf`, `bm25`, `query`, `stop_words` are ordinary registered functions, so the same primitives used by `PhysicalFtsScan` are available to users for custom ranking — and to `akar-search` for hybrid fusion.
- **Tantivy for the inverted index.** Using a battle-tested indexer (as in Kuzu's FTS extension) avoids re-implementing postings lists; Akar wraps it with its own metadata and sync logic.

## Why It Matters

Lexical recall complements vector semantic recall: misspellings, rare tokens, and exact phrases are exactly what BM25 ranks well. FTS is the second channel in `akar-search`'s hybrid recall and the default keyword path for `MATCH USING FTS INDEX`. The unify/deduplicate semantics of `FTS` (per its node) directly influence recall quality in agent memory queries.