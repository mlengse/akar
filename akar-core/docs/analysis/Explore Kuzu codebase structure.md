# Comprehensive Repository Analysis: Vela-Engineering/kuzu 17/07/2026

## 1. What Is This Project?

This is **KuzuDB** -- an **embedded property graph database** that uses the **openCypher query language** (same as Neo4j). It is an actively maintained fork of the original KuzuDB (developed at the University of Waterloo / Kuzu Inc., archived October 2025) now maintained by **Vela Partners** (San Francisco-based AI venture capital firm).

**Key Identity:**
- Repo name: `Vela-Engineering/kuzu` (this fork)
- The README states: *"Embedded graph database for AI agent memory. Built for speed. Concurrent multi-writer support. MIT licensed."*
- The primary motivation for this fork is **concurrent multi-writer support** for multi-agent AI systems (the original KuzuDB had a single-writer constraint).

**Two implementations exist side-by-side in this repo:**

| Implementation | Directory | Status |
|---|---|---|
| **C++ (Original)** | `src/` | Legacy, being superseded by Rust |
| **C++ (LadybugDB fork)** | `ladybug/` (git submodule) | Active C++ fork by LadybugDB |
| **Rust (Pure port)** | `akar-core/` | **Active development** -- 29 crates, ~66k LOC |

---

## 2. Languages & Frameworks

### Primary Languages
| Language | Where | Purpose |
|---|---|---|
| **Rust** | `akar-core/` (29 crates) | **Pure Rust reimplementation** (no FFI/cxx) of the entire database engine |
| **C++20** | `src/`, `ladybug/`, `extension/` | Original database engine + extensions |
| **C** | `src/c_api/`, `akar-core/akar-c/` | C bindings for both implementations |
| **Python** | `tools/python_api/` | Python bindings (via pybind11 for C++; native for Rust) |
| **JavaScript/TypeScript** | `tools/wasm/`, `tools/nodejs_api/` | Node.js API and WASM bindings |
| **Java** | `tools/java_api/` | Java bindings (via JNI) |
| **Go** | Referenced in README | Go bindings (external) |

### Build Systems
| System | For | Description |
|---|---|---|
| **CMake 3.15+** | C++ builds | Root `CMakeLists.txt` builds the C++ engine, tests, tools, extensions |
| **Cargo** | Rust builds | `akar-core/Cargo.toml` is a workspace of 29 Rust crates |
| **Make** | Both | `Makefile` wraps CMake targets with convenience commands |
| **setuptools** | Python | `tools/python_api/pyproject.toml` |

### Key Frameworks & Libraries

| Library | Used In | Purpose |
|---|---|---|
| **Arrow** (`arrow` crate v59.1.0) | Rust core | Internal data representation: `DataChunk`, columnar storage, expression evaluation. **Arrow-native execution** is a key differentiator |
| **Parquet** (`parquet` crate v59.1.0) | Rust storage | Persistent file format, data export/import |
| **pest.rs** | Rust `akar-parser` | PEG-based Cypher grammar parser |
| **ANTLR4** | C++ parser | C++ Cypher grammar parser |
| **pybind11** | Python API | C++/Python bridge for C++ engine |
| **wasm-bindgen** | Rust `akar-wasm` | WebAssembly bindings |
| **rayon** | Rust core | Multi-core parallelism |
| **hashbrown + ahash** | Rust core | High-performance hash tables for joins, aggregation |
| **tracing** | Rust core | Structured logging |
| **serde/serde_json** | Rust core | Serialization |
| **DuckDB** (via crate) | `akar-duckdb` | DuckDB integration (in-memory/file modes) |
| **rusqlite** | `akar-sqlite` | SQLite integration |
| **tokio-postgres** | `akar-postgres` | PostgreSQL integration |

---

## 3. Directory Structure

### Top-Level Structure
```
kuzu/
├── akar-core/          # PURE RUST IMPLEMENTATION (29 crates)
│   ├── akar-common/       # Type system, memory, serialization
│   ├── akar-storage/      # BufferManager, WAL, compression, CSR
│   ├── akar-transaction/  # MVCC, concurrency control
│   ├── akar-catalog/      # System catalog (schemas, tables)
│   ├── akar-parser/       # pest.rs PEG Cypher parser
│   ├── akar-binder/       # Semantic analysis, type inference
│   ├── akar-planner/      # Logical query plans (34 operators)
│   ├── akar-optimizer/    # 22 optimizer passes (exceeds C++)
│   ├── akar-processor/    # Physical operator execution
│   ├── akar-function/     # 234 registered functions
│   ├── akar-graph/        # CSR adjacency, GDS algorithms
│   ├── akar-extension/    # Extension framework trait
│   ├── akar-json/         # JSON extension
│   ├── akar-fts/          # Full-text search
│   ├── akar-algo/         # Graph algorithms (PageRank, Louvain, etc.)
│   ├── akar-httpfs/       # HTTP/HTTPS/S3 file system
│   ├── akar-vector/       # Vector similarity search (HNSW)
│   ├── akar-duckdb/       # DuckDB integration
│   ├── akar-sqlite/       # SQLite integration
│   ├── akar-postgres/     # PostgreSQL integration
│   ├── akar-delta/        # Delta Lake integration
│   ├── akar-iceberg/      # Apache Iceberg integration
│   ├── akar-azure/        # Azure Blob Storage
│   ├── akar-unity-catalog/ # Unity Catalog integration
│   ├── akar-neo4j/        # Neo4j Bolt protocol
│   ├── akar-llm/          # LLM integration
│   ├── akar-main/         # PUBLIC API (Database, Connection)
│   ├── akar-cli/          # Interactive Cypher shell
│   ├── akar-wasm/         # WASM bindings
│   ├── akar-c/            # C FFI bindings
│   ├── akar-migrate/      # C++ -> Rust migration tool
│   └── fuzz/              # Fuzzing targets
│
├── src/               # C++ ORIGINAL ENGINE (legacy)
│   ├── include/          # C++ headers
│   ├── main/             # Database, Connection
│   ├── storage/          # BufferManager, WAL, indices
│   ├── processor/        # Physical operators
│   ├── planner/          # Logical planning
│   ├── parser/           # ANTLR4 Cypher parser
│   ├── binder/           # Semantic analysis
│   ├── optimizer/        # Query optimization
│   ├── catalog/          # System catalog
│   ├── common/           # Types, utilities
│   ├── function/         # Built-in functions
│   ├── graph/            # Graph algorithms
│   ├── transaction/      # Transaction management
│   ├── extension/        # Extension framework
│   ├── expression_evaluator/ # Expression evaluation
│   └── c_api/            # C API bindings
│
├── ladybug/           # LADYBUGDB C++ FORK (git submodule)
│                       # Repo: github.com/mlengse/ladybug
│   ├── src/             # Same structure as src/ above
│   ├── CMakeLists.txt   # Builds Ladybug v0.18.0
│   └── ...              # Mirror of standard Kuzu structure
│
├── extension/         # C++ Extensions
│   ├── json/           # JSON extension
│   ├── fts/            # Full-text search
│   ├── vector/         # Vector search
│   ├── httpfs/         # HTTP file system
│   ├── duckdb/         # DuckDB scanner
│   ├── postgres/       # PostgreSQL scanner
│   ├── sqlite/         # SQLite scanner
│   ├── delta/          # Delta Lake scanner
│   ├── iceberg/        # Apache Iceberg scanner
│   ├── azure/          # Azure storage
│   ├── unity_catalog/  # Unity Catalog
│   ├── neo4j/          # Neo4j connector
│   ├── algo/           # Graph algorithms
│   └── llm/            # LLM functions
│
├── tools/             # LANGUAGE BINDINGS & TOOLS
│   ├── python_api/     # Python (C++ pybind11)
│   ├── rust_api/       # Rust API crate (wraps akar-main)
│   ├── java_api/       # Java JNI bindings
│   ├── nodejs_api/     # Node.js bindings
│   ├── wasm/           # WebAssembly (Emscripten C++ version)
│   ├── shell/          # Interactive shell (CLI)
│   ├── benchmark/      # C++ benchmark tool
│   └── stress/         # Stress testing
│
├── benchmark/         # BENCHMARK QUERIES & SCRIPTS
│   ├── queries/        # .benchmark files for various datasets
│   ├── click/          # ClickBench analytical benchmarks
│   ├── lsqb/           # LSQB graph benchmark
│   └── serialize/     # Dataset serialization helpers
│
├── dataset/           # TEST DATASETS (68 subdirs)
│   ├── tinysnb/        # Tiny social network benchmark
│   ├── ldbc-sf01/      # LDBC social network SF-0.1
│   ├── lsqb-sf01/      # LSQB dataset
│   └── ... (68 total)
│
├── third_party/       # VENDORED C/C++ LIBRARIES (28 packages)
│   ├── antlr4_cypher/  # ANTLR4 Cypher grammar
│   ├── antlr4_runtime/ # ANTLR4 runtime
│   ├── brotli/         # Compression
│   ├── zstd/           # Compression
│   ├── lz4/            # Compression
│   ├── snappy/         # Compression
│   ├── roaring_bitmap/ # Bitmap compression
│   ├── fastpfor/       # Integer compression
│   ├── parquet/        # Parquet reader
│   ├── thrift/         # Thrift (Parquet dependency)
│   ├── miniz/          # Zip support
│   ├── re2/            # Regex engine
│   ├── mbedtls/        # TLS (for HTTPFS)
│   ├── yyjson/         # JSON parsing
│   ├── simsimd/        # SIMD similarity search
│   ├── fast_float/     # Fast float parsing
│   ├── utf8proc/       # Unicode processing
│   ├── cppjieba/       # Chinese text segmentation
│   ├── pybind11/       # Python/C++ bridge
│   ├── nlohmann_json/  # JSON for C++
│   ├── spdlog/         # Logging
│   ├── alp/            # Adaptive lossless compression
│   ├── pcg/            # Random number generation
│   ├── pyparse/        # Python parsing
│   ├── glob/           # File globbing
│   ├── httplib/        # HTTP client
│   └── taywee_args/    # Argument parsing
│
├── test/              # C++ test suite (GoogleTest)
├── cmake/             # CMake templates & helpers
├── scripts/           # Build/CI utility scripts
└── .github/           # CI/CD workflows (GitHub Actions)
```

---

## 4. The Dual-Implementation Architecture

### The Rust Implementation (`akar-core/`) -- PRIMARY FOCUS

The Rust port is a **from-scratch pure Rust rewrite** (no FFI, no `cxx` bridge) of the Kuzu C++ engine. Key characteristics:

- **29 crate workspace** with explicit dependency graph
- **Edition 2024 Rust**
- **~66k lines of Rust code**
- **1099+ tests passing** (as of July 2026)
- **Arrow-native execution** -- data is stored and processed as Apache Arrow arrays, enabling vectorized compute kernels
- **Feature-gated extensions** -- all 15 extension crates are optional Cargo features
- **C++ parity achieved** -- `MATCH ... WHERE ... RETURN COUNT(*)` benchmark: 397 µs Rust vs 400 µs C++

**Rust Crate Dependency Graph** (simplified):
```
akar-cli ─┬─ akar-main ─┬─ akar-common (types, arrow)
           │              ├─ akar-storage ─┬─ akar-common
           │              │                 ├─ akar-catalog
           │              │                 ├─ akar-transaction
           │              │                 └─ akar-vector
           │              ├─ akar-parser
           │              ├─ akar-binder
           │              ├─ akar-planner
           │              ├─ akar-optimizer
           │              ├─ akar-processor ─┬─ akar-storage
           │              │                    ├─ akar-planner
           │              │                    ├─ akar-function
           │              │                    └─ akar-parser
           │              ├─ akar-function
           │              ├─ akar-graph
           │              ├─ akar-catalog
           │              ├─ akar-transaction
           │              ├─ akar-extension
           │              └─ [15 optional extension crates]
           │
           └─ akar-common
akar-c ─┬─ akar-main
         └─ akar-common
akar-wasm ─┬─ akar-main
            ├─ akar-common
            └─ wasm-bindgen
akar-migrate ─┬─ akar-main
               ├─ akar-storage
               └─ akar-common
```

### The C++ Implementation (`src/`) -- LEGACY

The `src/` directory contains the original C++ kernel matching the traditional Kuzu architecture:
- ANTLR4-based Cypher parser
- ValueVector-based execution (not Arrow arrays)
- 28 vendored third-party C/C++ libraries
- CMake build system with extensive configuration options

### The LadybugDB Submodule (`ladybug/`)

The `ladybug/` directory is a **git submodule** pointing to `https://github.com/mlengse/ladybug` (a fork by Arun Sharma). This is the LadybugDB C++ fork. It has its own independent `.git` directory, CI workflows, and build system (Ladybug v0.18.0). The main README references LadybugDB as *"General-purpose KuzuDB continuation"* alongside this Vela fork.

Notable: `ladybug/` has a near-identical structure to the main `src/` directory -- same subdirectory layout (binder, catalog, common, function, graph, main, optimizer, parser, planner, processor, storage, transaction, etc.). It also has its own `tools/`, `extension/`, `dataset/`, `benchmark/`, and `third_party/` directories.

---

## 5. Benchmark & Performance Comparison Files

### Rust Benchmarks (criterion-based)
| File | Content |
|---|---|
| `akar-core/BENCHMARK_COMPARISON.md` | **Full 504-line comparison** between Rust and C++ performance |
| `akar-core/akar-processor/` | 6 benchmark suites: `physical_scan`, `physical_filter`, `physical_hash_join`, `physical_order_by`, `physical_aggregate`, `evaluate_arrow` |
| `akar-core/akar-main/` | 2 benchmark suites: `query_pipeline`, `storage_bench` |
| `cpp_bench.json` | C++ benchmark configuration |

### Benchmark Datasets (in `benchmark/queries/`)
- `micro/` -- Micro-benchmarks
- `click/` -- ClickBench analytical workload
- `ldbc-sf100/` -- LDBC social network SF-100
- `soc-livejournal/` -- Social network analysis
- `graph500-27/` -- Graph500 benchmark
- `datagen-sf10k/` -- Synthetic data generator

### Key Performance Results (from BENCHMARK_COMPARISON.md)
- **Rust at parity with C++**: 397 µs vs 400 µs for `MATCH (p) WHERE p.age > 30 RETURN COUNT(p)` on 10k rows
- **Arrow-native filter**: 10-24x faster than per-row Value boxing
- **Scan throughput**: ~38M rows/second (10k rows, 4 columns)
- **Filter throughput**: 18.3 µs for pass-all (24x improvement over legacy)
- **Hash Join**: 137 µs for 100x100 matching keys
- **Order By**: 983 µs for single-key 10k sorting
- **Aggregate COUNT**: 158 µs for 10k rows

---

## 6. References to Vela, LadybugDB, and "drop-in replacement"

### Vela Partners References
- **README.md line 1**: `"KuzuDB — Maintained by Vela Partners"`
- **README.md line 5**: `"This fork, maintained by Vela Partners... extends the original with concurrent write support"`
- **`.github/workflows/build-extension-registry.yml`**: Vela extension registry pages
- **`akar-core/STATUS.md`**: Extensive references to "Vela C++" as the baseline

### LadybugDB References
- **`ladybug/`**: Full git submodule for LadybugDB C++ fork
- **README.md line 172**: `"LadybugDB | Arun Sharma | General-purpose KuzuDB continuation"`
- **`akar-core/STATUS.md`**: Compares Rust implementation against "LadybugDB C++" throughout
- **Tools references**: `@ladybugdb/wasm-core` npm package, `lbug` Rust crate, `ladybug` Python package

### "Drop-in replacement" References
- **`akar-core/implementation_plan.md` line 78**: `"P28 — Drop-in replacement — migration tool, CLI"` -- explicitly listed as a planned deliverable
- **`akar-core/MIGRATION.md`**: Full guide for migrating from C++ (Vela/LadybugDB) to Rust as a drop-in replacement

---

## 7. Serialization/Deserialization

**No protobuf or flatbuffers were found** anywhere in the repository. The serialization strategy is:

| Purpose | Format/Tool | Location |
|---|---|---|
| **Internal data representation** | **Apache Arrow** (`arrow` crate) | `DataChunk`, `ArrayRef`, used across all physical operators |
| **Persistent file storage** | **Parquet** | `COPY FROM 'file.parquet'`, `EXPORT DATABASE` |
| **WAL (Write-Ahead Log)** | Custom binary format via `std::io::Write` | `akar-core/akar-storage/src/wal.rs` -- binary records with type tags |
| **JSON data interchange** | `serde_json` / `yyjson` | Python API serialization, dataset loading |
| **Configuration/state** | JSON | `cpp_bench.json`, various test configs |
| **Arrow IPC format** | `arrow::ipc` | In-memory data transfer between operators |

The WAL format (`WALRecord` enum) uses a custom binary encoding:
```rust
pub enum WALRecord {
    Insert { table_id: u64, data: Vec<u8> },
    Delete { table_id: u64, row_id: u64 },
    Update { table_id: u64, row_id: u64, column: u32, data: Vec<u8> },
    UpdateFsm { page_idx: u64, is_free: bool },
    ColumnWrite { table_id: u64, col_id: u32, page_id: u64, data: Vec<u8> },
    LocalWALData { data: Vec<u8> },
    Commit { transaction_id: u64 },
    Rollback { transaction_id: u64 },
    Checkpoint,
    // 6 DDL variants
    CreateTable, DropTable, AlterTable, CreateIndex, DropIndex, CreateSequence
}
```

---

## 8. Dependency Graph & Architecture Summary

### Query Pipeline (Rust)
```
[Cypher Text]
     │
     ▼
┌────────────┐    ┌──────────┐    ┌──────────┐    ┌──────────────┐    ┌──────────────┐
│  Parser    │───▶│  Binder  │───▶│  Planner │───▶│  Optimizer   │───▶│  Processor   │
│ (pest.rs)  │    │(Catalog)  │    │(Logical) │    │ (22 passes)  │    │ (Physical)   │
│ 58 stmts   │    │(Types)   │    │34 ops    │    │ FilterPush   │    │ 46 operators │
└────────────┘    └──────────┘    └──────────┘    └──────────────┘    └──────────────┘
                                                                              │
                                                                              ▼
                                                                        Arrow DataChunks
```

### Storage Architecture
```
┌──────────────────────┐
│    BufferManager     │  Clock eviction, page pin/unpin
├──────────────────────┤
│    PageManager       │  Page alloc/dealloc via Free Space Manager
├──────────────────────┤
│    WAL + Replayer    │  Crash recovery, 6 DDL record types
├──────────────────────┤
│    Undo Buffer       │  Rollback via undo record replay
├──────────────────────┤
│    Indices           │  ART (Node4/16/48/256), HNSW (vector), Hash
├──────────────────────┤
│    Compression       │  Constant, Boolean, Dictionary encoding
├──────────────────────┤
│    Zone Maps         │  Predicate pushdown via ColumnChunkStats
└──────────────────────┘
```

### Transaction System
- **MVCC** with Serializable ACID isolation
- **AUTO/MANUAL** transaction modes
- **Dynamic table-level locking** for concurrent writes
- **OCC conflict detection**
- **Multi-writer** via `AtomicBool` + `Condvar`

### External Integrations (Extensions)
| Integration | Mechanism | Mode |
|---|---|---|
| DuckDB | `duckdb` Rust crate | In-memory / file |
| SQLite | `rusqlite` | Native |
| PostgreSQL | `tokio-postgres` | Bolt protocol |
| Neo4j | `bolt` protocol | Remote |
| Delta Lake | DuckDB delegation | Via DuckDB |
| Apache Iceberg | DuckDB delegation | Via DuckDB |
| Azure Blob | DuckDB delegation | Via DuckDB |
| Unity Catalog | DuckDB delegation | Via DuckDB |
| HTTPFS | Native Rust | HTTP/HTTPS/S3 |

### Language Bindings Architecture
```
                    ┌──────────────┐
                    │  akar-core   │
                    │  (Rust)      │
                    └──────┬───────┘
                           │
          ┌────────────────┼────────────────┐
          │                │                │
    ┌─────▼──────┐  ┌──────▼───────┐  ┌────▼──────┐
    │ tools/     │  │ tools/       │  │ tools/    │
    │ rust_api   │  │ python_api   │  │ java_api  │
    │ (Cargo)    │  │ (pybind11)   │  │ (JNI)     │
    └────────────┘  └──────────────┘  └───────────┘
                              │
                    ┌─────────▼──────────┐
                    │ tools/wasm         │
                    │ (Emscripten C++)   │
                    └────────────────────┘
```

---

## Summary

This repository contains **two parallel graph database implementations** of KuzuDB:

1. **C++ Original** (`src/`) + **LadybugDB fork** (`ladybug/` submodule) -- the traditional C++ implementation with 28 vendored C/C++ libraries, CMake build, ANTLR4 grammar, and broad ecosystem support.

2. **Pure Rust Port** (`akar-core/`) -- a complete from-scratch rewrite in Rust 2024 with 29 crates, Arrow-native execution (10-24x faster filters), 15+ extensions, 22 optimizer passes, 234 registered functions, 1099+ tests, and performance at parity with C++ (397 µs vs 400 µs for a representative query). This is the **primary active development focus**.

The unique value proposition of this Vela-Engineering fork is **concurrent multi-writer support** for multi-agent AI systems, which was a limitation of the original single-writer KuzuDB. The Rust port is explicitly designed as a **drop-in replacement** for the C++ version, with a dedicated migration tool (`akar-migrate`) that exports C++ databases via Parquet and re-imports them into the Rust engine.

**No protobuf or flatbuffers** are used -- the project relies on **Apache Arrow** for internal data representation, **Parquet** for persistent storage, and custom binary encoding for the WAL.
