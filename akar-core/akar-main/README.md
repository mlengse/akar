# Akar Main

Public API for the Akar database engine.

**`Database`** — Main entry point. Initializes all subsystems (storage, transaction
manager, catalog, function registry, extensions, stats store).

**`Connection`** — Query execution. Full pipeline: `parse → bind → plan → optimize →
execute`. Handles DDL, DML, COPY FROM, and prepared statements.

**`QueryResult`** — Result encapsulation with row/column counts, display formatting, iterator over columns.

**`PreparedStatement`** — Parameterized query preparation and execution with `$param` syntax.

**`SystemConfig`** — Configuration (buffer pool size, threads, compression, auto-checkpoint, read-only mode).

**`ConnectionPool`** (`pool.rs`) — Reusable connection pool over a shared `Arc<Database>`. Cheap to share behind an `Arc`; every method takes `&self`. Provides `get()`, `run()`, `idle_count()`, `stats()`.

**Typed bulk APIs** (`bulk.rs`) — `insert_nodes<T: TypedNode>`, `insert_edges`, `insert_edges_chunked` with batched UNWIND statements and row-scoped vector index maintenance.

**Row-scoped vector maintenance** — `Database::refresh_vector_indexes_for_rows(&[(u64, u64)])` — re-reads only the named rows; cost is O(scope). Used automatically by bulk write paths after commit.

**Tests:** ~81 unit + ~409 integration (verify live with `cargo test -p akar-main --lib` and `cargo test -p akar-main --test integration`)

Enable the `vector-extension` feature for HNSW vector indexes and the `markdown-extension` feature for the `read_markdown_wiki` table function.