# Akar Full-Text Search Extension

Full-text search for the Akar database engine using **Tantivy** (pure Rust search library).

## Architecture

- **`TantivyIndex`** (`index.rs`) — Wrapper around Tantivy `Index` with Akar-convenience constructors:
  - `create_on_disk(index_dir, schema)` — persistent on-disk index (MmapDirectory)
  - `create_in_memory(schema)` — fully in-memory index (ideal for tests)
  - `open_on_disk(index_dir)` — open existing index without providing schema
  - `writer(num_threads, overall_mem_budget)` — build `IndexWriter` with per-thread budget (min 15 MB)
  - `reader()` — `IndexReader` with `ReloadPolicy::Manual` (caller must call `reload()` after commit)
  - `search(reader, query_str, search_fields, limit)` — convenience search returning `(Score, DocAddress)` pairs

- **`FtsIndexHandle`** — Runtime handle for FTS indexes in the storage layer, managing Tantivy index lifecycle and commit-hook sync.

- **SQL DDL** — `CREATE FTS INDEX name ON table(column) [WITH TOKENIZER('...')] [IF NOT EXISTS]` and `USING FTS INDEX` in `MATCH` clauses.

- **Tokenizer** — Configurable via `WITH TOKENIZER('en_stem')` (P109.1); defaults to `en_stem` via `akar_fts::tokenizer::resolve`.

- **Legacy removed** — The legacy `fts_{idx}_docs` / `fts_{idx}_terms` / `fts_{idx}_appears_in` macro tables are **gone** (P104.2 clean break) — the Tantivy directory is the *only* FTS representation.

## Commit-hook sync

FTS indexes are kept current via commit hooks that apply document writes (insert/update/delete) to the Tantivy index at transaction commit time. See `akar-core/akar-fts/src/build.rs` for `append_docs` and `apply_doc_writes`.

## Tests

53 tests covering: index creation, document write application, search, tokenizer resolution, schema round-trips, and commit-hook sync.