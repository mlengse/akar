# Kuzu Core — Pure Rust Port

A from-scratch Rust port of [Kuzu](https://github.com/kuzudb/kuzu), an embedded property graph database management system (GDBMS) with openCypher query support.

## Architecture

```
kuzu-core/
├── kuzu-common/        # Type system (37 LogicalTypes, Value, DataChunk), memory, serialization
├── kuzu-storage/       # Columnar storage: BufferManager, WAL, compression, CSV/Parquet readers
├── kuzu-transaction/   # MVCC transaction manager with timestamp ordering
├── kuzu-catalog/       # System catalog (schemas, tables, types, columns)
├── kuzu-parser/        # PEG grammar (pest.rs) for full Cypher clause set
├── kuzu-binder/        # Semantic analysis, symbol resolution, type inference
├── kuzu-planner/       # Logical query plan construction (11 operator types)
├── kuzu-optimizer/     # 6 flat passes + 2 tree passes (FactorizationRewriting, CardinalityEstimation)
├── kuzu-processor/     # Physical operator execution (9+ operator types)
├── kuzu-function/      # Built-in function registry (50+ functions)
├── kuzu-graph/         # CSR adjacency, graph algorithms (BFS, PageRank, WCC, SCC, K-Core)
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
│ (pest.rs)   │    │(Catalog) │    │(logical) │    │ (6 flat + 2  │    │ (physical)   │
│ COPY, MATCH │    │(types)   │    │11 ops    │    │  tree passes)│    │9+ operators  │
│ DELETE, SET │    │(symbols) │    │          │    │ FactorRewr   │    │ Scan, Filter │
│ WITH, UNION │    │          │    │          │    │ CardEstimate │    │ HashJoin, etc│
│ UNWIND, etc │    │          │    │          │    │ JoinReorder  │    │              │
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
| Aggregation | ✅ | `RETURN COUNT(*), SUM(n.age), AVG(n.score), MIN(n.age), MAX(n.age)` |
| `GROUP BY` | ✅ (multi-key) | `RETURN n.gender, COUNT(*) GROUP BY n.gender` |
| `HAVING` | ✅ | `RETURN n.gender, COUNT(*) AS c GROUP BY n.gender HAVING c > 1` |
| Expressions | ✅ | Arithmetic, boolean, string functions, property access, function calls |
| Prepared Statements | ✅ | `conn.prepare("...")` + `conn.execute(&stmt, params)` |

## Test Suite Status

```
Total: 514+ tests — all passing ✅
```

| Crate | Tests | Status | Coverage |
|-------|-------|--------|----------|
| `kuzu-storage` | 129 | ✅ | BufferManager, Column*Chunk, NodeGroup, Table, Compression, WAL, Checkpoint, CSV/Parquet readers, Index |
| `kuzu-function` | 70 | ✅ | 50+ registered functions, scalar/aggregate/table dispatch |
| `kuzu-optimizer` | 42 | ✅ | 6 flat passes + FactorizationRewriting + CardinalityEstimation + JoinOrder |
| `kuzu-processor` | 28 | ✅ | PhysicalScan, Filter, Projection, Limit, OrderBy, Aggregate, HashJoin, CopyFrom, Delete, Set |
| `kuzu-main` (unit) | 15 | ✅ | Database, Connection, QueryResult, DDL/DML, COPY FROM |
| `kuzu-main` (integration) | 28 | ✅ | Full pipeline: parse→bind→plan→optimize→execute |
| `kuzu-common` | 25 | ✅ | Types (37 LogicalTypes, 17 PhysicalTypes, Value), Vectors, Memory, Serialization |
| `kuzu-parser` | 20 | ✅ | Cypher PEG grammar, 13 clause types, operator precedence |
| `kuzu-graph` | 16 | ✅ | CSR adjacency, BFS, PageRank, WCC, Shortest Path, Reachable Within |
| `kuzu-binder` | 13 | ✅ | Semantic analysis, type inference, symbol resolution |
| `kuzu-catalog` | 14 | ✅ | Catalog CRUD, lookup by name/id, schema management |
| `kuzu-transaction` | 11 | ✅ | MVCC, begin/commit/rollback, conflict detection |
| `kuzu-planner` | 14 | ✅ | Logical plan construction (11 operator variants) |
| `kuzu-json` | 12 | ✅ | extract, valid, type, structure, contains, keys, array_length |
| `kuzu-fts` | 14 | ✅ | Stemmer, Tokenizer, TF-IDF, BM25, stop words |
| `kuzu-algo` | 10 | ✅ | PageRank, WCC, SCC×2, K-Core, Louvain, spanning forest |
| `kuzu-llm` | 9 | ✅ | LLM function integration |
| `kuzu-duckdb` | 9 | ✅ | In-memory/file/local modes |
| `kuzu-httpfs` | 7 | ✅ | HTTP/HTTPS/S3 read support |
| `kuzu-vector` | 10 | ✅ | Vector similarity search |
| `kuzu-neo4j` | 12 | ✅ | Bolt protocol integration |
| Extension crates | 6 | ✅ | Azure(1), Delta(1), Iceberg(1), Postgres(1), SQLite(1), Unity(1) |

## Data Loading

| Format | Reader | Status |
|--------|--------|--------|
| CSV | `CsvReader` (csv crate) | ✅ Header detection, custom delimiter/quote/escape, type coercion, error reporting |
| TSV | `CsvReader` (tab delimiter) | ✅ |
| Parquet | `ParquetReader` (arrow/parquet crates) | ✅ Row group reading, Arrow→Kuzu type mapping, projection pushdown |
| HTTP(S)/S3 | `kuzu-httpfs` extension | ✅ |

## Optimizer Passes

| Pass | Type | Description |
|------|------|-------------|
| RemoveUnnecessary | Flat | Eliminates redundant operators |
| FilterPushDown | Flat | Pushes filters closer to scans |
| ProjectionPushDown | Flat | Eliminates unused columns early |
| ConstantFolding | Flat | Evaluates constant expressions at plan time |
| JoinOptimization | Flat | Removes redundant join conditions |
| TopK | Flat | Converts OrderBy + Limit to TopK scan |
| FactorizationRewriting | Tree | Inserts Flatten operators for hash joins |
| CardinalityEstimation | Tree | Annotates operators with estimated row counts |
| JoinOrderEnumeration | Tree | Greedy reorder of joins by cardinality |

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

MIT — see [LICENSE](../LICENSE).
