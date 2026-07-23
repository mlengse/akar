# Akar Core — Pure Rust Graph Database

[![Rust CI](https://github.com/anjangkusumanetra/akar/actions/workflows/rust-ci.yml/badge.svg)](https://github.com/anjangkusumanetra/akar/actions/workflows/rust-ci.yml)

Akar is a from-scratch pure Rust implementation of an embedded property graph database management system (GDBMS) with openCypher query support.

## Architecture

```
kuzu-core/
├── kuzu-common/        # Type system (37 LogicalTypes, Value, DataChunk), memory, serialization
├── kuzu-storage/       # Columnar storage: BufferManager, WAL+Replayer, UndoBuffer, PageManager, compression, CSV/Parquet readers
├── kuzu-transaction/   # MVCC + TransactionContext (AUTO/MANUAL), checkpoint worker, conflict detection
├── kuzu-catalog/       # System catalog (schemas, tables, types, columns)
├── kuzu-parser/        # PEG grammar (pest.rs) for full Cypher clause set
├── kuzu-binder/        # Semantic analysis, symbol resolution, type inference
├── kuzu-planner/       # Logical query plan construction (34 LogicalOperator variants)
├── kuzu-optimizer/     # 14 flat passes + 7 tree passes (21 total) — melebihi C++ Ladybug
├── kuzu-processor/     # Physical operator execution (22+ operator types: AggregateHashTable, JoinHashTable, BlockMergeSorter, etc.)
├── kuzu-function/      # Built-in function registry (110+ functions)
├── kuzu-graph/         # CSR adjacency, GDS framework (BFS, Dijkstra, PageRank, WCC, SCC, K-Core, Louvain)
├── kuzu-extension/     # Extension framework trait + registry
├── kuzu-json/          # JSON extension (extract, validate, type, structure, contains)
├── kuzu-fts/           # Full-Text Search extension (stemmer, tokenizer, BM25, TF-IDF)
├── kuzu-algo/          # Graph algorithm extensions (PageRank, WCC, SCC, K-Core, Louvain)
├── kuzu-httpfs/        # HTTP/HTTPS/S3 file system support
├── kuzu-duckdb/        # DuckDB integration (in-memory/file modes via duckdb crate)
├── kuzu-sqlite/        # SQLite integration (native rusqlite)
├── kuzu-postgres/      # PostgreSQL integration (tokio-postgres)
├── kuzu-delta/         # Delta Lake integration (DuckDB delegation)
├── kuzu-iceberg/       # Apache Iceberg integration (DuckDB delegation)
├── kuzu-azure/         # Azure Blob Storage integration (abfss:// URI)
├── kuzu-unity-catalog/ # Unity Catalog integration (DuckDB delegation)
├── kuzu-neo4j/         # Neo4j integration (bolt protocol)
├── kuzu-llm/           # LLM integration functions
├── kuzu-vector/        # Vector similarity search extension
├── kuzu-main/          # Public API: Database, Connection, QueryResult, PreparedStatement
└── kuzu-cli/           # Interactive Cypher shell (REPL)
```

## Quick Start

```bash
cargo build --release
cargo run --bin kuzu-cli
```

Or with a persistent database:

```bash
cargo run --bin kuzu-cli -- /path/to/db
```

### Example: Cypher via Rust API

```rust
use kuzu_main::database::{Database, SystemConfig};
use kuzu_main::connection::Connection;

let db = Database::new("/path/to/db", SystemConfig::default())?;
let conn = Connection::new(&db);

conn.query("CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name))")?;
conn.query("COPY Person FROM 'data.csv' (HEADER true)")?;

let result = conn.query("MATCH (p:Person) WHERE p.age > 25 RETURN p.name ORDER BY p.age")?;
```

## Query Pipeline

```
Cypher text
    │
    ▼
┌─────────────┐    ┌──────────┐    ┌──────────┐    ┌──────────────┐    ┌──────────────┐
│   Parser    │───▶│  Binder  │───▶│  Planner │───▶│  Optimizer   │───▶│  Processor   │
│ (pest.rs)   │    │(Catalog)   │    │(logical) │    │ (11 flat + 7 │    │ (physical)   │
│ COPY, MATCH │    │(types)   │    │34 ops    │    │  tree passes)│    │22+ operators │
│ DELETE, SET │    │(symbols) │    │          │    │ FilterPush   │    │ Scan, Filter │
│ WITH, UNION │    │          │    │          │    │ JoinReorder  │    │ HashJoin, etc│
│ UNWIND, etc │    │          │    │          │    │ SIP, CSU,    │    │ SemiMasker,  │
│ FOREACH,    │    │          │    │          │    │ AccHashJoin, │    │ RecursiveExt,│
│ MERGE       │    │          │    │          │    │ AggKeyDep,   │    │ Intersect,   │
│ ANALYZE     │    │          │    │          │    │ OrderByPush, │    │ CountRelTbl  │
└─────────────┘    └──────────┘    └──────────┘    └──────────────┘    └──────────────┘
                                                                             │
                                                                             ▼
                                                                       DataChunks
```

## Supported Cypher Clauses

| Clause | Status | Example |
|--------|--------|---------|
| `CREATE NODE TABLE` | ✅ | `CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name))` |
| `CREATE REL TABLE` | ✅ | `CREATE REL TABLE knows (FROM Person TO Person, since DATE, MANY_MANY)` |
| `DROP TABLE` | ✅ | `DROP TABLE Person` |
| `COPY FROM` (CSV) | ✅ | `COPY Person FROM 'file.csv' (HEADER true)` |
| `COPY FROM` (Parquet) | ✅ | `COPY Person FROM 'file.parquet'` |
| `MATCH` | ✅ | `MATCH (n:Person) RETURN n.name` |
| `MATCH` (edge patterns) | ✅ | `MATCH (a:Person)-[:knows]->(b:Person) RETURN a.name, b.name` |
| `WHERE` | ✅ | `MATCH (n) WHERE n.age > 25 RETURN n.name` |
| `RETURN` | ✅ | `RETURN n.name, n.age ORDER BY n.age LIMIT 10` |
| `ORDER BY` | ✅ (multi-key) | `RETURN n.name ORDER BY n.age DESC, n.score` |
| `LIMIT` / `OFFSET` | ✅ | `RETURN n.name LIMIT 5 OFFSET 10` |
| `SKIP` | ✅ | `RETURN n.name SKIP 10 LIMIT 5` |
| `DELETE` | ✅ | `MATCH (n:Person) WHERE n.name='Alice' DELETE n` |
| `SET` | ✅ | `MATCH (n:Person) WHERE n.name='Alice' SET n.age=35` |
| `ALTER TABLE` (ADD) | ✅ | `ALTER TABLE Person ADD COLUMN email STRING` |
| `ALTER TABLE` (DROP) | ✅ | `ALTER TABLE Person DROP COLUMN email` |
| `ALTER TABLE` (RENAME COLUMN) | ✅ | `ALTER TABLE Person RENAME COLUMN name TO full_name` |
| `ALTER TABLE` (RENAME TABLE) | ✅ | `ALTER TABLE Person RENAME TO Employee` |
| `OPTIONAL MATCH` | ✅ | `OPTIONAL MATCH (n:Person) RETURN n.name` |
| `WITH` | ✅ | `MATCH (n) WITH n.name AS name RETURN name ORDER BY name` |
| `UNION ALL` | ✅ | `RETURN 1 AS x UNION ALL RETURN 2 AS x` |
| `UNWIND` | ✅ | `UNWIND [1,2,3] AS x RETURN x` |
| `CREATE` (node) | ✅ | `CREATE (:Person {name: 'Alice', age: 25})` |
| Aggregation | ✅ | `RETURN COUNT(*), SUM(n.age), AVG(n.score), MIN(n.age), MAX(n.age), PERCENTILE_DISC(n.age, 0.5)` |
| `ANALYZE` | ✅ | `ANALYZE Person`, `ANALYZE *` — collect table statistics |
| `GROUP BY` | ✅ (multi-key) | `RETURN n.gender, COUNT(*) GROUP BY n.gender` |
| `HAVING` | ✅ | `RETURN n.gender, COUNT(*) AS c GROUP BY n.gender HAVING c > 1` |
| `CREATE SEQUENCE` | ✅ | `CREATE SEQUENCE seq1;` |
| `DROP SEQUENCE` | ✅ | `DROP SEQUENCE seq1;` |
| `CREATE MACRO` | ✅ | `CREATE MACRO add(a,b) AS a + b;` |
| `EXPORT DATABASE` | ✅ | `EXPORT DATABASE '/path/export';` |
| `IMPORT DATABASE` | ✅ | `IMPORT DATABASE '/path/export';` |
| `FOREACH` | ✅ | `FOREACH (x IN [1,2,3] | CREATE (:N {p: x}))` |
| `MERGE` | ✅ | `MERGE (n:Person {name: 'Alice'})` ON CREATE SET n.age=30 ON MATCH SET n.age=31 |
| Variable-length paths | ✅ | `MATCH (a)-[*1..5]->(b) RETURN a, b` |
| Expressions | ✅ | Arithmetic, boolean, string functions, property access, function calls |
| Prepared Statements | ✅ | `conn.prepare("...")` + `conn.execute(&stmt, params)` |
| `BEGIN` / `COMMIT` / `ROLLBACK` | ✅ | `BEGIN TRANSACTION`, `COMMIT`, `ROLLBACK` with AUTO/MANUAL mode |
| `CALL` (table functions) | ✅ | `CALL show_tables()`, `table_info()`, `show_functions()`, `show_indexes()`, `show_sequences()`, `show_macros()`, `show_connection()`, `db_version()`, `catalog_version()`, `current_setting()`, `stats_info()`, `storage_info()`, `bm_info()`, `file_info()`, `free_space_info()`, `disk_size_info()`, `storage_version()`, `show_loaded_extensions()`, `show_official_extensions()`, `clear_warnings()`, `show_warnings()` |

## Test Suite Status

```
Total: 954 tests — all passing ✅ (61 integration tests)
```

| Crate | Tests | Status | Coverage |
|-------|-------|--------|----------|
| `kuzu-common` | 21 | ✅ | Types (37 LogicalTypes, 17 PhysicalTypes, Value), Vectors, Memory, Serialization |
| `kuzu-parser` | 63 | ✅ | Cypher PEG grammar, 35+ Statement variants (incl. ANALYZE), operator precedence |
| `kuzu-binder` | 14 | ✅ | Semantic analysis, type inference, symbol resolution |
| `kuzu-planner` | 16 | ✅ | Logical plan construction (34 LogicalOperator variants) |
| `kuzu-optimizer` | 49 | ✅ | 14 flat passes + 7 tree passes (21 total, exceeds C++ Ladybug) |
| `kuzu-processor` | 77 | ✅ | PhysicalScan, Filter, Projection, Limit, OrderBy (RadixSort+BlockMergeSorter), Aggregate (parallel AggregateHashTable), HashJoin (parallel JoinHashTable), Intersect, SemiJoin, AntiJoin, SemiMasker, RecursiveExtend, CopyFrom (batch insert), CountRelTable, Delete, Set |
| `kuzu-function` | 159 | ✅ | 110+ registered functions (incl. PERCENTILE_DISC/CONT), scalar/aggregate/table dispatch |
| `kuzu-storage` | 242 | ✅ | BufferManager, Column*Chunk, NodeGroup, Table, Compression, WAL+Replayer, Checkpoint, CSV/Parquet readers, Index, FSM, Zone Map, UndoBuffer, PageManager |
| `kuzu-main` (unit) | 64 | ✅ | Database, Connection, QueryResult, DDL/DML, COPY FROM, CALL functions |
| `kuzu-main` (integration) | 44 | ✅ | RETURN *, FOREACH, MERGE, subqueries |
| `kuzu-catalog` | 37 | ✅ | Catalog CRUD, lookup by name/id, schema management, sequences |
| `kuzu-transaction` | 12 | ✅ | MVCC, begin/commit/rollback, AUTO/MANUAL modes, checkpoint worker, conflict detection |
| `kuzu-graph` | 31 | ✅ | CSR adjacency, GDS framework (BFS, Dijkstra, PageRank, WCC, SCC, K-Core, Louvain, Shortest Path) |
| `kuzu-vector` | 20 | ✅ | Vector similarity search |
| `kuzu-json` | 12 | ✅ | extract, valid, type, structure, contains, keys, array_length |
| `kuzu-fts` | 14 | ✅ | Stemmer, Tokenizer, TF-IDF, BM25, stop words |
| `kuzu-algo` | 19 | ✅ | PageRank, WCC, SCC×2, K-Core, Louvain, spanning forest, shortest path algorithms |
| `kuzu-llm` | 9 | ✅ | LLM function integration |
| `kuzu-duckdb` | 9 | ✅ | In-memory/file/local modes |
| `kuzu-httpfs` | 7 | ✅ | HTTP/HTTPS/S3 read support |
| `kuzu-neo4j` | 12 | ✅ | Bolt protocol integration |
| `kuzu-wasm` | - | ✅ | KuzuDatabase, KuzuConnection, PreparedStatement wrappers untuk NodeJS |
| Extension crates | 6 | ✅ | Azure(1), Delta(1), Iceberg(1), Postgres(1), SQLite(1), Unity(1) |

## Storage Engine Features

| Feature | Status | Description |
|---------|--------|-------------|
| Buffer Manager | ✅ | Clock eviction, page pin/unpin |
| Page Manager | ✅ | Page allocation/deallocation via FSM (`PageManager`) |
| Undo Buffer | ✅ | Rollback safety via undo record replay (`UndoBuffer`) |
| WAL + Replayer + DDL | ✅ | Write-ahead logging, crash recovery, 6 DDL record types (`WALReplayer`) |
| Free Space Manager | ✅ | Buddy-system allocation integrated in `FileHandle::allocate_page()` |
| Zone Map Predicate | ✅ | `ColumnChunkStats`-based predicate pushdown in `NodeTable::to_column_major_data_with_predicate()` |
| ART Index | ✅ | Node4/16/48/256 adaptive radix tree |
| HNSW Index | ✅ | Vector similarity search index (`VectorIndexTable`) |
| Hash Index | ✅ | On-disk + in-memory |
| WAL + Checkpointer | ✅ | Write-ahead logging, shadow file |
| Compression | ✅ | Constant, Boolean, dictionary encoding |
| MVCC / Multiwriter | ✅ | Transaction isolation, AUTO/MANUAL modes, dynamic table-level locking, OCC conflict detection |
| Virtual File System (VFS) | ✅ | Extensible registry for resolving files via HTTP/HTTPS/S3/Local |

## GDS (Graph Data Science) Framework

| Algorithm | Status | Description |
|-----------|--------|-------------|
| BFS | ✅ | Breadth-first search (Dense/Sparse frontiers) |
| Shortest Path (SSP) | ✅ | Single-source shortest path |
| All Shortest Paths (ASP) | ✅ | All shortest paths between nodes |
| Weighted Shortest Path (WSP) | ✅ | Dijkstra-based weighted shortest path |
| All Weighted Shortest Paths (AWSP) | ✅ | All weighted shortest paths |
| PageRank | ✅ | Iterative PageRank computation |
| WCC | ✅ | Weakly Connected Components |
| SCC | ✅ | Strongly Connected Components (Kosaraju) |
| K-Core Decomposition | ✅ | K-core decomposition |
| Louvain | ✅ | Community detection via Louvain method |
| Spanning Forest | ✅ | Minimum spanning forest |

## SIP (Sideways Information Passing)

| Component | Status |
|-----------|--------|
| LogicalSemiMasker operator | ✅ |
| PhysicalSemiMasker | ✅ |
| NodeSemiMask (Arc<AtomicBool>) | ✅ |
| ScanNode semi_mask integration | ✅ |
| SIPOptimization tree pass | ✅ |

## Data Loading

| Format | Reader | Status |
|--------|--------|--------|
| CSV | `CsvReader` (csv crate) | ✅ Header detection, custom delimiter/quote/escape, type coercion, error reporting |
| TSV | `CsvReader` (tab delimiter) | ✅ |
| Parquet | `ParquetReader` (arrow/parquet crates) | ✅ Row group reading, Arrow→Kuzu type mapping, projection pushdown |
| HTTP(S)/S3 | `kuzu-httpfs` extension | ✅ |

## Optimizer Passes — 21 Total (14 Flat + 7 Tree)

### Flat Passes
| # | Pass | Description |
|---|------|-------------|
| 1 | RemoveUnnecessaryOperators | Eliminates redundant operators |
| 2 | FilterPushDown | Pushes filters closer to scans |
| 3 | ProjectionPushDown | Eliminates unused columns early |
| 4 | ConstantFolding | Evaluates constant expressions at plan time |
| 5 | AggregateDetection | Detects and marks aggregation boundaries |
| 6 | JoinOptimization | Greedy cardinality-aware join reordering |
| 7 | TopKOptimization | Converts OrderBy + Limit to TopK scan |
| 8 | VectorSimilarityDetection | Detects vector similarity patterns |
| 9 | ArtRangeScanDetection | Detects ART index range scan patterns |
| 10 | LimitPushDown | Pushes limits closer to scans |
| 11 | CommonSubexpressionElimination | Eliminates duplicate expressions |
| 12 | **OrderByPushDown** | Pushes ORDER BY below UNION ALL (Ladybug) |
| 13 | **UnwindDedup** | Deduplicates consecutive UNWIND (Ladybug) |
| 14 | **CountRelTable** | Replaces ScanRel+COUNT with CSR metadata (Ladybug) |

### Tree Passes
| # | Pass | Description |
|---|------|-------------|
| 1 | FactorizationRewriting | Inserts Flatten operators for hash joins |
| 2 | ForeignJoinPushDown | Pushes foreign joins through operators |
| 3 | AccHashJoinOptimization | Optimizes accumulated hash joins |
| 4 | SIPOptimization | Sideways Information Passing via SemiMasker |
| 5 | CorrelatedSubqueryUnnesting | Unnests correlated subqueries |
| 6 | AggKeyDependency | Removes redundant grouping keys |
| 7 | CardinalityEstimation | Annotates operators with estimated row counts (StatsStore) |

## Extensions

| Extension | Type | Functions |
|-----------|------|-----------|
| **JSON** | Native Rust | `json_extract`, `json_valid`, `json_type`, `json_structure`, `json_contains`, `json_keys`, `json_array_length`, etc. |
| **FTS** | Native Rust | `stemmer`, `tokenizer`, `tf_idf`, `bm25`, `query`, `stop_words` |
| **ALGO** | Native Rust | `page_rank`, `wcc`, `scc`, `scc_kosaraju`, `k_core_decomposition`, `louvain`, `spanning_forest` |
| **HTTPFS** | Native Rust | HTTP/HTTPS/S3 file reads |
| **Vector** | Native Rust | Vector similarity search |
| **Neo4j** | Native Rust | Bolt protocol |
| **LLM** | Native Rust | LLM function integration |
| **DuckDB** | Rust crate | `duckdb_query`, `duckdb_scan` |
| **SQLite** | Native rusqlite | `sqlite_query`, `sqlite_scan` |
| **Postgres** | Native tokio-postgres | `sql_query` |
| **Delta** | DuckDB delegation | `delta_scan` |
| **Iceberg** | DuckDB delegation | `iceberg_scan`, `iceberg_metadata`, `iceberg_snapshots` |
| **Azure** | DuckDB delegation | `azure_scan` |
| **UnityCatalog** | DuckDB delegation | `uc_scan` |

## Benchmark Infrastructure

| Area | Benchmarks | Tool |
|------|-----------|------|
| Full pipeline | `query/match_return_all`, `query/match_order_by`, `query/match_limit` | criterion (kuzu-main) |
| Storage | `buffer/pin_unpin`, `scan/small_100_rows`, `scan/medium_1k_rows` | criterion (kuzu-main) |
| Scan throughput | 100/1K/10K rows, selective columns | criterion (kuzu-processor) |
| Filter throughput | Pass-all, remove-all, property check, batch, multi-column | criterion (kuzu-processor) |
| Hash join throughput | Various build/probe sizes, multi-column, no-match | criterion (kuzu-processor) |
| Sort throughput | Single/multi-key, ascending/descending, 100/1K/10K | criterion (kuzu-processor) |
| Aggregate throughput | COUNT, SUM, AVG, multi-func, GROUP BY (int/string/multi-key) | criterion (kuzu-processor) |

See [BENCHMARK_RUST.md](./BENCHMARK_RUST.md) for baseline numbers and [BENCHMARK_COMPARISON.md](./BENCHMARK_COMPARISON.md) for Rust vs C++ comparison.

## Building

### Prerequisites

- Rust 1.80+ (MSRV)
- Cargo

### Full Workspace

```bash
cargo build --release --workspace
cargo test --workspace
```

### Native (Windows/MinGW)

```bash
cargo build --target x86_64-pc-windows-gnu
cargo test --target x86_64-pc-windows-gnu --workspace
```

### WASM

```bash
rustup target add wasm32-unknown-unknown
cargo check --target wasm32-unknown-unknown --workspace
```

### With Extensions

Extensions are feature-gated:

```bash
# All extensions
cargo build --release -p kuzu-main --features json-extension,fts-extension,algo-extension

# WASM-safe (no native deps)
cargo check --target wasm32-unknown-unknown -p kuzu-main --features json-extension,fts-extension
```

### Running Benchmarks

```bash
# Full pipeline benchmarks
cargo bench -p kuzu-main

# Operator micro-benchmarks
cargo bench -p kuzu-processor

# All benchmarks
cargo bench --workspace
```

## License

GPLv3 — see [LICENSE](../LICENSE).

Copyright (c) 2026 Anjang Kusuma Netra
