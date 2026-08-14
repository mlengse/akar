# Akar — Repository Specification

> **Akar** — Pure Rust embedded graph database for AI agent memory.
> **Author:** Anjang Kusuma Netra | **License:** GPLv3 | **Edition:** Rust 2024
> **Repository:** `https://github.com/mlengse/akar`

---

## 1. Project Overview

Akar is a **from-scratch pure Rust reimplementation** of [KuzuDB](https://github.com/kuzudb/kuzu), an embedded property graph database optimized for complex analytical workloads. Originally forked from the Vela-Engineering/Kuzu project, Akar is now a **standalone pure Rust codebase** — zero C++ dependencies, zero FFI.

### 1.1 Design Goals

| Goal | Description |
|------|-------------|
| **AI Agent Memory** | Multi-hop graph traversal (`Founder → Company → Round → Outcome`) as in-process embedded DB — zero infrastructure |
| **Pure Rust** | No C++ deps, no FFI, memory-safe by default |
| **Performance Parity** | Verified 3-way parity with C++ implementations (Rust 397 µs ≈ Kuzu C++ 400 µs ≈ LadybugDB C++ 374 µs on 10K rows) |
| **Concurrent Multi-Writer** | OCC row-level conflict detection for multi-agent architectures |
| **Cross-Platform** | Linux, macOS, Windows, WebAssembly |

### 1.2 Key Metrics

| Metric | Value |
|--------|-------|
| Workspace crates | **32** |
| Lines of code | **~86K LOC** (pure Rust, git-tracked incl. tests) |
| Tests passing | **1,649 total, 5 ignored, 1,644 passed, 0 failed** (gate `test [akar-core]` 2026-08-14, P53.x: binder deadlock + flake FTS DashMap; sebelumnya 1,647 @ `340dbd0`) |
| Optimizer passes | **24** (18 flat + 6 tree) — exceeds C++ (17) |
| Registered functions | **259** (244 scalar + 14 aggregate + 1 table) |
| Logical operators | **58** variants |
| Physical operators | **48** structs |
| Extensions | **15** crates |
| Graph algorithms | **15** |

---

## 2. Repository Structure

```
akar/
├── .github/
│   ├── workflows/
│   │   ├── rust-ci.yml          # 12-job CI (3 OS + wasm + fuzz + coverage)
│   │   ├── rust-release.yml     # Release workflow (3-platform binaries)
│   │   ├── bench-ci.yml         # Benchmark CI
│   │   └── fuzz-ci.yml          # Fuzz testing CI
│   ├── ISSUE_TEMPLATE/
│   ├── CODEOWNERS
│   ├── dependabot.yml
│   └── pull_request_template.md
│
├── akar-core/                   # ★ Main Rust workspace (32 crates)
│   ├── Cargo.toml               # Workspace root
│   ├── Cargo.lock
│   ├── clippy.toml
│   ├── rustfmt.toml
│   │
│   ├── akar-common/             # Type system, Value, DataChunk, memory
│   ├── akar-storage/            # Columnar storage, WAL, buffer manager
│   ├── akar-transaction/        # MVCC, OCC, checkpoint
│   ├── akar-catalog/            # System catalog (schemas, tables, types)
│   ├── akar-parser/             # PEG grammar (pest.rs) for Cypher
│   ├── akar-binder/             # Semantic analysis, symbol resolution
│   ├── akar-planner/            # Logical plan construction
│   ├── akar-optimizer/          # 24 optimization passes
│   ├── akar-processor/          # Physical operator execution
│   ├── akar-function/           # 259 built-in functions
│   ├── akar-graph/              # CSR adjacency, GDS framework
│   ├── akar-extension/          # Extension framework trait + registry
│   │
│   ├── akar-json/               # JSON extension
│   ├── akar-fts/                # Full-Text Search (BM25)
│   ├── akar-vector/             # Vector similarity search
│   ├── akar-algo/               # 15 graph algorithms
│   ├── akar-httpfs/             # HTTP/HTTPS/S3 file system
│   ├── akar-duckdb/             # DuckDB integration
│   ├── akar-sqlite/             # SQLite integration (rusqlite)
│   ├── akar-postgres/           # PostgreSQL integration (tokio-postgres)
│   ├── akar-neo4j/              # Neo4j integration (bolt protocol)
│   ├── akar-delta/              # Delta Lake integration
│   ├── akar-iceberg/            # Apache Iceberg integration
│   ├── akar-azure/              # Azure Blob Storage integration
│   ├── akar-unity-catalog/      # Unity Catalog integration
│   ├── akar-llm/                # LLM function integration
│   │
│   ├── akar-main/               # ★ Public API: Database, Connection, QueryResult
│   ├── akar-cli/                # Interactive Cypher REPL shell
│   ├── akar-wasm/               # WebAssembly bindings
│   ├── akar-c/                  # C FFI API (extern "C")
│   ├── akar-server/             # Embedded TCP server mode
│   └── akar-migrate/            # C++ → Rust migration tool
│
├── dataset/                     # 68 test datasets (CSV, Parquet, JSON)
├── examples/
│   └── rust/                    # Rust API examples
├── tools/
│   └── rust_api/                # Native Rust API crate (thin wrapper)
│
├── README.md
├── LICENSE                      # GPLv3
├── CONTRIBUTING.md
├── CODE_OF_CONDUCT.md
├── CLA.md                       # Contributor License Agreement
├── MIGRATION.md                 # C++ → Rust migration guide
├── init.cypher                  # Sample Cypher init script
└── .gitignore
```

---

## 3. Architecture

### 3.1 Query Pipeline

```
Cypher text
    │
    ▼
┌─────────────┐    ┌──────────┐    ┌──────────┐    ┌──────────────┐    ┌──────────────┐
│   Parser    │───▶│  Binder  │───▶│  Planner │───▶│  Optimizer   │───▶│  Processor   │
│ (pest.rs)   │    │(Catalog) │    │(logical) │    │ (24 passes)  │    │ (physical)   │
│ 33 stmt     │    │33 bound  │    │58 ops    │    │ 18 flat +    │    │ 48 operators │
│ variants    │    │variants  │    │          │    │ 6 tree       │    │              │
└─────────────┘    └──────────┘    └──────────┘    └──────────────┘    └──────────────┘
                                                                             │
                                                                             ▼
                                                                       DataChunks
                                                                       (Arrow arrays)
```

### 3.2 Crate Dependency Graph

```
akar-common
  → akar-storage, akar-transaction, akar-function, akar-parser
    → akar-catalog
      → akar-binder
        → akar-planner
          → akar-optimizer
            → akar-processor
              → akar-graph
                → akar-main
                  → akar-cli, akar-wasm, akar-c, akar-server
```

Extension crates (`akar-json`, `akar-fts`, `akar-algo`, etc.) depend on `akar-common` /
`akar-extension` and are feature-gated in `akar-main`.

### 3.3 Core Subsystem Details

#### Parser ([akar-parser](akar-core/akar-parser))
- **Engine:** `pest.rs` PEG (replaces ANTLR4 C++)
- **Grammar:** `cypher.pest` — modular, composable rules
- **AST:** 33 Statement variants (DDL, DML, Transaction, Extension, Attach/Detach/Use DB) + 10 Clause sub-variants
- **Parity:** ~95%

#### Binder ([akar-binder](akar-core/akar-binder))
- Symbol resolution via `Arc<Mutex<Catalog>>`
- 33 BoundStatement variants
- Property type resolution via catalog (not hardcoded)
- **Parity:** ~90%

#### Planner ([akar-planner](akar-core/akar-planner))
- 58 LogicalOperator variants (ScanNode, ScanRel, HashJoin, CrossProduct, TopK, Intersect, SemiJoin, RecursiveExtend, +12 DDL operators)
- **Parity:** ~90%

#### Optimizer ([akar-optimizer](akar-core/akar-optimizer))
**18 Flat Passes:**

| # | Pass | Description |
|---|------|-------------|
| 1 | RemoveUnnecessaryOperators | Eliminates redundant operators |
| 2 | FilterPushDown | Pushes filters closer to scans |
| 3 | PredicatePushDown | Pushes predicates below joins |
| 4 | ProjectionPushDown | Eliminates unused columns early |
| 5 | ConstantFolding | Evaluates constants at plan time |
| 6 | AggregateDetection | Detects aggregation boundaries |
| 7 | JoinOptimization | Cardinality-aware join reordering |
| 8 | TopKOptimization | Converts OrderBy + Limit to TopK |
| 9 | VectorSimilarityDetection | **NO-OP (audit P52.5)** — old rewrite dropped the distance threshold and consumed Projection/OrderBy/Limit; also unreachable after FilterPushDown folds the predicate into the ScanNode |
| 10 | ArtRangeScanDetection | Detects ART index range scan patterns — **fixed conservatively (P52.4)**: rewrites only when the WHOLE filter is bounds on one property; never merges different-column conjuncts or drops predicates |
| 11 | LimitPushDown | Pushes limits closer to scans |
| 12 | CommonSubexpressionElimination | **NO-OP (audit P52.2)** — dedup changed the positional RETURN arity; the mapping was never applied to downstream consumers |
| 13 | OrderByPushDown | Pushes ORDER BY below UNION ALL — **NO-OP (audit P52.6)**: per-branch sort is not a global sort under UNION concat; the old rewrite dropped the global ORDER BY |
| 14 | UnwindDedup | Deduplicates consecutive UNWIND |
| 15 | CountRelTable | Replaces ScanRel+COUNT with CSR metadata |
| 16 | AggregateFusion | Fuses aggregate operations — **NO-OP (audit P52.7)**: fusion resolved the outer agg's args to NULL and changed COUNT(*) from groups to raw rows |
| 17 | SortElision | Eliminates redundant sorts |
| 18 | ExpressionInline | Inlines trivial expressions |

**6 Tree Passes:**

| # | Pass | Description |
|---|------|-------------|
| 1 | FactorizationRewriting | Inserts Flatten operators for hash joins |
| 2 | ForeignJoinPushDown | Pushes foreign joins through operators |
| 3 | AccHashJoinOptimization | Optimizes accumulated hash joins |
| 4 | CorrelatedSubqueryUnnesting | Unnests correlated subqueries |
| 5 | AggKeyDependency | Removes redundant grouping keys |
| 6 | CardinalityEstimation | Annotates with estimated row counts |

**Parity:** ~95% (exceeds C++ with 17 passes)

#### Processor ([akar-processor](akar-core/akar-processor))
- 48 physical operator structs (no single enum; DDL ops wired via mapper)
- Arrow-native expression evaluation (`evaluate_to_arrow` + `boolean_array_to_selection`)
- Parallel aggregation via `AggregateHashTable`
- Parallel hash join via `JoinHashTable`
- `BlockMergeSort` + `RadixSort` for ORDER BY
- `BinaryHeap` O(n log k) TopK
- **Parity:** ~90% essential, ~66% total count (split-phase accounting)

#### Storage ([akar-storage](akar-core/akar-storage))

| Component | Description |
|-----------|-------------|
| Buffer Manager | Clock eviction + mmap + NUMA + readahead |
| Page Manager | Page allocation/deallocation via buddy-system FSM |
| WAL + Replayer | Append-only (52× speedup), CRC32 per record, crash recovery |
| Undo Buffer | Rollback safety via undo record replay |
| NodeTable / RelTable | Columnar with CSR fwd/rev adjacency arrays |
| ART Index | Node4/16/48/256 adaptive radix tree |
| HNSW Index | Vector similarity search index |
| Hash Index | On-disk + in-memory |
| Compression | Constant, Boolean, StringDictionary encoding |
| Overflow pages | `.ovf` sidecar for oversized values |
| CSV/Parquet readers | Native readers with Arrow type mapping |

#### Transaction ([akar-transaction](akar-core/akar-transaction))
- MVCC with AUTO/MANUAL modes
- OCC row-level conflict detection (`RowConflictTracker`)
- Concurrent multi-writer via `AtomicBool` + `Condvar`
- Checkpoint worker
- Serializable ACID transactions

---

## 4. Supported Cypher Clauses

| Category | Clauses |
|----------|---------|
| **DDL** | `CREATE NODE TABLE`, `CREATE REL TABLE`, `DROP TABLE`, `ALTER TABLE` (ADD/DROP/RENAME COLUMN, RENAME TABLE), `CREATE SEQUENCE`, `DROP SEQUENCE`, `CREATE MACRO` |
| **DML** | `MATCH`, `WHERE`, `RETURN`, `ORDER BY`, `LIMIT`, `SKIP`, `DELETE`, `SET`, `CREATE` (node/rel), `MERGE` (ON CREATE/ON MATCH SET) |
| **Composition** | `WITH`, `UNION ALL`, `UNWIND`, `OPTIONAL MATCH`, `FOREACH` |
| **Data Loading** | `COPY FROM` (CSV, Parquet), `COPY TO` (CSV, Parquet), `EXPORT DATABASE`, `IMPORT DATABASE` |
| **Transaction** | `BEGIN TRANSACTION`, `COMMIT`, `ROLLBACK` |
| **Introspection** | `CALL show_tables()`, `table_info()`, `show_functions()`, `show_indexes()`, `db_version()`, `storage_info()`, `ANALYZE`, `EXPLAIN` |
| **Patterns** | Variable-length paths `[*1..5]`, edge patterns, multi-hop traversals |
| **Expressions** | Arithmetic, boolean, string, CASE, list/map/struct literals, subqueries, parameters |

---

## 5. Type System

### 5.1 Logical Types (37)

Scalar: `BOOL`, `INT8`, `INT16`, `INT32`, `INT64`, `INT128`, `UINT8`, `UINT16`, `UINT32`, `UINT64`, `FLOAT`, `DOUBLE`, `STRING`, `BLOB`, `UUID`, `SERIAL`, `DATE`, `TIMESTAMP`, `TIMESTAMP_NS`, `TIMESTAMP_MS`, `TIMESTAMP_S`, `TIMESTAMP_TZ`, `INTERVAL`

Composite: `LIST`, `ARRAY`, `MAP`, `STRUCT`, `UNION`, `NODE`, `REL`, `RECURSIVE_REL`, `INTERNAL_ID`

Special: `ANY`, `NULL`, `RDF_VARIANT`

### 5.2 Physical Types (19)

`ANY`, `BOOL`, `INT8`, `INT16`, `INT32`, `INT64`, `INT128`, `UINT8`, `UINT16`, `UINT32`, `UINT64`, `FLOAT`, `DOUBLE`, `STRING`, `BLOB`, `LIST`, `ARRAY`, `STRUCT`, `INTERVAL`

---

## 6. Functions (259 Registered)

> Registry count (verified via `FunctionRegistry::new()`): **244 scalar + 14 aggregate + 1
> table** = 259. `CALL show_tables()/db_version()/storage_info()` are handled as
> `BoundStandaloneCall` in the DDL layer, not registered table functions.

| Category | Count | Examples |
|----------|:-----:|---------|
| Arithmetic | 28 | `+`, `-`, `*`, `/`, `%`, `abs`, `ceil`, `floor`, `round`, `power`, `sqrt` |
| Comparison | 8 | `=`, `<>`, `<`, `>`, `<=`, `>=` |
| Boolean | 4 | `AND`, `OR`, `NOT`, `XOR` |
| String | 25 | `upper`, `lower`, `trim`, `substring`, `concat`, `contains`, `starts_with` |
| Date/Time | 16 | `date_part`, `date_trunc`, `now`, `age`, `make_date` |
| Cast | 14+ | Type casting functions |
| List | 14 | `list_extract`, `list_concat`, `list_contains`, `range` |
| Map | 5 | `map_extract`, `map_keys`, `map_values` |
| Struct | 2 | `struct_extract`, `struct_pack` |
| Array | 5 | Array operations |
| Path | 6 | `nodes`, `rels`, `properties`, `length` |
| Aggregate | 14 | `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`, `COLLECT`, `PERCENTILE_DISC`, `PERCENTILE_CONT` |
| Table (CALL) | 22 | `show_tables()`, `table_info()`, `db_version()`, `storage_info()` |
| Schema/Utility | 17 | Various system functions |

---

## 7. Extension Ecosystem (15 Crates)

| Extension | Type | Crate | Description |
|-----------|------|-------|-------------|
| JSON | Native Rust | [akar-json](akar-core/akar-json) | `json_extract`, `json_valid`, `json_type`, `json_structure`, `json_contains` |
| Full-Text Search | Native Rust | [akar-fts](akar-core/akar-fts) | Stemmer, tokenizer, TF-IDF, BM25, stop words |
| Vector | Native Rust | [akar-vector](akar-core/akar-vector) | HNSW index, vector similarity search |
| Graph Algorithms | Native Rust | [akar-algo](akar-core/akar-algo) | PageRank, WCC, SCC, K-Core, Louvain, spanning forest |
| HTTP/S3 | Native Rust | [akar-httpfs](akar-core/akar-httpfs) | HTTP/HTTPS/S3 file reads |
| DuckDB | Rust crate | [akar-duckdb](akar-core/akar-duckdb) | `duckdb_query`, `duckdb_scan` |
| SQLite | Native rusqlite | [akar-sqlite](akar-core/akar-sqlite) | `sqlite_query`, `sqlite_scan` |
| PostgreSQL | tokio-postgres | [akar-postgres](akar-core/akar-postgres) | `sql_query` |
| Neo4j | Native Rust | [akar-neo4j](akar-core/akar-neo4j) | Bolt protocol integration |
| LLM | Native Rust | [akar-llm](akar-core/akar-llm) | LLM function integration |
| Delta Lake | DuckDB delegation | [akar-delta](akar-core/akar-delta) | `delta_scan` |
| Iceberg | DuckDB delegation | [akar-iceberg](akar-core/akar-iceberg) | `iceberg_scan`, `iceberg_metadata` |
| Azure | DuckDB delegation | [akar-azure](akar-core/akar-azure) | `azure_scan` (abfss:// URI) |
| Unity Catalog | DuckDB delegation | [akar-unity-catalog](akar-core/akar-unity-catalog) | `uc_scan` |
| Server | Native Rust | [akar-server](akar-core/akar-server) | TCP listener + JSON framing |

Extensions are compiled statically via Cargo feature flags:

```toml
[dependencies]
akar-main = { git = "...", features = ["json-extension", "fts-extension", "vector-extension"] }
```

---

## 8. Graph Data Science (GDS) Framework

15 algorithms implemented in [akar-graph](akar-core/akar-graph) + [akar-algo](akar-core/akar-algo):

| Algorithm | Description |
|-----------|-------------|
| BFS | Breadth-first search (Dense/Sparse frontiers) |
| Dijkstra | Single-source weighted shortest path |
| All Shortest Paths | All shortest paths between nodes |
| All Weighted Shortest Paths | All weighted shortest paths |
| PageRank | Iterative PageRank computation |
| WCC | Weakly Connected Components |
| SCC (Tarjan) | Strongly Connected Components |
| SCC (Kosaraju) | Strongly Connected Components (alternate) |
| K-Core Decomposition | K-core decomposition |
| Louvain | Community detection |
| Spanning Forest | Minimum spanning forest |
| LPA | Label Propagation |
| Betweenness Centrality | Node betweenness centrality |
| Closeness Centrality | Node closeness centrality |
| Triangle Counting | Count triangles in graph |

**Parity:** ~100%

---

## 9. Build System

### 9.1 Workspace Configuration

[Cargo.toml](akar-core/Cargo.toml):

```toml
[workspace]
resolver = "2"
members = [
    "akar-common", "akar-storage", "akar-transaction", "akar-catalog",
    "akar-parser", "akar-binder", "akar-planner", "akar-optimizer",
    "akar-processor", "akar-function", "akar-graph", "akar-extension",
    "akar-json", "akar-fts", "akar-vector", "akar-httpfs", "akar-duckdb",
    "akar-algo", "akar-neo4j", "akar-llm", "akar-sqlite", "akar-delta",
    "akar-iceberg", "akar-azure", "akar-postgres", "akar-unity-catalog",
    "akar-main", "akar-cli", "akar-wasm", "akar-migrate", "akar-c",
    "akar-server",
]

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "GPL-3.0-or-later"
```

### 9.2 Key Dependencies

| Dependency | Version | Purpose |
|------------|---------|---------|
| `arrow` | 59.1.0 | Columnar in-memory format |
| `parquet` | 59.1.0 | Parquet file reader/writer |
| `serde` / `serde_json` | 1.x | Serialization |
| `rayon` | 1.x | Data parallelism |
| `hashbrown` | 0.17.1 | High-perf hash maps |
| `ahash` | 0.8 | Fast hashing |
| `pest` (via parser) | — | PEG grammar parser |
| `criterion` | 0.8.2 | Benchmarking framework |
| `thiserror` | 2.x | Error derive macros |
| `tracing` | 0.1 | Structured logging |

### 9.3 Build Commands

```bash
# Full workspace
cargo build --release --workspace

# Run CLI
cargo run --bin akar-cli -- /path/to/db

# With all extensions
cargo build --release -p akar-main \
  --features json-extension,fts-extension,vector-extension,algo-extension

# WASM target
rustup target add wasm32-unknown-unknown
cargo check --target wasm32-unknown-unknown --workspace

# Windows native (MinGW)
cargo build --target x86_64-pc-windows-gnu
```

### 9.4 Release Profiles

```toml
[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
panic = "abort"
strip = true

[profile.release-debug]
inherits = "release"
opt-level = 3
debug = true
strip = false
```

---

## 10. CI/CD

### 10.1 Workflows

| Workflow | File | Jobs |
|----------|------|------|
| **Rust CI** | [rust-ci.yml](.github/workflows/rust-ci.yml) | 12 jobs (see below) |
| **Rust Release** | [rust-release.yml](.github/workflows/rust-release.yml) | Build + publish GitHub Release |
| **Bench CI** | [bench-ci.yml](.github/workflows/bench-ci.yml) | Benchmark compilation check |
| **Fuzz CI** | [fuzz-ci.yml](.github/workflows/fuzz-ci.yml) | Fuzz testing |

### 10.2 Rust CI Jobs (12)

| # | Job | Runner | Description |
|---|-----|--------|-------------|
| 1 | `fmt` | ubuntu-24.04 | `cargo fmt --all -- --check` |
| 2 | `audit` | ubuntu-24.04 | `cargo audit` (security vulnerability check) |
| 3 | `clippy` | ubuntu-24.04 | `cargo clippy --workspace --all-targets -- -D warnings` |
| 4 | `test-ubuntu` | ubuntu-24.04 | `cargo build + cargo test --workspace` |
| 5 | `test-macos` | macos-14 | `cargo build + cargo test --workspace` |
| 6 | `test-windows` | windows-2022 | `cargo build + cargo test --workspace` |
| 7 | `feature-gated` | ubuntu-24.04 | Build + test with ALL 15 extension features |
| 8 | `wasm-check` | ubuntu-24.04 | `cargo check --target wasm32-unknown-unknown` |
| 9 | `bench-check` | ubuntu-24.04 | `cargo bench --workspace --no-run` |
| 10 | `coverage` | ubuntu-24.04 | `cargo-tarpaulin` → Codecov upload |
| 11 | `wasm-test` | ubuntu-24.04 | `wasm-pack test --node akar-wasm` |
| 12 | `rust-api` | ubuntu-24.04 | Build native `tools/rust_api` crate |

### 10.3 Release Pipeline

Triggered by pushing a version tag (`v*`):

1. Run full test suite on Ubuntu
2. Build CLI binaries for **Linux (x86_64)**, **macOS (arm64)**, **Windows (x86_64)**
3. Create GitHub Release with auto-generated changelog
4. Attach CLI binaries as release assets

> [!NOTE]
> crates.io publishing is **active** (since 2026-08-08, P50). All 31 publishable crates are
> published bottom-up (verified live on crates.io; latest release v0.1.5, 2026-08-11);
> GitHub Releases with prebuilt CLI binaries are produced as well.

---

## 11. Testing

### 11.1 Test Suite Summary

| Crate | Tests | Coverage Focus |
|-------|------:|----------------|
| `akar-common` | 24 | Types (37 LogicalTypes, Value), Vectors, Memory |
| `akar-parser` | 70 | PEG grammar, 33 Statement variants, operator precedence |
| `akar-binder` | 87 | Semantic analysis, type inference, symbol resolution |
| `akar-planner` | 21 | Logical plan construction |
| `akar-optimizer` | 68 | 24 optimization passes (audit P52.2–P52.7: 5 passes reviewed 2026-08-10, ART range scan fixed + 4 documented NO-OPs, +12 regression tests) |
| `akar-processor` | 142 | Physical operators (Scan, Filter, HashJoin, OrderBy, Aggregate, etc.) |
| `akar-function` | 176 | 259 registered functions |
| `akar-storage` | 341 | BufferManager, WAL, Compression, CSV/Parquet readers, ART Index |
| `akar-main` (unit) | 70 | Database, Connection, QueryResult, DDL/DML, COPY FROM |
| `akar-main` (integration) | 297 | RETURN *, FOREACH, MERGE, subqueries, WCOJ, crash recovery, durability |
| `akar-catalog` | 39 | Catalog CRUD, schema management |
| `akar-transaction` | 18 | MVCC, begin/commit/rollback, checkpoint, conflict detection |
| `akar-graph` | 34 | CSR adjacency, all GDS algorithms |
| `akar-vector` | 22 | Vector similarity search |
| `akar-json` | 12 | JSON functions |
| `akar-fts` | 14 | Stemmer, Tokenizer, BM25 |
| `akar-algo` | 34 | Graph algorithm extensions |
| `akar-extension` | 15 | Extension framework registry |
| `akar-c` (FFI) | 14 | `extern "C"` binding tests |
| `akar-server` | 12 | TCP framing, session, concurrency |
| `akar-postgres` | 7 | PostgreSQL integration |
| `akar-duckdb` | 9 | DuckDB integration |
| `akar-httpfs` | 7 | HTTP/S3 file reads |
| `akar-neo4j` | 12 | Bolt protocol |
| `akar-llm` | 9 | LLM functions |
| `akar-sqlite` / `akar-azure` / `akar-delta` / `akar-iceberg` / `akar-unity-catalog` | 5 | Integration extensions (1 each) |
| `akar-wasm` | 0* | WASM bindings (*3 via `wasm-pack test --node` on CI) |
| `akar-migrate` | 1 | Migration tool (idempotent, fixed P48.5) |
| Doc-tests | 8 (5 ignored) | Doc-tests across all crates |
| **Total** | **1,649** | **1,649 total, 5 ignored, 1,644 passed, 0 failed** (gate `test [akar-core]` 2026-08-14, P53.x: binder deadlock + flake FTS DashMap; sebelumnya 1,647 @ `340dbd0`) |

### 11.2 Test Datasets

68 test dataset directories under [dataset/](dataset) including:
- `tinysnb` — core small-dataset tests
- `ldbc-sf01` / `lsqb-sf01` — LDBC Social Network Benchmark
- `fts-*` — Full-text search datasets
- `csv-*` — CSV edge cases and error handling
- `copy-*` — COPY FROM/TO tests
- `parquet` — Parquet format tests

### 11.3 Running Tests

```bash
# Full workspace
cargo test --workspace --no-fail-fast

# Specific crate
cargo test -p akar-parser
cargo test -p akar-main

# With all features
cargo test -p akar-main --features json-extension,fts-extension,algo-extension
```

---

## 12. Benchmarks

### 12.1 Performance Results

**Full pipeline (10K rows):**

| Query | Time |
|-------|------|
| `MATCH (p) WHERE p.age > 30 RETURN COUNT(p)` | **397 µs** |
| Scan 10K rows (4 columns) | **1,050 µs** |
| Scan 10K selective (2/4 cols) | **168 µs** |

**Scaling benchmarks:**

| Scale | Scan | Filter | COUNT | Filter+COUNT |
|-------|------|--------|-------|-------------|
| 10K rows | 3.0 ms | 2.9 ms | 2.8 ms | 3.2 ms |
| 100K rows | 23.4 ms | 22.7 ms | 23.8 ms | 23.7 ms |
| 1M rows | 222 ms | 235 ms | 212 ms | 237 ms |

**3-way C++ parity:**

| Implementation | Time | Query |
|----------------|------|-------|
| **Akar (Rust)** | 397 µs | `MATCH (p) WHERE p.age > 30 RETURN COUNT(p)` |
| **Kuzu C++ (Vela)** | 400 µs | Same query, same dataset |
| **LadybugDB C++** | 374 µs | Same query, same dataset |

### 12.2 Benchmark Infrastructure

| Area | Tool | Crate |
|------|------|-------|
| Full pipeline | criterion | `akar-main` |
| Operator micro-benchmarks | criterion | `akar-processor` |
| Storage benchmarks | criterion | `akar-main` |
| Scale tests (10K/100K/1M) | criterion | `akar-main` |

```bash
# Run benchmarks
cargo bench -p akar-main                          # Full pipeline + storage
cargo bench -p akar-processor                     # Operator micro-benchmarks
cargo bench --workspace                           # All benchmarks

# Scale benchmarks
cargo bench -p akar-main --bench ladybug_suite -- "ladybug_100k"  # 100K rows
cargo bench -p akar-main --bench ladybug_suite -- "ladybug_1m"    # 1M rows
```

---

## 13. Public API

### 13.1 Core API ([akar-main](akar-core/akar-main))

```rust
use std::sync::Arc;
use akar_main::{Database, Connection, SystemConfig};

// Create database
let config = SystemConfig::default();
let db = Arc::new(Database::new("/path/to/db", config)?);

// Create connection
let conn = Connection::new(&db);

// Execute queries
conn.query("CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name))")?;
conn.query("COPY Person FROM 'data.csv' (HEADER true)")?;
let result = conn.query("MATCH (p:Person) WHERE p.age > 25 RETURN p.name ORDER BY p.age")?;

// Prepared statements
let stmt = conn.prepare("MATCH (a:Person {name: $name}) RETURN a.age")?;
let result = conn.execute(&stmt, vec![("name", "Alice".into())])?;

// Manual transactions
conn.query("BEGIN TRANSACTION")?;
conn.query("CREATE (:Person {name: 'Bob', age: 30})")?;
conn.query("COMMIT")?;
```

### 13.2 C FFI ([akar-c](akar-core/akar-c))
- `extern "C"` API for language bindings

### 13.3 WebAssembly ([akar-wasm](akar-core/akar-wasm))
- `AkarDatabase`, `AkarConnection`, `AkarPreparedStatement` wrappers for Node.js

### 13.4 TCP Server ([akar-server](akar-core/akar-server))
- Length-prefixed JSON framing over TCP
- Session bridging via `TransactionManager`
- Supports concurrent read/write clients

---

## 14. Error Handling

Unified error type hierarchy defined in [akar-common](akar-core/akar-common):

```
AkarError
├── StorageError     (12 variants)
├── TransactionError (6 variants)
├── CatalogError     (5 variants)
├── BinderError      (6 variants)
├── PlannerError     (1 variant)
└── ProcessorError   (3 variants)
```

All crates use `Result<T, E>` with `?` propagation. No `panic!()` or `.unwrap()` in
production code paths (replaced with `ok_or_else()`, epsilon float comparisons, etc.).

---

## 15. Concurrency & Safety

| Feature | Implementation |
|---------|---------------|
| Multi-writer | `AtomicBool` + `Condvar` + OCC row-level conflict detection |
| Transaction isolation | MVCC with AUTO/MANUAL modes |
| Conflict detection | `RowConflictTracker`, `written_rows`, `validate_write_set` |
| Lock poisoning | `.lock().map_err()` across 17 files (75 calls migrated) |
| File locking | Exclusive/shared file locks for multi-process safety |
| WAL safety | Atomic rename (`write .tmp → sync → rename → fsync parent`) |

---

## 16. Versioning & Release

- **Current version:** `0.1.3` (highest published crate patch); latest release tag **v0.1.5** (2026-08-11)
- **Versioning:** [Semantic Versioning 2.0.0](https://semver.org/)
- **MSRV:** Rust 1.80+
- **Release artifacts:** CLI binaries for Linux (x86_64), macOS (arm64), Windows (x86_64)
- **crates.io:** **superseded 2026-08-08 (P50)** — publishing active since 2026-08-08; see §10.3

---

## 17. Design Decisions Log

| # | Decision | Rationale |
|---|----------|-----------|
| #11 | crates.io publishing deferred | API not yet stable for public consumption — **superseded 2026-08-08 (P50): publishing active**, 31/31 crates at 0.1.0 |
| #66 | No premature production publish | Don't publish before truly production-ready — satisfied by P50 gate (1,594 tests, audits CLEAN) |
| #67 | WCOJ benchmark deferred | Legacy bench never runnable; pre-existing bugs |

---

## 18. Known Limitations

1. **No direct Neo4j/vector DB benchmarks** — verified comparisons limited to Kuzu C++ and LadybugDB C++
2. **Physical operator count** — 48 vs C++ 67 (split-phase accounting difference; essential parity ~90%)
3. **`akar-main` is the primary published crate** — install via `cargo add akar-main`;
   `akar-c` (C FFI) is intentionally not published (build locally)
4. **WASM** — some extensions excluded from WASM builds (DuckDB, SQLite, Postgres, Neo4j,
   HTTPFS, Delta, Iceberg, Azure, Unity Catalog)
5. **Vector index (HNSW) is write-only from SQL** — optimizer pass `VectorSimilarityDetection` is a
   documented NO-OP (P52.5), so `VectorSimilarityScan` is unreachable from queries; the index is
   still maintained (refreshed after DML, P52.38) but vector queries go through normal scan+filter.
    GDS table functions (`shortest_path`, `page_rank`, ...) read the graph from `TableCatalog` via
    `CatalogGraphSource` (P52.46, committed `0290a8c`); a hard-coded 5-node sample ring fallback is
    used only when no catalog is available (registry/direct-call paths)

---

## 19. References

- **Research paper:** [Kuzu GDBMS, CIDR 2023](https://www.cidrdb.org/cidr2023/papers/p48-jin.pdf) (original architecture)
- **Benchmark comparison:** [BENCHMARK_COMPARISON.md](akar-core/BENCHMARK_COMPARISON.md)
- **Migration guide:** [MIGRATION.md](MIGRATION.md)
- **Release process:** [RELEASE.md](akar-core/RELEASE.md)
