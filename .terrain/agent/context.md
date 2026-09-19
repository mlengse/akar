---
type: agent_context
project: akar
title: Agent Architecture Context
source: .
---

## Project Overview
Akar is a **pure Rust, embedded graph database for AI agent memory**, and a from-scratch reimplementation of KuzuDB (archived C++ graph DB from U. of Waterloo). It keeps Kuzu's architectural DNA — **Cypher query language, worst-case optimal joins, factorized execution, columnar storage, MVCC** — with **zero C++ and zero FFI** in the core engine. Performance is validated at 3-way parity against the C++ originals (397 µs vs 400 µs vs 374 µs on a 10K-row hot path). Primary consumer is an AI agent memory engine (`sulur`, ex

-`kairos`, ships via PyPI `akar>=X.Y.Z`), but akar also ships a CLI shell, a blocking-network server daemon, Python/C/WASM bindings, and an ADBC source. Delivery is **spec-driven**: `SPEC.md` is the live contract; every batch gates on `cargo test` (`0 failed / 0 ignored`), clippy `-D warnings`, and fmt.

## Architecture
A Cargo workspace of ~35 crates under `akar-core/` organized into layers. Pure-Rust core (ADR-002, ADR-001 pest-over-antlr), column-major storage (ADR-004), MVCC transactions (ADR-005).

| Layer | Role | Key crates / paths |
|---|---|---|
| Server-side / vectorized engine | Parallel push-based execution over Arrow data chunks; operator mapper binder + physical operators | `akar-processor` |
| Aggregation / hash tables | Split single-pass aggregation, radix sort, block-merge sort, top-K | `akar-processor/src/physical/order_aggregate` |
| Query shaping | AST → bound statement → logical plan → optimized physical plan; join ordering; WCOJ `Intersect` | `akar-parser` → `akar-binder` → `akar-planner` → `akar-optimizer` |
| Physical storage | Columnar node-groups with per-column chunks, ART index, page/buffer management, WAL + checkpoint + crash recovery, compression, LSM-ish persistence | `akar-storage` |
| Cardinality / stats & vector index | Per-column statistics, HNSW vector index, SIMD distance kernels | `akar-storage/src/vector_index.rs`, `akar-vector` |
| Embedded library entry | `Database`, connection sessions, plan cache, table functions, query result, storage driver to disk DBs | `akar-main` (incl. `src/connection/*`) |
| Stacking assemblies | Runs Berry/CLI, daemon server, Rust/C/Python/WASM API entrypoints | `akar-cli`, `akar-server`, `akar-c`, `akar-python`, `akar-wasm`, `akar-migrate` |
| Extensions | FTS (Tantivy), ANN, GDS, hybrid search, scalar/aggregate functions, ML, Dream consolidation, JSON, object-store formats | `akar-fts`, `akar-vector`, `akar-graph`, `akar-algo`, `akar-search`, `akar-function`, `akar-ml`, `akar-dream`, `akar-json`, `akar-duckdb`, `akar-azure`, `akar-httpfs`, `akar-iceberg`, `akar-delta`, `akar-unity-catalog` |
| Compatibility | Interop adapters for foreign engines | `akar-neo4j`, `akar-postgres`, `akar-sqlite` |

**WCOJ + parallel execution:** `QueryProcessor` runs a mapper/scheduler over physical operator trees; parallel scans feed hash-join PMCs that materialize worst-case optimal joins, with per-splunk factorized pass back.

## Module Map
| Module (crate dir) | Responsibility | Primary paths |
|---|---|---|
| `akar-main` | Embedded library facade, `Database`, connection sessions, DDL/DML/COPY orchestration, plan cache, ADBC, server client | `akar-core/akar-main/src/` |
| `akar-parser` | Cypher grammar via pest, AST for DDL/DML/expressions, `cypher.pest` | `akar-core/akar-parser/src/parser` |
| `akar-binder` | AST → `BoundStatement`, DDL bind, confidential statement analysis, bound plan owner | `akar-core/akar-binder/src/binder` |
| `akar-planner` | Logical operators, logical planner, join-order enumeration | `akar-core/akar-planner/src` |
| `akar-optimizer` | 25 rule passes (18 flat + 7 tree): WCOJ/factorization, constant folding, filter/projection pushdown, aggregate fusion, ART range scan, FTS & vector-similarity pushdown, subquery unnesting | `akar-core/akar-optimizer/src/passes` |
| `akar-processor` | Physical operator execution, expression evaluator, scan/filter/join/aggregate/write operators, plan serializer, graph-source projection | `akar-core/akar-processor/src/physical`, `.../processor` |
| `akar-storage` | Columnar storage engine, node-groups, ART index, WAL/checkpoint/recovery, parquet/CSV/npy readers, stats, HNSW vector index | `akar-core/akar-storage/src` |
| `akar-transaction` | MVCC transaction lifecycle, commit history, isolation visibility | `akar-core/akar-transaction/src` |
| `akar-common` | Arrow vectors/DataChunk, types, selection, memory accounting, VFS, task system | `akar-core/akar-common/src` |
| `akar-function` | Scalar + aggregate function registry (arithmetic, string, date, cast, list/map, JSON path…) | `akar-core/akar-function/src/scalar`, `src/aggregate` |
| `akar-vector` / `akar-fts` / `akar-search` / `akar-graph` / `akar-algo` | ANN (HNSW + SIMD), full-text (Tantivy), hybrid/RRF search, GDS (BFS/random-walk,node2vec/Louvain/LPA) | `akar-core/akar-vector/src`, `akar-fts/src`, `akar-search/src`, `akar-graph/src/gds`, `akar-algo/src/gds` |
| `akar-llm` / `akar-ml` / `akar-dream` | Embedding API clients, LSTM/SBYO/sparse models, memory-consolidation sleep-cycle phases (NREM → SUPERSEDES → REM → Insight → AFE → Synthesis → DAE) | `akar-core/akar-llm/src`, `akar-ml/src`, `akar-dream/src/phases` |
| `akar-duckdb` + attach family | DuckDB delegation for external formats: Delta, Iceberg, Azure Blob, Unity Catalog | `akar-core/akar-duckdb/src`, `akar-delta`, `akar-iceberg`, `akar-azure`, `akar-unity-catalog` |

## Core Flows
1. **Query execution (read path):** session statement → `CyParser` parses Cypher → binder produces `BoundStatement` → planner builds logical plan (+ join order) → optimizer applies 25 rewrites (WCOJ factorization, predicate/vector/FTS pushdown) → `QueryProcessor` maps to physical operators → parallel scan/filter/join/aggregate over Arrow `DataChunk`s → results committed through the plan-cache-aware session.
2. **Write path + durability:** DDL/DML bind and execute inside an MVCC transaction → rows appended/updated to columnar node-groups, WAL log appended first → commit publishes visibility via O(1) `commit_history` lookup → async checkpoint + group commit persist dirty pages to disk → crash recovery replays WAL at startup.
3. **Vector ANN query:** `MATCH (n) WHERE cosine_similarity(n.emb, $q) > thr RETURN … ORDER BY cos DESC LIMIT k` is rewritten by the `VectorSimilarityDetection` pass into `[VectorSimilarityScan(HNSW) → Filter(cos>thr) → OrderBy → Projection → Limit]`; explicit `CALL vector_similarity_scan(…)` runs the same HNSW read path (distance kernels: SSE2/AVX/NEON SIMD with scalar fallback).
4. **FTS lifecycle:** `CREATE FTS INDEX` builds a Tantivy index (on-disk, per table/column) → inserts/deletes propagate to the index on **durable commit only** (aborted/rolled-back writes never leak) → `USING FTS INDEX` predicates push down to the scan leaf, cardinality is estimated from Tantivy `doc_freq` (min with table cardinality) without executing the search.

## Tech Stack
- **Language:** Rust; deliberately **no FFI / no C++** in the core engine (ADR-002); SIMD intrinsics via `#[target_feature]` (`is_x86_feature_detected!` SSE2/AVX, NEON).
- **Query front-end:** Cypher; grammar in **pest** (`cypher.pest`, ADR-001; no ANTLR).
- **Columnar runtime:** Apache Arrow vectors/`DataChunk`s (`akar-common/src/arrow_vector.rs`), column-major storage (ADR-004).
- **FTS:** Tantivy; **ANN:** self-contained HNSW (`akar-vector/src/hnsw.rs`) + SIMD distance kernels.
- **External format read:** DuckDB (libduckdb C++ extension) delegated by `akar-duckdb/src/attach_helper.rs` for Delta/Iceberg/Azure/Unity Catalog; Parquet/CSV/NPY/JSON readers born in `akar-storage`.
- **Concurrency:** worker-thread scheduler (`akar-common/src/task_system.rs`), parallel scans, group commit; MVCC snapshot isolation.
- **Embeddings:** HTTP clients for OpenAI-compatible endpoints (OpenAI, Ollama, VoyageAI, Bedrock, Vertex/Gemini).
- **Build/test/CI:** cargo workspace (35 crates, `max_width=120` rustfmt), gate `cargo test` (~2,060 tests, 0 failed/0 ignored), clippy `-D warnings`, GitHub Actions (`rust-ci.yml`, `rust-release.yml`); fuzz targets (nightly) in `akar-core/fuzz`, prop-test in `test_proptest.rs`. Releases automated bottom-up via `tools/release.py` + `tools/doc-check.py`.

## System Boundaries
- **Trust boundary — SQL/blang input:** arbitrary Cypher and COPY payloads are attacker-adjacent; fuzzed (`cypher_query`, `expression_eval`, `copy_from_csv`, `wal_bytes`, `compression_roundtrip`); malformed CSV/JSON/UTF-8 ingestion is covered by dedicated dataset suites under `dataset/`.
- **External data-plane (read-only attach):** DuckDB-driven Delta Lake, Apache Iceberg, Azure Blob Storage (with `CREATE SECRET`), Unity Catalog — credential secrets flow through attach setup SQL, not the core engine.
- **Embedding providers:** outbound HTTP only at query time; timeouts + no-keystore (keys supplied by caller config).
- **Foreign-engine interop:** `akar-neo4j`, `akar-postgres`, `akar-sqlite`, `akar-json` import/convert outside objects; results cross the type-boundary via Arrow/Value conversion.
- **Server daemon:** blocking TCP server (`akar-server/src/bin/akar_server.rs`) with idle-timeout auto-shutdown; session/ping protocol in `remote.rs`/`session.rs`.
- **On-disk format contract:** `STORAGE_VERSION = 1` (`akar-storage/src/version_info.rs`); bump without migration ⇒ required `0.2.0` SemVer release.
- **Release/versioning:** crates publish bottom-up with crates.io rate limiting; tag `v*` triggers 3-OS CLI binary builds + GitHub Release.

## Code Map Index
| Concept | Location | Notes |
|---|---|---|
| Query processor / exec context | `akar-core/akar-processor/src/processor/mod.rs`, `physical_operator.rs` | Parallel vectorized exec |
| Optimizer passes | `akar-core/akar-optimizer/src/passes/{flat,tree}` | 18 flat + 7 tree rules |
| WCOJ / factorization | `akar-core/akar-optimizer/src/passes/tree/factorization.rs`, `akar-planner/src/join_order.rs` | `Intersect` WCOJ plan shape |
| Vector similarity rewrite | `akar-core/akar-optimizer/src/passes/flat/vector_similarity.rs` | HNSW scan rewrite (P71.4) |
| HNSW + distance kernels | `akar-core/akar-vector/src/hnsw.rs`, `distance.rs` | SSE2/AVX/NEON |
| Storage engine | `akar-core/akar-storage/src/table.rs`, `node_group.rs`, `page_manager.rs`, `wal.rs` | Columnar, MVCC, group commit |
| Transaction / MVCC | `akar-core/akar-transaction/src/lib.rs`, `akar-storage/src/version_info.rs` | O(1) commit-history visibility |
| DDL/DML/COPY sessions | `akar-core/akar-main/src/connection/{ddl,dml,copy,query}.rs` | Session handlers |
| FTS index + pushdown | `akar-core/akar-fts/src/index.rs`, `akar-optimizer/src/passes/tree/fts_predicate_pushdown.rs`, `akar-main/src/connection/fts_estimate.rs` | Tantivy, commit-gated sync |
| GDS / graph algorithms | `akar-core/akar-graph/src/gds`, `akar-algo/src/gds` | BFS, random walk, node2vec, Louvain, LPA |
| ML / Dream consolidation | `akar-core/akar-ml/src`, `akar-dream/src/orchestrator.rs` + `phases/` | LSTM, SBYO, NREM/REM sleep-cycle |
| Server daemon | `akar-core/akar-server/src/bin/akar_server.rs`, `session.rs` | TCP, idle shutdown |
| Bindings & CLI | `akar-core/akar-cli/src/main.rs`, `akar-python/src`, `akar-c/src/lib.rs`, `akar-wasm/src/lib.rs`, `akar-main/src/adbc.rs` | Python/WASM/C, ADBC |
| Spec / plan / release | root `SPEC.md`, `CHANGELOG.md`, `implementation plan.md`, `tools/release.py` | Spec-driven governance |
| Benchmarks | `akar-core/akar-main/benches/*.rs`, `akar-storage/benches/hybrid_eval.rs` | Parity vs C++ baselines |