---
type: agent_context
project: akar
title: Agent Architecture Context
source: .
---

## Project Overview
Akar is a **pure-Rust, embedded graph database** built to be the durable memory of an AI agent: it stores knowledge-graph facts (nodes/relationships) speakable in an OpenCypher-compatible dialect, alongside dense `float32` vectors so recall becomes similarity search. It reimplements the KuzuDB architecture from scratch in safe Rust with **zero FFI in the core** (ADR-002), embedding in-process: `Database::new(path, config)` → `Connection::query(cypher)` → `QueryResult`, no server round-trip. Consumers are agent memory systems (e.g. PyPI `sulur`), plus a TCP daemon, CLI, C FFI, WASM and Python bindings. Constraints: GPL-3.0-or-later, Rust edition 2024, WASM-viable, strictly layered 35-crate Cargo workspace under `akar-core/` (~106K LOC, **2,091 tests, 0 failed**, gate 2026-09-17). Development is **SPEC-driven** — `SPEC.md` is the living spec; ADRs in `akar-core/docs/adr/`.
## Architecture
| Dimension | Design |
|---|---|
| Query pipeline | Single chain in `Connection::query`: **parse** (pest PEG → AST, 33 `Statement` variants) → **bind** (catalog-aware `Binder` → 33 `BoundStatement`) → **plan** (`QueryPlanner` → 59 `LogicalOperator` variants) → **optimize** (26 rule passes: 19 flat + 7 tree, pinned by test) → **execute** (`QueryProcessor`, 55 physical operators) → `QueryResult` |
| Execution model | Vectorized Volcano: every `PhysicalOperatorExec` maps `Vec<DataChunk>` → `Vec<DataChunk>`; `DataChunk` = Arrow `ArrayRef` columns + physical types + optional `SelectionVector` (row indirection). Hot path uses `ExpressionEvaluator::evaluate_to_arrow` (Arrow compute kernels, no `Value` boxing); parallel agg/hash-join via rayon `TaskSystem`. **F6 memory blow-up mitigation:** Extend column pruning (`ExtendPrune` — conservative downstream-tail analysis materialises only referenced columns; identity `id`/`_id` always kept); `PhysicalExtend` borrows rel/dest catalog tables (no per-execution wholesale clone of adjacency/columns), resolves neighbours via adjacency-index lookups (`scan_adj_list`/`scan_rev_adj_list`) and reads destination/properties lazily per emitted row; trailing LIMIT budgets push down to both `PhysicalCrossProduct` (`execute_binary_budgeted`) and `PhysicalExtend` (`limit_budget`); unpruned extends/cross-products enforce safety caps (`AKAR_MAX_EXTEND_ROWS` 5M / `AKAR_MAX_CROSS_ROWS` 100K, env-overridable) to prevent OOM |
| Optimizer | Two-phase, ordered, rule-based: flat passes (`ExtendFilterPushDown` hoisting source-only predicates above `Extend` so anchored hops cost O(degree); filter/predicate/projection pushdown, top-k, DP-bushy join reorder, `VectorSimilarityDetection` rewriting cosine→HNSW scan, ART range scan) then 7 bottom-up tree passes (factorization, foreign-join, acc-hash-join, subquery unnesting, agg-key-dependency, cardinality, **FTS predicate pushdown**). `FtsCardinalityEstimator` blends Tantivy `doc_freq` into cardinality (zero-extra-probe) |
| Storage | Column-major: `NodeTable`/`RelTable` → `NodeGroup` (4096 rows) → `ColumnChunk`; `BufferManager` page cache + `PageManager`/`FreeSpaceManager`; append-only typed **WAL** (magic `b"AKAR"`, CRC32, self-sufficient replay) + `WalReplayer` + `ShadowFile`/`Checkpoint`; CSR adjacency for rels; ART PK i

…
## Module Map
| Module | Responsibility | Primary paths |
|---|---|---|
| Query Frontend | Cypher text → AST (33 stmt) → catalog-resolved `BoundStatement`; pest-PEG superset of C++ grammar | `akar-core/akar-parser`, `akar-core/akar-binder` |
| Planning & Optimization | Logical plans (59 ops), 26-pass rewrite, join reorder, EXPLAIN | `akar-core/akar-planner`, `akar-core/akar-optimizer` |
| Query Execution | Physical pipeline over Arrow chunks; logical→physical mapper; 259 functions; SIMD distance kernels; Extend column pruning; cross-product row budgets | `akar-core/akar-processor`, `akar-core/akar-function` |
| Storage Engine | Columnar persistence, buffer/pages, WAL+recovery, CSR, ART/HNSW/hash indexes, compression, CSV/Parquet/NPY IO, spill; vector index restore from catalog at startup | `akar-core/akar-storage` |
| Transactions | MVCC + OCC, two-phase commit, undo, checkpoint coordination, group-commit | `akar-core/akar-transaction` |
| Catalog & Schema | Tables, sequences, macros, type aliases, FTS/vector index entries, foreign tables; `catalog.json` persistence | `akar-core/akar-catalog` |
| Graph Algorithms & GDS | CSR engine + 18 algorithms as `CALL`-able table functions | `akar-core/akar-graph`, `akar-core/akar-algo` |
| Search & Record | Tantivy BM25 FTS (commit-gated visibility, predicate pushdown, per-index tokenizer incl. CJK), HNSW vector ANN (quickselect O(N+k log k) top-k), **native BM25 + hybrid fusion (RRF) + hybrid/multi scans**, JSON fns | `akar-core/akar-fts`, `akar-vector`, `akar-json`, `akar-search` |
| Data Integration Extensions | Attach/scan external stores (httpfs/S3, DuckDB, SQLite, Postgres, Neo4j Bolt, Delta, Iceberg, Azure, Unity Catalog) | `akar-core/akar-{httpfs,duckdb,sqlite,postgres,neo4j,delta,iceberg,azure,unity-catalog}` |
| LLM · ML · Dream | LLM fns, ML embed/training (multi-layer LSTM `LstmModel<F>` f32/f64 incl. online `train_pair` BPTT, `forward_sequence_hidden` for variable-length sequence hidden-state extraction, node2vec, sparse; binary persistence `save_bin`/`load_bin`), memory-consolidation orchestrator (NREM→…→DAE) | `akar-core/akar-llm`, `akar-ml`, `akar-dream` |
| Public API & Shells | `Database`/`Connection`/`QueryResult`/`PreparedStatement`; CLI REPL, TCP server, C FFI, WASM, Python (`akar`, Kuzu drop-in), migrate | `akar-core/akar-main`, `aka

…
## Core Flows
1. **Query execution** — `Connection::query` parses → binds against `Arc<Mutex<Catalog>>` → plans → optimizes (26 passes; plan-cache hit skips parse→optimize, LRU, invalidated by catalog version) → captures MVCC `(snapshot_ts, commit_history)` → `QueryProcessor` executes operator tree over `DataChunk`s → `QueryResult`. Extend column pruning collects downstream-tail references at top level (disabled for child sub-plans); `forward_limit_budget` pushes LIMIT+SKIP down to `PhysicalCrossProduct` and `PhysicalExtend`. DDL writes `catalog.json` and signals checkpoint; writes re-wrap then refresh vector indexes.
2. **Write transaction (commit)** — classify via `is_write_statement` → snapshot + `transaction_id` → single-writer table locks OR row-level OCC `record_write(txn_id, table_id, row_id)` → operators mutate tables in place while emitting typed `WALRecord`s + `UndoRecord`s (buffered in `LocalWAL`) → `prepare_commit` validates OCC write set → durable `StorageManager::commit_transaction` (WAL → flush → shadow-apply → checkpoint; group-commit batching) → `finish_commit` publishes timestamp; losers replay undo records; FTS indexes sync on commit before publish.
3. **Crash recovery** — open DB dir → `akar.lock` (exclusive write / shared read-only) → load `catalog.json` + `STORAGE_VERSION` gate → `WalReplayer` redoes committed WAL records (WAL is the sole recovery source; mirrors re-persisted only at checkpoint/recover) → truncate WAL, restore vector indexes from catalog entries (`restore_vector_index`), rebuild in-memory indexes (ART/HNSW/hash) including HNSW graphs for restored vector indexes → ready.
4. **Vector similarity search** — flat pass `VectorSimilarityDetection` rewrites `MATCH … WHERE cosine_similarity(n.col, q)>thr ORDER BY cos DESC LIMIT k` into `[VectorSimilarityScan(HNSW), Filter, OrderBy, Projection, Limit]`; explicit `CALL vector_similarity_scan(…)` also available; HNSW graphs refreshed after DML; SIMD dot/norm kernels in `akar-vector`.
5. **Hybrid text+vector search** — `akar-search` combines FTS/BM25 doc-id filters and vector scans, fusing rankings via Reciprocal Rank Fusion (native BM25 in `native_bm25.rs`, hybrid/multi scans); end-to-end `MATCH … USING FTS INDEX` and `CALL` entry points.
6. **Dream consolidation** — `DreamOrchestrator::run_cycle` runs NREM → SUPERSED

…
## Tech Stack
- **Language/packaging:** Rust edition 2024, Cargo workspace (35 crates under `akar-core/`), `0.2.2` SemVer, GPL-3.0-or-later; CI enforces `-Dwarnings` + clippy + fmt (`max_width=120`, rustfmt.toml).
- **Runtime core:** `arrow`/`parquet` 59 (columnar batch + IO), `rayon` (task system / parallel agg / join), `hashbrown`+`ahash`, `serde`/`serde_json` (catalog), `thiserror`, `tracing`, `pest` (PEG parser, replaces ANTLR4), `criterion` (benches).
- **Search/vector:** Tantivy 0.26 (BM25 FTS), native HNSW (ahash keyed, optimized beam search + quickselect top-k) + SIMD single-pass distance kernels, native BM25 + RRF hybrid fusion.
- **Extensions (feature-gated into `akar-main`):** Rust DuckDB, rusqlite, tokio-postgres, Neo4j Bolt client, HTTP/S3 VFS, PyO3/maturin (Python), wasm-bindgen (WASM), rustyline (CLI REPL), clap+ctrlc (server), MD-5/SHA-2/base64 (utility fns). `akar-server` exposes `json-extension`, `fts-extension`, `vector-extension`, `parquet-export`, `full` feature gates.
- **Infra:** GitHub Actions CI/CD (`.github/workflows/rust-ci.yml`, `rust-release.yml`), `tools/release.py` full release pipeline, fuzz targets (nightly) for parser/WAL/CSV/expression paths.
- **Data model:** nodes/rels + dense `float32` vectors; internal columns `_id`, `_label`, `_src`, `_dst`, `_rel_id`.
## System Boundaries
| Boundary | Contract |
|---|---|
| I/O surface | `akar.lock` cross-process lock (2nd writer rejected); `catalog.json` atomic rename; WAL/checkpoint files; Tantivy FTS index dirs `<db_path>/fts/<idx>`, FTS visibility commit-gated (aborted writes never searchable); `:memory:` in-memory mode skips persistence; WASM always in-memory |
| TCP server (port 9876) | Length-prefixed JSON frames `[u32 LE][payload]`, 128 MiB cap; ops `query/ping/flush/stats/export/shutdown/dream_control`; optional 32-byte hex auth token on each request; `RemoteDatabase` client with stale-frame/desync handling; **out of scope: multi-process writers over shared files** |
| C FFI (`akar-c`) | `extern "C"`, `panic="abort"` + catch wrapper, `aker_*` symbols; error strings freed only via `akar_error_message_free`; `concurrent_writes`/`spill_threshold` pinned true/0 (not exposed) |
| Python (`akar`) | PyO3, Kuzu drop-in; params interpolated Python-side into Cypher text (injection surface); `close()` releases file lock; submodules `knn/search/spread/louvain/dream/lstm` (incl. multi-layer `LstmModel` with `num_layers`, `forward_sequence_hidden`, online `train_pair`, and `save_bin`/`load_bin` binary persistence); `akar.lstm` registered in `sys.modules` |
| WASM | In-memory only; flat JS-object params; result rows as objects keyed by column name |
| Extension crates | Register into `FunctionRegistry` via `ExtensionContext` / `CatalogGraphSource` (`TableFunction::CustomTableWithGraph`); several require network or cloud credentials |
| External services | LLM providers (`OPENAI_API_KEY`), embedding model (`AKAR_EMBED_MODEL`, default `BGESmallENV15`), Azure Blob (`AZURE_STORAGE_ACCOUNT`/`AZURE_STORAGE_SAS_TOKEN`); active only with matching feature crates — core reads no env vars |
| Trust model | Read-only mode rejects writes; `SET spill_threshold` / `SET concurrent_writes` runtime knobs; OCC conflicts fail cleanly (`WriteConflict`); poisoned locks detected; memory governor prevents runaway buffering; Extend/cross-product safety caps prevent OOM on large graph scans |
| Version gates | `STORAGE_VERSION` bump without auto-migration is breaking → SemVer `0.2.0`; optimizer pass count pinned at 26 |
## Code Map Index
| Concept | Location | Notes |
|---|---|---|
| Engine entry (`Database`, `SystemConfig`) | `akar-core/akar-main/src/database.rs` | `Database::new`, locks, catalog/file paths, vector + ART index APIs; vector index restore + HNSW rebuild at startup |
| Query pipeline orchestration | `akar-core/akar-main/src/connection/query.rs` | parse→bind→plan→optimize→execute, plan cache, transaction wrap, FTS wiring |
| Connection & txn layer | `akar-core/akar-main/src/connection/` | `mod.rs`, `transaction.rs`, `ddl.rs`, `dml.rs`, `copy.rs`, `fts_estimate.rs`, `standalone_call.rs`, `plan_cache.rs` |
| Public result types | `akar-core/akar-main/src/{query_result,prepared_statement,storage_driver,remote}.rs` | `QueryResult`, `PreparedStatement`, `StorageDriver`, `RemoteDatabase` |
| Parser (pest) | `akar-core/akar-parser/` | `cypher.pest` grammar, `src/ast.rs`, `src/parser/{ddl,dml,expression}.rs` (incl. FTS tokenizer clause) |
| Binder | `akar-core/akar-binder/` | `src/bound_statement.rs`, `src/lib.rs` |
| Logical planning | `akar-core/akar-planner/src/` | `planner.rs`, `logical_operator.rs`, `join_order.rs` |
| Optimizer | `akar-core/akar-optimizer/src/` | `optimizer.rs`, `passes/flat/` (incl. `extend_filter_pushdown.rs`, `vector_similarity.rs`, `art_range_scan.rs`), `passes/tree/` (incl. `fts_predicate_pushdown.rs`, `cardinality.rs`); `fts_estimate.rs` |
| Execution core | `akar-core/akar-processor/src/` | `processor/mod.rs` (`QueryProcessor`, `forward_limit_budget`), `processor/mapper/`, `physical/` (incl. `PhysicalCrossProduct` row caps, `PhysicalExtend` column pruning + `limit_budget` + adjacency-index lookups, `fts_sync.rs` FTS commit sync), `expression_evaluator.rs`, `processor/extend_prune.rs` (`ExtendPrune`, `collect_extend_prune`, `keep_column`) |
| Function library | `akar-core/akar-function/src/` | `registry.rs` (259 fns), `scalar/`, `aggregate/`, `graph.rs` |
| Storage engine | `akar-core/akar-storage/src/` | `lib.rs` (`StorageManager`, `restore_vector_index`), `table.rs` (`create_vector_index_with_id`), `wal.rs`/`wal_replayer.rs`, `checkpoint.rs`, `buffer_manager.rs`, `csr.rs`, `art_index.rs`, `vector_index.rs`, `column_chunk.rs`, `memory_account.rs` (group-commit) |
| Transactions | `akar-core/akar-transaction/src/lib.rs` | `TransactionManager`, `Transaction`, OCC write-set/undo |


…