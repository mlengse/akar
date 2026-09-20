---
type: agent_context
project: akar
title: Agent Architecture Context
source: .
---

## Project Overview

Akar is a **pure-Rust, embedded graph database for AI agent memory** — a from-scratch reimplementation of KuzuDB (archived C++ graph DB, U. of Waterloo) with **zero C++ and zero FFI** (ADR-002). It keeps Kuzu's architectural DNA: Cypher query language (pest grammar, ADR-001), worst-case optimal joins (WCOJ), factorized execution, column-major storage (ADR-004), and MVCC (ADR-005). Hot-path performance is validated at 3-way parity with the C++ originals (397 µs vs 400 µs vs 374 µs on a 10K-row query). Primary consumer is the `sulur` AI memory engine (ships as PyPI `akar>=X.Y.Z`); akar also ships a CLI shell, a TCP daemon server, Python/C/WASM bindings, and an ADBC source. Governance is **spec-driven**: `SPEC.md` is the live contract; every batch gates on `cargo test` (0 failed / 0 ignored), clippy `-D warnings`, and fmt.

## Architecture

A Cargo workspace of ~35 crates under `akar-core/`, layered bottom-up; `akar-main` is the embedded library facade.

| Layer | Role | Key paths |
|---|---|---|
| Query front-end | Cypher grammar (pest), AST, DDL/DML/expression parsing | `akar-core/akar-parser/src/` |
| Semantic binding | AST → `BoundStatement`, DDL bind, confidential-statement analysis | `akar-core/akar-binder/src/` |
| Logical planning | Logical operators/planner, join-order enumeration | `akar-core/akar-planner/src/` |
| Optimization | 25 rule passes (18 flat + 7 tree): WCOJ/factorization, pushdowns, aggregate fusion, subquery unnesting, FTS & vector-similarity rewrites | `akar-core/akar-optimizer/src/passes/` |
| Physical execution | Parallel push-based vectorized execution over Arrow `DataChunk`s; scan/filter/join/aggregate/write operators + expression evaluator | `akar-core/akar-processor/src/physical/`, `src/processor/`, `src/expression_evaluator.rs` |
| Storage | Columnar node-groups, per-column chunks, ART index, page/buffer manager, WAL + checkpoint + crash recovery, compression, stats, HNSW vector index | `akar-core/akar-storage/src/` |
| Transactions | MVCC lifecycle, commit history, snapshot visibility | `akar-core/akar-transaction/src/` |
| Embedding facade | `Database`, sessions, DDL/DML/COPY orchestration, plan cache, table functions, storage driver | `akar-core/akar-main/src/` (incl. `connection/`) |
| Extensions | FTS, ANN, GDS algorithms, hybrid search, scalar/aggregate functions, ML models, Dream consolidation, JSON, object-store formats | `akar-fts`, `akar-vector`, `akar-graph`, `akar-algo`, `akar-search`, `akar-function`, `akar-ml`, `akar-dream`, `akar-json` |
| Interop adapters | Foreign-engine compatibility | `akar-duckdb`, `akar-delta`, `akar-iceberg`, `akar-azure`, `akar-unity-catalog`, `akar-neo4j`, `akar-postgres`, `akar-sqlite` |
| Entry points & bindings | CLI shell, TCP daemon, Rust/C/Python/WASM/ADBC/migrate | `akar-cli`, `akar-server`, `akar-c`, `akar-python`, `akar-wasm`, `akar-migrate` |

**Execution model:** `QueryProcessor` runs a mapper/scheduler over physical operator trees; parallel scans feed hash-join PMCs that materialize WCOJ plans, with factorized pass back per projection group.

## Module Map

| Module (crate dir) | Responsibility | Primary paths |
|---|---|---|
| `akar-main` | Embedded facade: `Database`, sessions, DDL/DML/COPY, plan cache, ADBC, remote client, table functions | `akar-core/akar-main/src/`, `src/connection/`, `src/database.rs`, `src/adbc.rs` |
| `akar-parser` | Cypher grammar + AST (pest, `cypher.pest`) | `akar-core/akar-parser/src/parser/`, `src/ast.rs` |
| `akar-binder` | AST → bound statements, DDL binding | `akar-core/akar-binder/src/binder/`, `src/bound_statement.rs` |
| `akar-planner` | Logical plan construction, join ordering, WCOJ `Intersect` shape | `akar-core/akar-planner/src/planner.rs`, `src/join_order.rs`, `src/logical_operator.rs` |
| `akar-optimizer` | Rule-based rewrites (18 flat + 7 tree passes) | `akar-core/akar-optimizer/src/passes/{flat,tree}/`, `src/optimizer.rs`, `src/join_order.rs` |
| `akar-processor` | Physical operators, expression evaluator, plan serializer, write ops, graph-source projection | `akar-core/akar-processor/src/physical/`, `src/processor/`, `src/physical/write_ops/` |
| `akar-storage` | Columnar engine: node-groups, ART, WAL/checkpoint/recovery, parquet/CSV/NPY readers, statistics, vector index | `akar-core/akar-storage/src/` |
| `akar-transaction` | MVCC lifecycle & commit-history visibility | `akar-core/akar-transaction/src/lib.rs` |
| `akar-common` | Arrow vectors/DataChunk, types, selection, memory accounting, VFS, task system | `akar-core/akar-common/src/` |
| `akar-function` | Scalar + aggregate function registry (arithmetic, string, date, list/map, JSON path, hash, …) | `akar-core/akar-function/src/scalar/`, `src/aggregate/`, `src/graph.rs` |
| `akar-vector` / `akar-fts` / `akar-search` | ANN (HNSW + SIMD), full-text (Tantivy), hybrid/RRF/multi-stage search | `akar-core/akar-vector/src/hnsw.rs`, `akar-fts/src/index.rs`, `akar-search/src/` |
| `akar-graph` / `akar-algo` / `akar-ml` / `akar-dream` | GDS (BFS, random walk, node2vec, Louvain, LPA), ML (LSTM/SBYO/sparse), memory consolidation sleep-cycle | `akar-core/akar-graph/src/gds/`, `akar-algo/src/gds/`, `akar-ml/src/`, `akar-dream/src/phases/`, `src/orchestrator.rs` |
| `akar-duckdb` + attach family | DuckDB-delegated read of Delta / Iceberg / Azure Blob / Unity Catalog | `akar-core/akar-duckdb/src/`, `akar-delta/src/`, `akar-iceberg/src/`, `akar-azure/src/`, `akar-unity-catalog/src/` |

## Core Flows

1. **Query (read path):** session statement → parse Cypher → bind to `BoundStatement` → logical plan (+ join order) → optimizer rewrites (WCOJ factorization, predicate/projection/FTS/vector pushdown) → `QueryProcessor` maps to physical operators → parallel vectorized scan/filter/join/aggregate over Arrow chunks → result through the plan-cache-aware session (`akar-main/src/connection/query.rs`).
2. **Write + durability:** DDL/DML bound+executed inside an MVCC transaction → WAL appended before data pages → commit publishes visibility via O(1) commit-history lookup → group-commit + async checkpoint flush dirty pages → crash recovery replays WAL on startup (`akar-storage/src/wal.rs`, `wal_replayer.rs`, `checkpoint.rs`).
3. **Vector ANN query:** `MATCH … WHERE cosine_similarity(n.emb, $q) > thr … ORDER BY cos DESC LIMIT k` is rewritten by `VectorSimilarityDetection` into `[VectorSimilarityScan(HNSW) → Filter(cos>thr) → OrderBy → Projection → Limit]`; `CALL vector_similarity_scan(…)` uses the same HNSW read path with SIMD distance kernels and scalar fallback.
4. **FTS lifecycle:** `CREATE FTS INDEX` builds an on-disk Tantivy index per table/column → inserts/deletes sync **only on durable commit** (aborted writes never leak) → `USING FTS INDEX` predicates push down to the scan leaf; cardinality estimated from Tantivy `doc_freq` (min with table cardinality) without executing search.
5. **Python drop-in path:** `akar-python` mirrors the Kuzu Python API so `sulur` can `cargo`-embed or `pip install akar`; Kuzu-compat harness (53/53 tests) guards behavioral parity (e.g. `MATCH..SET..RETURN` phantom-row fix, P59.1).

## Tech Stack

- **Language:** Rust; deliberately FFI-free in the core engine; SIMD via `#[target_feature]` (`is_x86_feature_detected!` SSE2/AVX, NEON).
- **Query front-end:** Cypher; grammar in **pest** (`cypher.pest`).
- **Columnar runtime:** Apache Arrow vectors / `DataChunk`; column-major on-disk format.
- **Indexes:** ART (+ ART range-scan optimization), Tantivy FTS, self-contained HNSW ANN.
- **External formats:** DuckDB (libduckdb C++ extension) delegated via `akar-duckdb` for Delta/Iceberg/Azure/Unity Catalog; native Parquet/CSV/NPY/JSON readers in `akar-storage`.
- **Concurrency:** worker-thread scheduler, parallel scans, group-commit WAL, MVCC snapshot isolation.
- **Embeddings:** HTTP clients for OpenAI-compatible endpoints (OpenAI, Ollama, VoyageAI, Bedrock, Vertex/Gemini).
- **Build/test/CI:** cargo workspace (35 crates, `max_width=120`), gate `test [akar-core]` (~1,954 tests, 0 failed / 0 ignored; per-crate suites listed), clippy `-D warnings`, GitHub Actions (`rust-ci.yml`, `rust-release.yml`), fuzz targets in `akar-core/fuzz` (nightly). Releases automated via `tools/release.py` (bottom-up publish) + `tools/doc-check.py`.

## System Boundaries

- **Trust boundary — query & ingest input:** arbitrary Cypher, COPY, and JSON/CSV payloads are attacker-adjacent; fuzzed (`cypher_query`, `expression_eval`, `copy_from_csv`, `compression_roundtrip`, `wal_bytes`); malformed-file suites under `dataset/`.
- **External data-plane (read-only attach):** DuckDB-driven Delta Lake, Iceberg, Azure Blob (`CREATE SECRET`), Unity Catalog — credentials flow through attach-setup SQL, never into the core engine.
- **Embedding providers:** outbound HTTP only at query time; no keystore, keys supplied by caller config.
- **Foreign-engine interop:** `akar-neo4j`, `akar-postgres`, `akar-sqlite`, `akar-json` convert external objects across the Arrow/Value type boundary.
- **Server daemon:** blocking TCP server (`akar-server`) with idle-timeout auto-shutdown; session/ping wire protocol now supports parameter binding via the prepared-statement pipeline. Status: deprecated for production (sulur migrates to in-process embedding, P124).
- **On-disk format contract:** `STORAGE_VERSION = 1` (`akar-storage/src/version_info.rs`); a bump without auto-migration ⇒ mandatory `0.2.0` SemVer release.
- **Release/versioning:** crates publish bottom-up respecting crates.io rate limits; tag `v*` triggers 3-OS CLI binary builds + GitHub Release; akar released before `sulur` (cross-repo dependency enforced by `sulur/tools/boundary-check.py`).

## Code Map Index

| Concept | Location |
|---|---|
| Embedded entrypoint / `Database` | `akar-core/akar-main/src/database.rs`, `src/lib.rs` |
| Connection sessions (DDL/DML/COPY/query/transaction) | `akar-core/akar-main/src/connection/{ddl,dml,copy,query,transaction,plan_cache,standalone_call}.rs` |
| Query processor / exec context | `akar-core/akar-processor/src/processor/mod.rs`, `src/physical_operator.rs` |
| Optimizer passes | `akar-core/akar-optimizer/src/passes/{flat,tree}/` |
| WCOJ / factorization | `akar-core/akar-optimizer/src/passes/tree/factorization.rs`, `akar-planner/src/join_order.rs` |
| Vector-similarity HNSW rewrite | `akar-core/akar-optimizer/src/passes/flat/vector_similarity.rs`, `akar-processor/src/physical/write_ops/vectorsimilarityscan.rs` |
| HNSW + distance kernels | `akar-core/akar-vector/src/hnsw.rs`, `src/distance.rs` |
| Storage engine (node-groups, ART, WAL, MVCC) | `akar-core/akar-storage/src/{table,node_group,art_index,wal,wal_replayer,checkpoint,version_info}.rs` |
| MVCC transactions | `akar-core/akar-transaction/src/lib.rs` |
| FTS index + pushdown + estimate | `akar-core/akar-fts/src/index.rs`, `akar-optimizer/src/passes/tree/fts_predicate_pushdown.rs`, `akar-main/src/connection/fts_estimate.rs` |
| GDS / graph algorithms | `akar-core/akar-graph/src/gds/`, `akar-algo/src/gds/` |
| ML models / Dream consolidation | `akar-core/akar-ml/src/`, `akar-dream/src/orchestrator.rs`, `akar-dream/src/phases/` |
| Hybrid / RRF search | `akar-core/akar-search/src/{hybrid,rrf,hierarchical,fused,multi}.rs` |
| Server daemon + wire protocol | `akar-core/akar-server/src/bin/akar_server.rs`, `src/session.rs`, `akar-main/src/remote.rs` |
| Bindings & CLI | `akar-core/akar-cli/src/main.rs`, `akar-python/src/lib.rs`, `akar-c/src/lib.rs`, `akar-wasm/src/lib.rs`, `akar-main/src/adbc.rs` |
| Spec / plan / release governance | root `SPEC.md`, `CHANGELOG.md`, `implementation plan.md`, `tools/release.py`, `tools/doc-check.py` |

Note: full implementation detail (symbols, signatures, code) lives in `agent/repomix.md`.