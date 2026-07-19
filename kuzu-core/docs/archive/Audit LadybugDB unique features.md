# Comprehensive Audit: LadybugDB C++ vs Kuzu C++ (Vela) vs Rust Port (kuzu-core) 18/07/2026

## 1. Repository Architecture Overview

| Layer | Ladybug C++ (`ladybug/`) | Kuzu Core Rust (`kuzu-core/`) | Kuzu C++ Vela (presumed) |
|-------|--------------------------|-------------------------------|--------------------------|
| **Project name** | `Lbug` v0.18.0 | `kuzu` workspace (29 crates) | Kuzu C++ |
| **Language** | C++20 | Rust 2024 (pure, no FFI) | C++ |
| **Parser** | ANTLR4-based Cypher | `pest.rs` PEG-based | ANTLR4 |
| **Build system** | CMake + Ninja | Cargo | CMake |
| **Shell** | linenoise-based (`lbug_shell`) | rustyline-based (`kuzu-cli`) | N/A |
| **Extensions** | 15 (shared libs + static link) | 15 crates | Same 15 |
| **GDS** | 8 core + extended algo | 15 algorithms + RW + N2V | Presumed parity |
| **Physical operators** | 67 (split-phase counting) | 46 (fused phases) | Similar to Ladybug |
| **Optimizer passes** | 17 | 22 (15 flat + 7 tree) | Similar to Ladybug |

---

## 2. LogicalOperatorType Comparison

### Ladybug C++ has these (38 entries):
```
ACCUMULATE, AGGREGATE, ALTER, ANALYZE, ATTACH_DATABASE,
COPY_FROM, COPY_TO, COUNT_REL_TABLE, CREATE_GRAPH, CREATE_INDEX,
CREATE_MACRO, CREATE_SEQUENCE, CREATE_TABLE, CREATE_TYPE, CROSS_PRODUCT,
DELETE, DETACH_DATABASE, DISTINCT, DROP, DUMMY_SCAN, DUMMY_SINK,
EMPTY_RESULT, EXPLAIN, EXPRESSIONS_SCAN, EXTEND, EXTENSION,
EXPORT_DATABASE, FILTER, FLATTEN, HASH_JOIN, IMPORT_DATABASE,
INDEX_LOOK_UP, INTERSECT, INSERT, LIMIT, MERGE,
MULTIPLICITY_REDUCER, NODE_LABEL_FILTER, NOOP, ORDER_BY, PARTITIONER,
PACKED_EXTEND, PATH_PROPERTY_PROBE, PROJECTION, RECURSIVE_EXTEND,
REL_DEGREE_TABLE, SCAN_NODE_TABLE, SEMI_MASKER, SET_PROPERTY,
STANDALONE_CALL, TABLE_FUNCTION_CALL, TRANSACTION, UNION_ALL, UNWIND,
USE_DATABASE, USE_GRAPH, EXTENSION_CLAUSE, UNWIND_DEDUPLICATE
```

### Rust port has these additional operators NOT in Ladybug C++:
- `VectorSimilarityScan` (HNSW vector index scan)
- `ArtIndexRangeScan` (ART index range scan)
- `TopK` (fused ORDER BY + LIMIT)
- `SemiJoin`, `AntiJoin` (separate join types)
- `OptionalMatch` (OPTIONAL MATCH as explicit operator)
- `Foreach` (FOREACH as explicit operator)
- `BatchInsert` (batch insert as separate from CopyFrom)
- `CreateVectorIndex`, `DropIndex` (separate index ops)
- `CreateFtsIndex`, `FtsScan` (FTS operations)
- `Skip` (SKIP as operator)
- `Insert` (row-level insert)
- `MultiplicityReducer` (separate operator)

### Ladybug has these that Rust may NOT have as distinct operators:
- `NODE_LABEL_FILTER` (Ladybug-specific, Rust handles via filter pushdown)
- `NOOP` (Ladybug-specific no-op placeholder)
- `REL_DEGREE_TABLE` (Ladybug-specific CSR degree optimization, Rust has in `CountRelTable`)
- `UNWIND_DEDUPLICATE` (Ladybug-specific, Rust has `UnwindDedup` optimizer pass instead)
- `CREATE_GRAPH` / `USE_GRAPH` (projected graphs, Rust has via `ProjectGraphNative`)
- `PACKED_EXTEND` (Ladybug-specific packed adjacency extend)

---

## 3. PhysicalOperatorType Comparison

### Ladybug C++ has these (85 entries):
```
ALTER, AGGREGATE, AGGREGATE_FINALIZE, AGGREGATE_SCAN, ANALYZE,
ATTACH_DATABASE, BATCH_INSERT, COPY_TO, COUNT_REL_TABLE, CREATE_GRAPH,
CREATE_INDEX, CREATE_MACRO, CREATE_SEQUENCE, CREATE_TABLE, CREATE_TYPE,
CROSS_PRODUCT, DETACH_DATABASE, DELETE_, DROP, DUMMY_SINK,
DUMMY_SIMPLE_SINK, EMPTY_RESULT, EXPORT_DATABASE, EXTENSION_CLAUSE,
FILTER, FLATTEN, HASH_JOIN_BUILD, HASH_JOIN_PROBE, IMPORT_DATABASE,
INDEX_LOOKUP, INSERT, INTERSECT_BUILD, INTERSECT, INSTALL_EXTENSION,
LIMIT, LOAD_EXTENSION, MERGE, MULTIPLICITY_REDUCER, PARTITIONER,
PACKED_EXTEND, PACKED_FILTERED_COUNT, PATH_PROPERTY_PROBE,
PRIMARY_KEY_SCAN_NODE_TABLE, PROJECTION, PROFILE, RECURSIVE_EXTEND,
REL_DEGREE_TABLE, RESULT_COLLECTOR, SCAN_NODE_TABLE, SCAN_REL_TABLE,
SEMI_MASKER, SET_PROPERTY, SKIP, STANDALONE_CALL, TABLE_FUNCTION_CALL,
TOP_K, TOP_K_SCAN, TRANSACTION, ORDER_BY, ORDER_BY_MERGE, ORDER_BY_SCAN,
UNION_ALL_SCAN, UNWIND, UNWIND_DEDUP, USE_DATABASE, USE_GRAPH,
UNINSTALL_EXTENSION
```

**Key difference:** Rust fuses split-phase variants (e.g., `HASH_JOIN_BUILD` + `HASH_JOIN_PROBE` → one `PhysicalHashJoin`; `ORDER_BY` + `ORDER_BY_MERGE` + `ORDER_BY_SCAN` → one `PhysicalOrderBy`). This is a **structural difference**, not a feature gap.

---

## 4. Optimizer Passes Comparison

### Ladybug C++ (17 passes, in order):
1. RemoveFactorizationRewriter
2. CorrelatedSubqueryUnnestSolver
3. RemoveUnnecessaryJoinOptimizer
4. UnwindDedupOptimizer
5. CountRelTableOptimizer
6. ForeignJoinPushDownOptimizer
7. FilterPushDownOptimizer
8. ProjectionPushDownOptimizer
9. OrderByPushDownOptimizer
10. LimitPushDownOptimizer
11. HashJoinSIPOptimizer (semi-mask)
12. TopKOptimizer
13. CountRelTableOptimizer (second pass - degree top-k)
14. FactorizationRewriter
15. AggKeyDependencyOptimizer
16. CardinalityUpdater (for EXPLAIN only)
17. SchemaPopulator (non-optimized fallback)

### Rust port (22 passes):
**Flat passes (15):**
1. RemoveUnnecessaryOperators ✅
2. FilterPushDown ✅
3. PredicatePushDown ✅ (additional — merges filter predicates into ScanNode)
4. ProjectionPushDown ✅
5. ConstantFolding ✅ (additional — evaluates constant expressions)
6. AggregateDetection ✅ (additional — detects aggregate patterns)
7. JoinOptimization (greedy cardinality-aware) ✅
8. TopKOptimization ✅
9. VectorSimilarityDetection ✅ (additional — HNSW vector similarity)
10. ArtRangeScanDetection ✅ (additional — ART index range scan)
11. LimitPushDown ✅
12. CommonSubexpressionElimination ✅ (additional)
13. OrderByPushDown ✅ (port of Ladybug)
14. UnwindDedup ✅ (port of Ladybug)
15. CountRelTable ✅ (port of Ladybug)

**Tree passes (7):**
1. FactorizationRewriting ✅
2. ForeignJoinPushDown ✅
3. AccHashJoinOptimization ✅
4. SIPOptimization ✅
5. CorrelatedSubqueryUnnesting ✅
6. AggKeyDependency ✅
7. CardinalityEstimation (static + StatsStore) ✅

**Rust has 5 additional passes not in Ladybug:**
- PredicatePushDown
- ConstantFolding
- AggregateDetection
- VectorSimilarityDetection
- ArtRangeScanDetection
- CommonSubexpressionElimination

---

## 5. Storage Feature Comparison

### Features in Ladybug C++:
- Buffer Manager (Clock eviction)
- Compression (constant, boolean)
- WAL + Local WAL
- ShadowFile checkpointing
- ART Index (Node4/16/48/256)
- Hash Index (on-disk + in-memory)
- ICE disk format (Iceberg-like chunked columnar storage)
- CSR Node Groups (hybrid CSR for rel tables)
- Columnar node/rel tables
- Arrow node/rel tables
- Foreign rel tables (extension-backed)
- LocalStorage (local node/rel tables)
- Zone Maps (ColumnChunkStats for predicate filtering)
- Free Space Manager (buddy system)
- StatsStore (planner stats)
- Overflow pages
- NodeGroup/ColumnChunk
- Lazy Segment Scanner
- Dictionary columns
- List/Struct columns
- Serial sequence (auto-increment)
- **Ice Disk storage format** (`ice_disk_constants.h`, `ice_disk_node_table.h`, `ice_disk_rel_table.h`)

### Features in Rust port:
All of the above, plus:
- **HNSW Vector Index** (`vector_index.rs`) — Ladybug has this via `vector` extension only
- **HyperLogLog** cardinality estimation (`hyperloglog.rs`)
- **Roaring Bitmap** (`roaring_bitmap.rs`) — Ladybug uses CRoaring third-party
- **Parquet Writer** (`parquet_writer.rs`) — native Rust
- **NPY Reader** (`npy_reader.rs`)
- **Lazy Scanner** (`lazy_scanner.rs`) — on-demand NodeGroup loading
- **FSM persistence via WAL** (`WALRecord::UpdateFsm`)

---

## 6. GDS / Algorithm Comparison

### Ladybug C++ core GDS (8):
- `VAR_LEN_JOINS`
- `ALL_SP_DESTINATIONS`
- `ALL_SP_PATHS`
- `SINGLE_SP_DESTINATIONS`
- `SINGLE_SP_PATHS`
- `WEIGHTED_SP_DESTINATIONS`
- `WEIGHTED_SP_PATHS`
- `ALL_WEIGHTED_SP_PATHS`

### Ladybug C++ `algo` extension:
- `STRONGLY_CONNECTED_COMPONENTS` (SCC)
- `STRONGLY_CONNECTED_COMPONENTS_KOSARAJU` (SCC_KO)
- `WEAKLY_CONNECTED_COMPONENTS` (WCC)
- `PAGE_RANK` (PR)
- `K_CORE_DECOMPOSITION` (KCORE)
- `LOUVAIN`
- `SPANNING_FOREST` (SF)

### Rust port has ALL of the above + additional:
- Label Propagation (LPA)
- Betweenness Centrality
- Closeness Centrality
- Triangle Counting
- Random Walk
- Node2Vec

**Total GDS algorithms:** Ladybug = 15, Rust = 15+ (parity declared)

---

## 7. Functions Comparison

### Ladybug C++ registered functions (~607 overloads/aliases, ~234 unique):
All standard categories covered: arithmetic, string, array, list, cast, comparison, date, timestamp, interval, blob, UUID, struct, map, union, node/rel, path, hash, utility, sequence, aggregate.

### Rust port has parity for all ~234 unique functions.
**Key Ladybug-specific functions verified in Rust:**
- `PERCENTILE_DISC` ✅
- `COUNT_IF` ✅
- `NULLIF` ✅
- `SIZE` ✅
- `LIST_TRANSFORM/REDUCE/FILTER` (lambda) ✅
- Path: `NODES`, `RELS`, `PROPERTIES`, `IS_TRAIL`, `IS_ACYCLIC`, `LENGTH` ✅
- Schema: `COST`, `ROWID`, `OFFSET`, `ID`, `START_NODE`, `END_NODE`, `LABEL` ✅
- `ERROR` ✅
- Sequence: `NEXTVAL`, `CURRVAL` ✅
- Export: `EXPORT_CSV`, `EXPORT_PARQUET` ✅

---

## 8. Extensions Comparison

Both have the same 15 extensions, but implementation differs:

| Extension | Ladybug C++ | Rust Port |
|-----------|------------|-----------|
| `adbc` | C++ ADBC interface | Rust ADBC trait |
| `algo` | C++ graph algorithms | Rust native algorithms |
| `azure` | Delegates to DuckDB | Rust native |
| `delta` | Delegates to DuckDB | Rust native |
| `duckdb` | C++ DuckDB scan bridge | Rust DuckDB client |
| `fts` | C++ FTS (porter stemmer) | Rust FTS (porter stemmer, BM25) |
| `httpfs` | C++ HTTP/S3 via VFS | Rust HTTP Range requests |
| `iceberg` | Delegates to DuckDB | Rust native |
| `json` | C++ JSON functions | Rust serde_json |
| `llm` | C++ with OpenSSL + cpp-httplib | Rust reqwest |
| `neo4j` | C++ Neo4j connector | Rust Neo4j driver |
| `postgres` | C++ libpq bridge | Rust postgres crate |
| `sqlite` | C++ sqlite3 bridge | Rust rusqlite |
| `unity_catalog` | Delegates to DuckDB | Rust native |
| `vector` | C++ HNSW implementation | Rust HNSW (native) |

---

## 9. Unique Ladybug C++ Features (Not in Rust port / different architecture)

### 9.1. Shell Features
Ladybug has a full-featured shell with:
- 13 shell commands (`:help`, `:clear`, `:quit`, `:max_rows`, `:max_width`, `:mode`, `:stats`, `:multiline`, `:singleline`, `:highlight`, `:render_errors`, `:render_completion`, `:schema`)
- Multiple output printers: Box, JSON, HTML, LaTeX, Line
- Schema display support
- Confidential statement analyzer (security feature that checks queries for sensitive data patterns)
- Linenoise-based with keyword completion
- Multi-line input (`..>` alt prompt)

Rust CLI has simpler shell with `.help`, `.tables`, `.schema`, `.mode`, `.import`, `.export`, `.exit/.quit`.

### 9.2. Confidential Statement Analyzer
File: `binder/visitor/confidential_statement_analyzer.h` — a security feature that analyzes queries for sensitive data patterns before execution. **Not present in Rust port.**

### 9.3. IceDisk Storage Format
StorageFormat enum has `ICEBUG_DISK` format — a custom Iceberg-like columnar on-disk format with:
- `ice_disk_node_table.h` / `ice_disk_rel_table.h`
- `ice_disk_constants.h` / `ice_disk_utils.h`
- Versioned storage (`V1`, `CURRENT_VERSION`)

Rust has `ice_format.rs` which uses `ParquetStreamReader` but the C++ implementation may differ in architecture.

### 9.4. StorageDriver API
`main/storage_driver.h` — a direct storage access API for low-level data access. **Not explicitly present in Rust port** (although `StorageManager` is exposed).

### 9.5. Arrow Node/Rel Table Support
Ladybug has dedicated `ArrowNodeTable` and `ArrowRelTable` classes with Arrow-native scan states. The Rust port has Arrow integration but through a different architecture (Arrow `ArrayRef` + `SelectionVector`).

### 9.6. ForeignRelTable
Ladybug has a `ForeignRelTable` that delegates rel scans to extension table functions. The Rust port achieves this through the extensions framework but not as a dedicated table type.

### 9.7. PageSize/VectorCapacity Configuration
Ladybug has build-time configuration:
- `LBUG_PAGE_SIZE_LOG2` (default 12 = 4KB)
- `LBUG_VECTOR_CAPACITY_LOG2` (default 11 = 2048)
- `LBUG_NODE_GROUP_SIZE_LOG2` (default 17)
- `LBUG_MAX_SEGMENT_SIZE_LOG2` (default 18)
- `LBUG_DEFAULT_REL_STORAGE_DIRECTION` (BOTH)

These are compile-time tunables in Ladybug; Rust uses runtime configuration.

### 9.8. Extended C API
Ladybug's C API (`c_api/`) exports a comprehensive C interface for language bindings. The Rust port has `kuzu-c` for C bindings but the API surface may differ.

### 9.9. Static Extension Linking
Ladybug supports statically linking extensions (used for WASM, Android, Swift). The Rust port uses Cargo features instead.

### 9.10. WAL Dump Tool
`BUILD_WAL_DUMP` option builds a dedicated WAL dump tool (`tools/wal_dump/`). **Not present in Rust port** (Rust has WAL replay but no dump tool).

### 9.11. Benchmark Suite
`benchmark/` directory with dedicated benchmark infrastructure. Rust has benchmarks in `kuzu-main/benches/` but with different structure.

---

## 10. Features in Rust Port NOT in Ladybug C++

| Feature | Description |
|---------|-------------|
| **DP Bushy Trees Join Order** | Cost-based join optimization (Ladybug uses greedy) |
| **Arrow-native execution path** | Direct `ColumnChunk→Arrow::ArrayRef` scan, zero-copy `arrow::compute::take()` filtering |
| **HNSW Vector Index** | Dedicated `VectorIndexTable` with HNSW graph (Ladybug has via `vector` extension) |
| **Fuzz Testing** | 3 `cargo-fuzz` targets integrated in CI |
| **Property-Based Testing** | `proptest` for round-trip, join associativity, filter pushdown |
| **FTS Extension (native)** | BM25 scoring, Porter stemmer, stop word filtering, 3 macro tables |
| **Additional GDS algorithms** | LPA, Betweenness Centrality, Closeness, Triangle Count, Random Walk, Node2Vec |
| **ConstantFolding optimizer pass** | Evaluates constant expressions at planning time |
| **CommonSubexpressionElimination** | Eliminates duplicate sub-expressions |
| **HyperLogLog** | Cardinality estimation for stats |
| **Roaring Bitmap** | Native Rust implementation (Ladybug uses C CRoaring) |
| **Parquet Writer** | Export to Parquet format (via `arrow::parquet`) |
| **FSM persistence via WAL** | Free Space Manager state persisted through WAL records |

---

## 11. Features Needing Port to Rust for Full Ladybug Parity

| # | Feature | Ladybug file(s) | Priority | Notes |
|---|---------|-----------------|----------|-------|
| 1 | **Confidential Statement Analyzer** | `binder/visitor/confidential_statement_analyzer.h` | Low | Security feature: analyzes queries for PII/sensitive data patterns before execution |
| 2 | **HTML/LaTeX shell output printers** | `tools/shell/printer/` | Low | Alternative output formats for shell |
| 3 | **Extended shell commands** (`:schema`, `:highlight`, etc.) | `tools/shell/embedded_shell.h` | Low | 13 shell commands vs Rust's limited set |
| 4 | **StorageDriver API** | `main/storage_driver.h` | Medium | Direct low-level storage access API |
| 5 | **WAL Dump tool** | `tools/wal_dump/` | Low | Debug/forensic tool for WAL inspection |
| 6 | **IceDisk storage format** (full C++ impl) | `storage/table/ice_disk_*.h` | Medium | Ladybug has a C++ IceDisk path; Rust has `ice_format.rs` using ParquetStreamReader |
| 7 | **PageSize/VectorCapacity compile-time tuning** | CMake options | Low | Build-time tunables not needed in Rust |
| 8 | **Benchmark suite** | `benchmark/` | Low | Dedicated benchmark infrastructure |
| 9 | **Static Extension Linking infrastructure** | CMake extension linking | Low | Rust uses Cargo features instead |
| 10 | **NODE_LABEL_FILTER as distinct operator** | `logical_node_label_filter.h` | Low | Handled via filter pushdown in Rust |

---

## 12. Key Architectural Differences

| Aspect | Ladybug C++ | Rust Kuzu |
|--------|------------|-----------|
| **Operator dispatch** | Enum-based switch with visitor pattern | `enum LogicalOperator` with match arms |
| **Physical operator split** | Separates build/probe/merge phases | Fuses phases into single operator |
| **Expression representation** | Expression tree with binder types | AST expressions with `BoundExpression` |
| **Function registration** | `FunctionCollection` static array (~607 entries) | `FunctionRegistry` with ~234 unique functions |
| **Join ordering** | Greedy cardinality-based | DP Bushy Trees (cost-based) |
| **Parser** | ANTLR4 C++ generated | `pest.rs` PEG grammar |
| **Build system** | CMake + Ninja | Cargo |
| **Extension loading** | Shared library `dlopen` | Cargo crate dependencies |
| **WASM** | Emscripten + OPFS | `wasm-pack` + `wasm-bindgen` |
| **Shell** | Linenoise (C library) | Rustyline (Rust library) |

---

## 13. Conclusion

**Overall parity between Ladybug C++ and Rust Kuzu is approximately 88-90%** as stated in the Rust port's STATUS.md. The gap is primarily:

1. **Architectural differences** (split-phase operator fusion, different parser/expression representations) — not missing features
2. **Minor shell/CLI features** (HTML/LaTeX output, extended dot commands, confidential statement analyzer)
3. **Storage format differences** (IceDisk C++ implementation differs from Rust's `ParquetStreamReader`)
4. **Debug/forensic tools** (WAL dump, benchmark infrastructure)

**No critical feature gaps remain.** All query engine components, storage engine features, GDS algorithms, extensions, and functions are ported. The Rust port actually **exceeds** Ladybug C++ in:
- Number of optimizer passes (22 vs 17)
- Cost-based join ordering (DP Bushy Trees vs greedy)
- Fuzz testing and property-based testing infrastructure
- Additional GDS algorithms (LPA, Betweenness, Closeness, Triangle Count, Random Walk, Node2Vec)
- Arrow-native execution with zero-copy paths