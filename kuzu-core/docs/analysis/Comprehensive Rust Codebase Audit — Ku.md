# Comprehensive Rust Codebase Audit — Kuzu Repository 19/07/2026

## 1. Rust Source File Inventory and Module Categorization

There are **three distinct Rust codebases** in this repository:

### A. `kuzu-core/` — Pure Rust Reimplementation (Workspace, ~230+ `.rs` files)
A full from-scratch port of the Kuzu graph database engine in Rust. Located at `kuzu-core/`, it is a Cargo workspace with **32 member crates** and **zero C++ dependencies**. Edition: Rust 2024.

### B. `tools/rust_api/` — Crate: `kuzu` (Thin wrapper over kuzu-core, ~2 `.rs` files)
A minimal public API crate that re-exports `kuzu-main` types. Provides backward-compatible `Database`/`Connection` wrappers with `Result<_, Error>` instead of `Result<_, String>`.

**Files:**
- `src/lib.rs` — Module declaration, re-exports
- `src/native.rs` — `Database`, `Connection`, `Error` wrapper types
- `build.rs` — Sets `cfg=native` flag, no C++ compilation

### C. `ladybug/tools/rust_api/` — Crate: `lbug` (FFI bindings to Ladybug C++, ~8 `.rs` files)
The **original LadybugDB** Rust bindings using `cxx` FFI to link against the C++ Ladybug library. Located at `ladybug/tools/rust_api/`.

**Files:**
- `src/lib.rs` — Module declarations, `VERSION`, `LBUG_LIBRARY_SOURCE` constants
- `src/ffi.rs` — `#[cxx::bridge]` FFI declarations (428 lines)
- `src/ffi/arrow.rs` — Arrow FFI submodule (feature-gated)
- `src/database.rs` — `Database` and `SystemConfig` (384 lines)
- `src/connection.rs` — `Connection`, `PreparedStatement` (826 lines)
- `src/value.rs` — `Value` enum and conversions (1263+ lines)
- `src/query_result.rs` — `QueryResult`, `ArrowIterator`, `CsrResult` (391 lines)
- `src/logical_type.rs` — `LogicalType` enum (315 lines)
- `src/error.rs` — `Error` enum (81 lines)
- `examples/src/main.rs` — Example usage (27 lines)
- `build.rs` — Complex C++ build orchestration (414 lines)

### D. `examples/rust/` — Crate: `kuzu-rust-example` (Simple example, ~1 `.rs` file)
- `src/main.rs` — Example using the `kuzu` crate
- `build.rs` — Minimal (sets `-rdynamic` for extensions)

---

## 2. Module Hierarchy (kuzu-core workspace)

### Workspace Cargo.toml (`kuzu-core/Cargo.toml`)
```
resolver = "2", edition = 2024
32 workspace members, each a separate crate
```

| # | Crate | Description | Main Source Files |
|---|-------|-------------|-------------------|
| 1 | **`kuzu-common`** | Foundation types, type system, vectors, filesystem, memory, serialization | `types.rs`, `vector.rs`, `data_chunk.rs`, `enums.rs`, `file_system.rs`, `memory.rs`, `task_system.rs`, `serialization.rs`, `arrow_vector.rs`, `selection.rs`, `gzip_file_system.rs`, `progress_bar.rs` |
| 2 | **`kuzu-storage`** | Storage engine — columns, tables, WAL, buffer mgr, indexing, compression, CSR | `column.rs`, `column_chunk.rs`, `table.rs`, `node_group.rs`, `wal.rs`, `wal_replayer.rs`, `buffer_manager.rs`, `page.rs`, `page_manager.rs`, `compression.rs`, `csr.rs`, `art_index.rs`, `art_key.rs`, `art_node.rs`, `index.rs`, `roaring_bitmap.rs`, `shadow_file.rs`, `local_storage.rs`, `local_wal.rs`, `free_space_manager.rs`, `checkpoint.rs`, `stats.rs`, `spiller.rs`, `lazy_scanner.rs`, `predicate.rs`, `version_info.rs`, `update_info.rs`, `undo_buffer.rs`, `hyperloglog.rs`, `ice_format.rs`, `csv_reader.rs`, `parquet_reader.rs` (cfg), `parquet_writer.rs` (cfg), `npy_reader.rs`, `vector_index.rs` |
| 3 | **`kuzu-transaction`** | MVCC transaction manager, undo records | `lib.rs` (758 lines, self-contained) |
| 4 | **`kuzu-catalog`** | Schema catalog — tables, sequences, indexes, macros, foreign tables, projected graphs | `lib.rs` (~1600 lines, self-contained) |
| 5 | **`kuzu-parser`** | Cypher query parser (pest.rs PEG) — AST, parser modules | `lib.rs`, `ast.rs`, `parser/mod.rs`, `parser/expression.rs`, `parser/dml.rs`, `parser/ddl.rs` |
| 6 | **`kuzu-binder`** | Semantic analysis, symbol resolution, type checking | `lib.rs`, `binder/mod.rs`, `binder/ddl.rs`, `binder/dml.rs`, `binder/helpers.rs`, `bound_statement.rs`, `confidential_statement_analyzer.rs` |
| 7 | **`kuzu-planner`** | Query planning — logical operators, join order | `lib.rs`, `planner.rs`, `logical_operator.rs`, `join_order.rs` |
| 8 | **`kuzu-optimizer`** | Optimization passes — tree & flat passes | `lib.rs`, `optimizer.rs`, `join_order.rs`, `passes/mod.rs`, `passes/tree/*.rs` (7 files), `passes/flat/*.rs` (13 files) |
| 9 | **`kuzu-processor`** | Physical execution — operators, expression evaluator | `lib.rs`, `physical_operator.rs`, `expression_evaluator.rs`, `physical/mod.rs`, `physical/types.rs`, `physical/common.rs`, `physical/scan_filter/*.rs` (7 files), `physical/order_aggregate/*.rs` (6 files), `physical/write_ops/*.rs` (13 files), `physical/join_ops.rs`, `physical/misc.rs`, `physical/batch_insert.rs`, `physical/index_lookup.rs`, `physical/missing_ops.rs`, `processor/mod.rs`, `processor/mapper/*.rs` (6 files), `processor/chunk_helpers.rs`, `processor/join_helpers.rs`, `processor/projection_helper.rs`, `processor/union_helpers.rs`, `processor/plan_serializer.rs` |
| 10 | **`kuzu-function`** | Function registry, scalar/aggregate evaluation | `lib.rs`, `registry.rs`, `scalar/mod.rs`, `scalar/*.rs` (18 files covering arithmetic, boolean, cast, comparison, date, hash, interval, list, map_struct, path, schema, string, union, utility, utils, blob, array), `aggregate/mod.rs` |
| 11 | **`kuzu-graph`** | Graph analytics — GDS algorithms, CSR graph representation | `lib.rs`, `graph.rs`, `algorithms.rs`, `gds/mod.rs`, `gds/compute.rs`, `gds/bfs_graph.rs`, `gds/frontier.rs`, `gds/output_writer.rs`, `gds/utils.rs` |
| 12 | **`kuzu-extension`** | Extension framework — trait, registry, context | `lib.rs` (28 lines), `registry.rs`, `context.rs` |
| 13 | **`kuzu-main`** | Public API — Database, Connection, QueryResult, ADBC | `lib.rs`, `database.rs`, `connection/mod.rs`, `connection/query.rs`, `connection/ddl.rs`, `connection/dml.rs`, `connection/copy.rs`, `connection/substitute.rs`, `connection/transaction.rs`, `connection/standalone_call.rs`, `connection/utils.rs`, `query_result.rs`, `prepared_statement.rs`, `storage_driver.rs`, `adbc.rs` |
| 14 | **`kuzu-cli`** | Interactive REPL CLI with rustyline | `src/main.rs` (733 lines) |
| 15 | **`kuzu-wasm`** | WASM bindings via wasm-bindgen | `src/lib.rs` (196 lines), `tests/wasm_test.rs` |
| 16-32 | **Extension crates** | Optional extension implementations | See below |

### Extension Crates (optional, feature-gated):
| Crate | Purpose | Files |
|-------|---------|-------|
| `kuzu-json` | JSON extension | `src/lib.rs` |
| `kuzu-fts` | Full-text search extension | `src/lib.rs` |
| `kuzu-vector` | Vector similarity search (HNSW) | `src/lib.rs`, `src/hnsw.rs` |
| `kuzu-httpfs` | HTTP/S3 filesystem support | `src/lib.rs` |
| `kuzu-duckdb` | DuckDB ATTACH support | `src/lib.rs`, `src/connection.rs`, `src/attach_helper.rs`, `src/type_converter.rs`, `src/result_converter.rs` |
| `kuzu-algo` | Graph algorithms (node2vec, random walk) | `src/lib.rs`, `src/gds/mod.rs`, `src/gds/node2vec.rs`, `src/gds/random_walk.rs` |
| `kuzu-neo4j` | Neo4j import support | `src/lib.rs` |
| `kuzu-llm` | LLM integration | `src/lib.rs` |
| `kuzu-sqlite` | SQLite ATTACH support | `src/lib.rs` |
| `kuzu-delta` | Delta Lake support | `src/lib.rs`, `src/native_reader.rs` |
| `kuzu-iceberg` | Apache Iceberg support | `src/lib.rs`, `src/native_reader.rs` |
| `kuzu-azure` | Azure Blob Storage support | `src/lib.rs`, `src/azure_storage.rs` |
| `kuzu-postgres` | PostgreSQL ATTACH | `src/lib.rs` |
| `kuzu-unity-catalog` | Unity Catalog integration | `src/lib.rs`, `src/native_client.rs` |
| `kuzu-migrate` | Database migration tool | `src/main.rs` |
| `kuzu-c` | C API bindings | `src/lib.rs` |

---

## 3. Cargo.toml Dependencies and Features

### Workspace Dependencies (`kuzu-core/Cargo.toml`):
```
serde, serde_json, tracing, tracing-subscriber, thiserror, rayon,
regex, hashbrown, ahash, bitflags, bytes, criterion, arrow (59.1.0),
parquet (59.1.0), flate2, csv, uuid, ureq, time, rust_decimal,
md-5, sha2, base64, indicatif
```

### `kuzu-main` Features (extension toggles):
```
default = []
adbc, parquet-export,
json-extension, fts-extension, vector-extension, httpfs-extension,
duckdb-extension, algo-extension, neo4j-extension, llm-extension,
sqlite-extension, delta-extension, delta-native,
iceberg-extension, iceberg-native, azure-extension, azure-native,
postgres-extension, unity-catalog-extension, unity-catalog-native
```

### `kuzu` crate (`tools/rust_api/Cargo.toml`):
- Depends on: `kuzu-main`, `kuzu-common` (path deps into workspace)
- No features
- Edition 2021, rust-version 1.81

### `lbug` crate (`ladybug/tools/rust_api/Cargo.toml`):
- Depends on: `cxx = "=1.0.138"`, `arrow` (optional), `rust_decimal`, `serde_json`, `time`, `uuid`
- Build deps: `cmake`, `cxx-build = "=1.0.138"`, `rustversion`
- Features: `default = []`, `arrow`, `extension_tests`
- Edition 2021, rust-version 1.81

---

## 4. FFI/Bindings to C++ Code

### `lbug` crate — Heavy cxx FFI (ladybug variant):
- **`build.rs`** (414 lines): Complex build script that:
  1. Downloads prebuilt `liblbug` static libs from GitHub releases or builds from source via CMake
  2. Links C++ static libraries: `lbug`, `utf8proc`, `antlr4_cypher`, `antlr4_runtime`, `re2`, `fastpfor`, `parquet`, `thrift`, `snappy`, `zstd`, `miniz`, `mbedtls`, `brotlidec`, `brotlicommon`, `lz4`, `roaring_bitmap`, `simsimd`, `yyjson`
  3. Uses `cxx-build` to generate FFI bridges
- **`src/ffi.rs`** — `#[cxx::bridge]` declaring C++ interop for:
  - `Database::new()` with 11 parameters
  - `Connection` (query, prepare, execute, interrupt, timeout)
  - `QueryResult` (columns, rows, display, arrow, CSR)
  - `PreparedStatement` (is_read_only, error_message)
  - `Value` type (create/get all primitive types, List, Struct, Map, Union, Node, Rel, RecursiveRel)
  - `LogicalType` (create/modify all type variants)
  - `StringView`, `QueryParams`, `ValueListBuilder`, `TypeListBuilder`
- **`src/ffi/arrow.rs`** — Feature-gated Arrow FFI (C Data Interface)

### `kuzu-core` — Zero C++ FFI:
- **`build.rs`** in `kuzu-main` only sets `cfg` check for WASM
- **`build.rs`** in `tools/rust_api` sets `cfg=native`
- No `extern "C"`, no `cxx`, no `#[link]` — fully native Rust
- One `unsafe` block exists in `kuzu-storage/src/buffer_manager.rs:136` (pointer cast for Frame access)

---

## 5. Fully Ported vs. Stubs/Skeletons

### Fully Ported (complete, functional Rust implementations):
| Module | Status | Evidence |
|--------|--------|----------|
| **kuzu-common** (types, vectors, file system, memory, task system, serialization) | **Complete** | 12 modules, all seemingly functional |
| **kuzu-storage** (columns, tables, WAL, buffer mgr, compression, indexing, CSR, checkpoint) | **Mostly complete** | 30+ modules with full implementations and integration tests |
| **kuzu-transaction** (MVCC transaction manager) | **Complete** | Full impl with concurrent write support, undo, drain/checkpoint |
| **kuzu-catalog** (schema management) | **Complete** | Full CRUD for tables, sequences, indexes, macros, foreign tables, projected graphs |
| **kuzu-function** (scalar/aggregate registry) | **Mostly complete** | 18 scalar function files, aggregate support, registry |
| **kuzu-main** (Database, Connection, QueryResult) | **Complete** | Full query lifecycle: parse → bind → plan → optimize → execute |
| **kuzu-main/connection** (DDL, DML, copy, transaction, standalone_call) | **Complete** | All query routing implemented |
| **kuzu-cli** (REPL shell) | **Complete** | Full CLI with rustyline, multiple output modes, CSV import/export |
| **kuzu-wasm** (WASM bindings) | **Complete** | wasm-bindgen bindings for basic query execution |

### Partially Ported (functional but simplified):
| Module | Status | Evidence |
|--------|--------|----------|
| **kuzu-parser** | **Working** | pest.rs PEG parser for Cypher — AST, expressions, DML, DDL |
| **kuzu-binder** | **Working** | Binder with DDL, DML, helpers, confidential statement analysis |
| **kuzu-planner** | **Working** | Logical plan creation, join order optimization |
| **kuzu-optimizer** | **Working** | 13 flat passes + 7 tree passes, ladybug-specific passes |
| **kuzu-processor** | **Working** | Physical plan execution, many operators implemented |
| **kuzu-graph** | **Initial** | Graph representation, BFS, GDS compute/frontier/output_writer |

### Stubs/Skeletons (marked as placeholders):
| Module | Status | Evidence |
|--------|--------|----------|
| `kuzu-json` | **Stub** | Extension registered, but likely simplified |
| `kuzu-fts` | **Stub** | Extension registered (placeholder) |
| `kuzu-vector` (HNSW) | **Initial** | HNSW implementation partially complete |
| `kuzu-httpfs` | **Stub** | Extension registered (placeholder) |
| `kuzu-duckdb` | **Stub** | "2 functions registered (placeholder)" in log |
| `kuzu-algo` | **Initial** | node2vec, random walk GDS algorithms |
| `kuzu-neo4j` | **Stub** | Extension registered |
| `kuzu-llm` | **Stub** | Extension registered |
| `kuzu-sqlite` | **Stub** | "2 functions registered (placeholder)" in log |
| `kuzu-delta` | **Stub** | Extension registered (placeholder) + native reader |
| `kuzu-iceberg` | **Stub** | "3 functions registered (placeholder)" + native reader |
| `kuzu-azure` | **Stub** | Extension registered (placeholder) + storage impl |
| `kuzu-postgres` | **Stub** | "1 function registered (placeholder)" in log |
| `kuzu-unity-catalog` | **Stub** | "Extension loaded (placeholder)" in log |
| `kuzu-migrate` | **Partial** | CLI wrapper that calls Python script |
| `kuzu-c` | **Stub** | C API bindings (minimal) |

### Placeholder/Stub Indicators Found:
- `kuzu-iceberg/src/lib.rs:302` — "Iceberg extension loaded: 3 functions registered (placeholder)"
- `kuzu-postgres/src/lib.rs:104` — "PostgreSQL extension loaded: 1 function registered (placeholder)"
- `kuzu-sqlite/src/lib.rs:129` — "SQLite extension loaded: 2 functions registered (placeholder)"
- `kuzu-duckdb/src/lib.rs:135` — "DuckDB extension loaded: 2 functions registered (placeholder)"
- `kuzu-delta/src/lib.rs:126` — "Delta extension loaded (placeholder)"
- `kuzu-azure/src/lib.rs:146` — "Azure extension loaded (placeholder)"
- `kuzu-unity-catalog/src/lib.rs:169` — "Unity Catalog extension loaded (placeholder)"
- `kuzu-main/src/adbc.rs:94` — "This is a stub for Arrow translation."
- `kuzu-main/src/connection/ddl.rs:161` — "Types are stored as catalog entries — for now, just a placeholder"
- `kuzu-graph/src/gds/output_writer.rs:269` — "Simplified: no deep cloning of bfs_graph — use empty placeholder"
- `kuzu-storage/src/index.rs:135` — "placeholder; real recovery uses L1 cache"

---

## 6. "Ladybug" Specific Modules

### In `kuzu-core` (pure Rust port):
- **`kuzu-optimizer/src/passes/flat/ladybug.rs`** — Three Ladybug-specific optimizer passes:
  1. `OrderByPushDown` — Pushes ORDER BY below UNION ALL
  2. `UnwindDedup` — Merges consecutive UNWIND operators
  3. `CountRelTable` — Replaces `ScanRel + COUNT` with CSR metadata lookup
- Referenced in `kuzu-planner/src/logical_operator.rs:62` — "Logical COUNT on a rel table — optimized via CSR metadata (Ladybug)"
- Referenced in `kuzu-optimizer/src/optimizer.rs:47-51` (passes 13-15)
- `kuzu-storage/src/ice_format.rs:15` — "Based on Ladybug's IceDiskRelTable implementation"
- `kuzu-storage/src/csr.rs:40` — "or mutable CSR structure used by Kuzu/LadybugDB"
- `kuzu-storage/benches/hybrid_eval.rs:8` — benchmark group "LadybugDB Hybrid CSR"

### In `ladybug/` directory (separate from kuzu-core):
- The `ladybug/` directory is a **standalone C++ project** (LadybugDB fork) with its own:
  - CMake build system
  - C++ source code
  - Python API (`tools/python_api/`)
  - Shell CLI (`tools/shell/`)
  - Rust API (`tools/rust_api/` = the `lbug` crate described above)
- The `ladybug/AGENTS.md` file describes build commands for the Ladybug C++ library
- The `ladybug/` directory contains a `docs/` folder with `morsel_scan.py`

---

## 7. Main Entry Points and Binary Targets

### Binary Targets:
| Binary | Location | Description |
|--------|----------|-------------|
| **`kuzu-cli`** | `kuzu-core/kuzu-cli/src/main.rs` | Interactive Cypher REPL shell with rustyline, supports `.mode`, `.import`, `.export`, `.tables`, `.schema`, `.help`, `.exit`, multi-line input, tab completion |
| **`kuzu-migrate`** | `kuzu-core/kuzu-migrate/src/main.rs` | Database migration tool (calls Python extraction script) |

### Library Entry Points:
| Library | Path | Description |
|---------|------|-------------|
| **`kuzu`** | `tools/rust_api/src/lib.rs` | Public-facing crate, re-exports `kuzu-main` types |
| **`lbug`** | `ladybug/tools/rust_api/src/lib.rs` | C++ FFI-based Ladybug bindings |
| **`kuzu-main`** | `kuzu-core/kuzu-main/src/lib.rs` | Core database API |
| **`kuzu-wasm`** | `kuzu-core/kuzu-wasm/src/lib.rs` | WASM target via wasm-bindgen |
| **`kuzu-c`** | `kuzu-core/kuzu-c/src/lib.rs` | C API |

### Additional Binaries:
| Binary | Location | Description |
|--------|----------|-------------|
| `kuzu-parser/src/bin/test_parse.rs` | Parser test binary | Tests Cypher parsing |
| `kuzu-main/src/bin/print_plan.rs` | Plan printer | Prints logical plans |
| `kuzu-main/src/bin/wal_dump.rs` | WAL dump tool | Dumps WAL contents |

---

## 8. Tests and Benchmarks

### Unit Tests (inline `#[cfg(test)]`):
- **`kuzu-common`** — tests in `types.rs`
- **`kuzu-storage`** — Extensive integration tests in `lib.rs` (8+ tests: persistence, WAL recovery, compression roundtrip, multi-node-group, stress 10k rows)
- **`kuzu-transaction`** — 10+ tests in `lib.rs` (begin_read, begin_write, commit, rollback, concurrent_writer_limit, write-write conflict, MVCC visibility)
- **`kuzu-catalog`** — 20+ tests in `lib.rs` (create/drop tables, sequences, foreign tables, macros, indexes, rename, serial sequences)
- **`kuzu-binder`** — `binder_test.rs`
- **`kuzu-parser`** — `parser_test.rs`
- **`kuzu-optimizer`** — `passes_test.rs`

### Integration Tests:
- **`kuzu-main/tests/`** — Multiple test files:
  - `integration_test.rs` — Full pipeline test
  - `test_bug.rs`, `test_boundary_values.rs`, `test_concurrency.rs`, `test_copy_to.rs`
  - `test_ddl_errors.rs`, `test_delete_set.rs`, `test_empty_tables.rs`
  - `test_fts.rs`, `test_httpfs.rs`, `test_nested_types.rs`, `test_null_handling.rs`
  - `test_unicode.rs`, `test_proptest.rs`
  - `fase_b_verification.rs`
  - `common/mod.rs` — Test helpers
- **`kuzu-migrate/tests/integration_test.rs`**
- **`kuzu-function/tests/scalar_tests.rs`**
- **`kuzu-wasm/tests/wasm_test.rs`**

### Benchmarks:
- **`kuzu-storage/benches/hybrid_eval.rs`** — "LadybugDB Hybrid CSR" benchmark
- **`kuzu-processor/benches/`** — 6 bench files:
  - `evaluate_arrow.rs`, `physical_aggregate.rs`, `physical_filter.rs`
  - `physical_hash_join.rs`, `physical_order_by.rs`, `physical_scan.rs`
- **`kuzu-main/benches/`** — 2 bench files:
  - `query_pipeline.rs`, `storage_bench.rs`

### Property-Based Testing:
- **`kuzu-main/tests/test_proptest.rs`** — Uses `proptest` crate

### Fuzz Targets:
- **`kuzu-core/fuzz/fuzz_targets/`** — 3 fuzz targets:
  - `copy_from_csv.rs`, `cypher_query.rs`, `expression_eval.rs`

### Scratch/Dev Files:
- `kuzu-core/scratch/test_processor.rs` — Unfinished test
- `kuzu-core/kuzu-main/scratch_test.rs` — Scratch file
- `kuzu-core/test_kernels.rs`, `test_arrow.rs` — Loose test files

---

## 9. Python Bindings

### For the C++ Ladybug (ladybug/ directory):
- **`ladybug/tools/python_api/`** — Full Python API using pybind11 (C++ bindings, not Rust)
  - `src_py/` — Python source files (connection.py, database.py, query_result.py, prepared_statement.py, types.py, constants.py, torch_geometric_*.py, async_connection.py)
  - `test/` — 30+ test files covering connections, queries, types, Arrow, Pandas, Polars, PyArrow, UDFs, extensions, networkx, torch_geometric, async, etc.
- **`tools/python_api/`** — Same Python API (mainline Kuzu, also C++ pybind11 based)
  - No PyO3 or maturin anywhere in the repository

### For Pure Rust kuzu-core:
- **No PyO3/maturin bindings exist** for the Rust port
- The `kuzu-c` crate provides a C API that could be used for Python bindings, but no Python wrapper is implemented yet

---

## 10. Unsafe Code and FFI Calls

### `kuzu-core` (Pure Rust) — Minimal `unsafe`:
| File | Line | Usage |
|------|------|-------|
| `kuzu-storage/src/buffer_manager.rs` | 136 | `unsafe { &*(frame as *const Frame) }` — Pointer cast to get reference to Frame from raw pointer |
| **Total: 1 `unsafe` block in the entire kuzu-core workspace** |

- **No `extern "C"` blocks** in kuzu-core
- **No `cxx` crate dependency** in kuzu-core workspace
- **No FFI declarations** anywhere in kuzu-core

### `lbug` crate (ladybug) — Heavy unsafe via cxx:
- **All FFI calls go through `#[cxx::bridge]`** which generates unsafe code internally
- **`ffi.rs`** contains ~428 lines of `unsafe extern "C++"` declarations
- **`database.rs`** — Uses `UnsafeCell<UniquePtr<ffi::Database>>` for interior mutability
- **`connection.rs`** — Uses `UnsafeCell<UniquePtr<ffi::Connection>>` for interior mutability
- **`unsafe impl Send/Sync`** for Database, Connection, QueryResult
- **`query_result.rs`** — `unsafe` calls to `arrow::ffi::from_ffi` and `arrow::ffi::from_ffi_and_data_type`

---

## Summary: Overall Port Status

| Area | Complexity | Completion Estimate |
|------|-----------|-------------------|
| **Type System & Common** | Medium | ~95% (all basic types, Value, DataChunk, vectors, filesystem, serialization) |
| **Storage Engine** | Very High | ~85% (columns, tables, buffer mgr, WAL, compression, indexing, CSR, checkpoint, spill-to-disk — some advanced features like FSM querying are simplified) |
| **Transaction Manager** | High | ~95% (MVCC with concurrent write support, checkpoint drain, auto-checkpoint worker) |
| **Catalog** | Medium | ~100% (full CRUD for all entry types) |
| **Parser** | High | ~70% (Cypher PEG parser works but may not cover all edge cases) |
| **Binder** | High | ~70% (semantic analysis, type checking, symbol resolution) |
| **Planner** | Medium | ~80% (logical plan construction, join order) |
| **Optimizer** | Medium | ~85% (13 flat passes + 7 tree passes) |
| **Processor** | Very High | ~80% (many physical operators implemented; some "missing ops" stubs) |
| **Function Registry** | Medium | ~90% (all built-in scalar functions, aggregate framework) |
| **Graph/GDS** | Medium | ~40% (basic graph representation, BFS, GDS framework) |
| **CLI** | Medium | ~90% (full REPL with rustyline, multiple output modes, tab completion) |
| **WASM** | Low | ~90% (basic bindings for query execution) |
| **Extensions** | Varies | ~10-30% per extension (most are placeholders with 1-3 functions registered) |
| **Migration Tool** | Low | ~50% (calls Python scripts) |

**Overall Rust Native Port Completion: ~75-80%**

The core query pipeline (parse → bind → plan → optimize → execute → store) works end-to-end for basic operations (CREATE NODE TABLE, CREATE REL TABLE, MATCH/RETURN, CREATE/DELETE/SET, COPY FROM/TO). The extension system is mostly placeholder/stub and will need significant work to reach feature parity with the C++ Kuzu/Ladybug.