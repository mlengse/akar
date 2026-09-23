---
type: agent_context
project: akar
title: Agent Architecture Context
source: .
---

## Project Overview

Akar is a **pure-Rust embedded property-graph database for AI-agent memory** (GPL-3.0-or-later, edition 2024, ~139K LOC, 2,242 tests). It is a from-scratch, no-FFI reimplementation of the KuzuDB design: worst-case-optimal joins, factorized execution, column-major storage, and MVCC, compressed into an in-process library with no C++ and no external runtime. Consumers embed it via `akar-main` (Rust), `pip install akar` (Python), or Akar Server `akarshell`/`akar-server`. It executes **openCypher** over a durable, columnar, multi-writer graph store; hot path measured at parity with Kuzu C++ (~397 µs/query). Primary downstream consumer: **Sulur**, the memory engine (formerly kairos), which embeds Akar in-process. Key constraints: embedded-only (no Docker/infra), 100% pure Rust (ADR-002), spec-driven development governed by `SPEC.md`, and cross-product release sequencing with Sulur.

## Architecture

Akar is organized as a layered pipeline within 35 Cargo crates in the `akar-core/` workspace. Data flows verbatim through query frontend → logical planning → cost-based optimizer → vectorized physical execution → columnar storage.

| Layer | Crates | Responsibility |
|---|---|---|
| Frontend | `akar-parser`, `akar-binder`, `akar-catalog` | openCypher grammar (pest), AST, name/type binding, DDL analysis |
| Planning | `akar-planner`, `akar-optimizer` | Logical operators, join-order, flat + tree rule passes (pushdown, fusion, subquery unnest, vector-similarity rewrite) |
| Execution | `akar-processor`, `akar-function`, `akar-common` | Arrow-vectorized operators, scalar/aggregate functions, mapper, spillable join/aggregate |
| Storage | `akar-storage`, `akar-transaction`, `akar-binder` | Column-major tables, ART index, CSR graph storage, WAL/checkpoint, group commit, MVCC undo |
| Search & Graph | `akar-fts`, `akar-vector`, `akar-search`, `akar-algo`, `akar-graph` | Tantivy BM25, HNSW ANN, hybrid/RRF fusion, GDS algorithms (Louvain, node2vec, LPA) |
| Cognitive/AI | `akar-dream`, `akar-ml`, `akar-llm` | Sleep-phase memory consolidation (NREM/REM/AFE/DAE), embedding + LSTM, provider-agnostic LLM calls |
| Entry points | `akar-main`, `akar-cli`, `akar-server`, `akar-python`, `akar-c`, `akar-wasm` | Embedded DB API, shell, network broker (deprecated), FFI/wasm bindings |
| Integration | `akar-duckdb`, `akar-sqlite`, `akar-postgres`, `akar-neo4j`, `akar-azure`, `akar-httpfs`, `akar-iceberg`, `akar-delta`, `akar-unity-catalog`, `akar-json`, `akar-markdown` | External engines, object stores, lakehouse formats |

Major internal dependencies: `akar-main` is the facade (ADBC + connection layer + plan cache); `akar-common` is the shared Arrow data-chunk/enum/error substrate consumed everywhere; `akar-extension` + `akar-azure`/`akar-httpfs` form the pluggable filesystem/extension registry.

## Module Map

| Module | Responsibility | Primary paths |
|---|---|---|
| `akar-parser` | openCypher grammar (pest) → AST | `akar-core/akar-parser/src/parser/`, `cypher.pest` |
| `akar-binder` | Name/type binding, confidential-statement analysis | `akar-core/akar-binder/src/binder/` |
| `akar-planner` | Logical plan + join-order enumeration | `akar-core/akar-planner/src/planner.rs`, `join_order.rs` |
| `akar-optimizer` | Cost-based rule passes (flat/tree) | `akar-core/akar-optimizer/src/passes/` |
| `akar-processor` | Vectorized physical ops, mapper, spills, write ops | `akar-core/akar-processor/src/processor/`, `physical/` |
| `akar-function` | Scalar/aggregate/table-function registry | `akar-core/akar-function/src/scalar/`, `aggregate/` |
| `akar-storage` | Tables, columns, WAL, checkpoints, ART index, CSR | `akar-core/akar-storage/src/` |
| `akar-transaction` | MVCC transaction lifecycle | `akar-core/akar-transaction/src/` |
| `akar-fts` / `akar-search` | BM25 full-text + hybrid/RRF scoring | `akar-core/akar-fts/src/`, `akar-search/src/` |
| `akar-vector` | HNSW ANN + distance kernels | `akar-core/akar-vector/src/hnsw.rs` |
| `akar-graph` / `akar-algo` | Graph algorithms & GDS (Node2Vec, Louvain, RandomWalk) | `akar-core/akar-graph/src/gds/` |
| `akar-dream` / `akar-ml` / `akar-llm` | Memory consolidation, embedding/LSTM, provider embeddings | `akar-core/akar-dream/src/phases/`, `akar-ml/src/` |
| `akar-main` | Embedded DB facade: connection, DDL/DML, COPY, plan cache, ADBC | `akar-core/akar-main/src/` |
| `akar-python` / `akar-c` / `akar-wasm` / `akar-cli` | Bindings & CLI surface | `akar-core/akar-python/src/`, `akar-cli/src/main.rs` |

## Core Flows

1. **Query pipeline** — text → `akar-parser` (pest AST) → `akar-binder` (schema/type resolution) → `akar-planner` (logical plan) → `akar-optimizer` (flat + tree passes; subquery unnesting, filter/join pushdown, vector-similarity rewrite, FTS predicate pushdown) → `akar-processor` mappers build vectorized physical operators → Arrow chunks streamed to `QueryResult`. Plans cached in `akar-main/src/connection/plan_cache.rs`.
2. **Write path (COPY / DML)** — `COPY FROM` CSV/JSON/Parquet/NPY via `attention`-less bulk ingest (`copyfrom.rs`), DML (insert/update/delete/set) converted to physical write ops; write rows go to in-memory node groups, group-committed to the **WAL**, then checkpointed into columnar pages which are compressed and written via shadow-file (atomic swap). MVCC uses undo buffers in `akar-transaction`.
3. **Hybrid retrieval** — query rewrites into `[VectorSimilarityScan(HNSW), Filter(cos), OrderBy, Limit]`; explicit `CALL vector_similarity_scan(...)` path exists. Combined text+vector scoring via BM25 (`akar-fts`, Tantivy) and reciprocal-rank fusion in `akar-search` (hybrid/RRF, hierarchical). Vector indexes maintained on DML via catalog refresh.
4. **Memory consolidation (Dream)** — ingested memories are periodically reprocessed by `akar-dream`'s sleep/rest phases (NREM/REM/AFE/DAE): decay curves (Ebbinghaus), insight extraction, supersede/synthesis into durable graph structure, with local embedding (Candle) in `akar-ml`.

## Tech Stack

- **Language/edition:** Rust 2024, Cargo workspace of 35 crates; standard CI clippy `-Dwarnings`, `cargo fmt` (max_width 120).
- **Parsing:** `pest` grammar (`cypher.pest`), AST in `akar-parser` (ADR-001).
- **Execution:** Arrow-vectorized `DataChunk`/`SelectionVector` in `akar-common`; physical operator tree in `akar-processor`; spill-to-disk for hash join/aggregate (radix/block-merge sort).
- **Storage:** column-major (ADR-004), page manager, WAL + replayer, group commit, ART index, CSR for graph, string dictionary, compression (zstd/gzip via `akar-httpfs`/gzip FS).
- **Indexes/search:** Tantivy (BM25) in `akar-fts`; custom HNSW in `akar-vector`; hybrid RRF in `akar-search`.
- **AI/ML:** local embedding via Candle (`akar-ml`), provider embeddings via `akar-llm`, LSTM, SBYO, sparse embeddings.
- **Bindings:** PyO3 Python package (`pip install akar`), C library, Wasm target, CLI.
- **Infra/tooling:** GitHub Actions (`rust-ci.yml`, `rust-release.yml`), ADR docs (`akar-core/docs/adr/`), fuzz targets (nightly toolchain), `tools/release.py` + `tools/doc-check.py`.

## System Boundaries

| Boundary | Interface | Direction |
|---|---|---|
| External LLM providers | OpenAI, Google Gemini/Vertex, AWS Bedrock, Ollama, Voyage AI (via `akar-llm`) | outbound HTTP |
| Object storage | Azure Blob (`akar-azure`), HTTP/S3-style (`akar-httpfs`) | outbound HTTP |
| External engines | DuckDB, SQLite, PostgreSQL, Neo4j connectors (binary/network libs) | integration |
| Lakehouse formats | Iceberg, Delta, Unity Catalog (Apache Avro reader) | integration |
| Consumer API | `akar-python` (PyPI) consumed by **Sulur**; sulur/akar boundary enforced by `sulur/tools/boundary-check.py` | cross-repo contract |
| Network broker | `akar-server` (TCP JSON) — **deprecated** for production; degraded to test harness / wire reference | outbound→inbound |
| Trust boundaries | Untrusted cypher input & host SQL/JSON path reads; local file IO confined to catalog paths; network outbound only for provider/storage access | — |

## Code Map Index

| Concept | Location | Notes |
|---|---|---|
| Embedded DB facade | `akar-core/akar-main/src/database.rs`, `lib.rs` | Entry, pool, prepared statements |
| Connection layer (query/DDL/DML/COPY/plan-cache) | `akar-core/akar-main/src/connection/` | Includes transaction, substitute, standalone_call |
| Copy ingestion (CSV/JSON/Parquet/NPY) | `akar-core/akar-main/src/bulk.rs`, `akar-storage/src/csv_reader.rs` | Dialect sniffing, multi-file |
| Optimizer passes | `akar-core/akar-optimizer/src/passes/` | Flat + tree pass dirs |
| Physical operators | `akar-core/akar-processor/src/physical/` | scan_filter, order_aggregate, write_ops, join |
| Vector similarity scan path | `akar-core/akar-optimizer/src/passes/flat/vector_similarity.rs`, `akar-processor/src/physical/write_ops/vectorsimilarityscan.rs` | Docs in SPEC §8/vector notes |
| Storage & WAL | `akar-core/akar-storage/src/wal.rs`, `wal_replayer.rs`, `checkpoint.rs`, `uart` | Group commit, crash recovery |
| FTS build/catch-up | `akar-core/akar-fts/src/build.rs`, `index.rs`; `akar-main/src/connection/fts_estimate.rs` | Tantivy-backed |
| Hybrid search fusion | `akar-core/akar-search/src/hybrid.rs`, `rrf.rs`, `fused.rs`, `hierarchical.rs` | BM25+vector RRF |
| HNSW | `akar-core/akar-vector/src/hnsw.rs` | ONNX-free, owned engine |
| Dream (memory consolidation) | `akar-core/akar-dream/src/phases/` | NREM/REM/AFE/DAE/insights/supersedes |
| Graph algorithms | `akar-core/akar-algo/src/gds/`, `akar-graph/src/gds/` | Louvain, LPA, Node2Vec |
| Python bindings | `akar-core/akar-python/src/` | dream, knn, louvain, lstm, search, spread |
| CLI & server | `akar-core/akar-cli/src/main.rs`, `akar-server/src/bin/akar_server.rs` | shell + deprecated broker |
| Release tooling | `tools/release.py`, `tools/doc-check.py`, root `CHANGELOG.md`, `SPEC.md` | Spec-driven flow gate |