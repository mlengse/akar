# Explore codebase structure 19/07/2026

## Complete Codebase Structure: `C:\Users\anjan\dev\memory\kuzu`

This repository contains **two independent projects** that share a common lineage:

1. **KuzuDB** (original C++ project, with Rust re-implementation under `kuzu-core/`)
2. **LadybugDB** (a fork/continuation under `ladybug/`)

---

## 1. TOP-LEVEL DIRECTORY TREE (depth 3-4)

```
C:\Users\anjan\dev\memory\kuzu/
├── .cargo/                                          # Empty cargo config dir
├── .clang-format
├── .clang-tidy
├── .clang-tidy-analyzer
├── .claude/
├── .git/
├── .github/
│   ├── agents/
│   ├── CODEOWNERS
│   ├── copilot-instructions.md
│   ├── dependabot.yml
│   ├── docs/
│   ├── ISSUE_TEMPLATE/
│   ├── prompts/
│   ├── pull_request_template.md
│   ├── skills/
│   └── workflows/
│       ├── build-and-release.yml                    # Build + release pipeline
│       ├── build-extension-registry.yml
│       ├── disabled/
│       ├── fuzz-ci.yml                              # cargo-fuzz CI
│       ├── lcov_exclude
│       ├── rust-ci.yml                              # Rust workspace CI
│       └── rust-release.yml                         # Rust release workflow
├── .gitignore
├── .gitmodules
├── .idea/
│   ├── .gitignore
│   ├── inspectionProfiles/
│   ├── kuzu.iml
│   ├── modules.xml
│   ├── vcs.xml
│   └── workspace.xml
├── .lcovrc
├── benchmark/
│   ├── benchmark_runner.py
│   ├── click/
│   ├── lsqb/
│   ├── queries/
│   ├── serialize.cypher
│   ├── serializer.py
│   └── version.py
├── build/
│   └── release/
├── CLA.md
├── cmake/
│   └── templates/
├── CMakeLists.txt                                   # Root C++ CMake (Kuzu v0.12.0)
├── CODE_OF_CONDUCT.md
├── CONTRIBUTING.md
├── cpp_bench.json
├── dataset/                                         # 68+ test datasets
│   ├── all_types/
│   ├── tinysnb/
│   ├── ldbc-sf01/
│   ├── lsqb-sf01/
│   ├── snap/
│   ├── parquet/
│   └── ...
├── examples/
│   ├── c/
│   ├── cpp/
│   ├── README.md
│   └── rust/                                        # Rust example (uses kuzu crate)
├── extension/
│   ├── algo/
│   ├── azure/
│   ├── CMakeLists.txt
│   ├── delta/
│   ├── duckdb/
│   ├── extension_config.cmake
│   ├── fts/
│   ├── httpfs/
│   ├── iceberg/
│   ├── json/
│   ├── llm/
│   ├── neo4j/
│   ├── postgres/
│   ├── sqlite/
│   ├── unity_catalog/
│   └── vector/
├── init.cypher
├── kuzu-core/                                       # RUST REWRITE (pure Rust embedded graph DB)
│   ├── .cargo/
│   ├── .claude/
│   ├── .gitignore
│   ├── BENCHMARK_COMPARISON.md
│   ├── Cargo.lock
│   ├── Cargo.toml                                   # Workspace root (30+ crates)
│   ├── check_env.bat
│   ├── clippy.toml
│   ├── CONTRIBUTING.md
│   ├── docs/
│   ├── errors.txt
│   ├── flamegraph.svg
│   ├── fuzz/                                        # fuzz targets
│   │   ├── Cargo.toml
│   │   └── fuzz_targets/
│   ├── implementation_plan.md
│   ├── kuzu-algo/
│   ├── kuzu-azure/
│   ├── kuzu-binder/
│   ├── kuzu-c/                                      # C FFI bindings
│   ├── kuzu-catalog/
│   ├── kuzu-cli/                                    # CLI binary
│   ├── kuzu-common/                                 # Core common types & utils
│   ├── kuzu-delta/
│   ├── kuzu-duckdb/
│   ├── kuzu-extension/
│   ├── kuzu-fts/                                    # Full-text search
│   ├── kuzu-function/                               # Built-in functions
│   ├── kuzu-graph/
│   ├── kuzu-httpfs/
│   ├── kuzu-iceberg/
│   ├── kuzu-json/
│   ├── kuzu-llm/
│   ├── kuzu-main/                                   # Core library entry point
│   ├── kuzu-migrate/
│   ├── kuzu-neo4j/
│   ├── kuzu-optimizer/
│   ├── kuzu-parser/                                 # PEG parser (pest)
│   ├── kuzu-planner/
│   ├── kuzu-postgres/
│   ├── kuzu-processor/                              # Query execution engine
│   ├── kuzu-sqlite/
│   ├── kuzu-storage/                                # Disk-based columnar storage
│   ├── kuzu-transaction/                            # ACID transactions
│   ├── kuzu-unity-catalog/
│   ├── kuzu-vector/                                 # Vector index
│   ├── kuzu-wasm/                                   # WASM bindings
│   ├── libdp_bushy.rlib
│   ├── MIGRATION.md
│   ├── profile.json.gz
│   ├── README.md
│   ├── RELEASE.md
│   ├── rust_bench.txt
│   ├── rustfmt.toml
│   ├── scratch/
│   ├── STATUS.md
│   ├── target/
│   ├── target-wsl/
│   ├── test_arrow.rs
│   └── test_kernels.rs
├── ladybug/                                         # LADYBUGDB FORK (C++ continuation)
│   ├── .clang-format
│   ├── .clang-format-ignore
│   ├── .clang-tidy
│   ├── .clang-tidy-analyzer
│   ├── .git/
│   ├── .github/
│   │   └── workflows/
│   │       ├── benchmark-workflow.yml
│   │       ├── build-and-deploy-extension.yml
│   │       ├── build-and-deploy.yml
│   │       ├── build-extensions.yml
│   │       ├── ci-musl-test-workflow.yml
│   │       ├── ci-workflow.yml                      # Main CI (433 lines)
│   │       ├── codeql.yml
│   │       ├── deploy-extension.yml
│   │       ├── gen-docs.yml
│   │       ├── get-extensions-from-ghcr.yml
│   │       ├── java-workflow.yml
│   │       ├── lcov_exclude
│   │       ├── lsqb-benchmark-workflow.yml
│   │       ├── nodejs-workflow.yml
│   │       ├── precompiled-bin-workflow.yml
│   │       ├── purge-extension.yml
│   │       ├── python-wheel-workflow.yml
│   │       ├── release-artifacts.yml
│   │       ├── wasm-workflow.yml
│   │       └── zizmor.yml
│   ├── .gitignore
│   ├── .gitmodules
│   ├── .lcovrc
│   ├── AGENTS.md                                    # Build/test instructions for agents
│   ├── benchmark/
│   ├── build/
│   │   └── release/
│   ├── cmake/
│   │   └── templates/
│   ├── CMakeLists.txt                               # Root CMake (Ladybug v0.18.0, 561 lines)
│   ├── CONTRIBUTING.md
│   ├── dataset/                                     # 72+ test datasets
│   ├── docs/                                        # Developer docs
│   │   ├── build_tips.md
│   │   ├── cpp_style.md
│   │   ├── extensions.md
│   │   ├── grammar.md
│   │   ├── icebug-disk.md
│   │   ├── incidents/
│   │   ├── python.md
│   │   ├── shell.md
│   │   └── testing.md
│   ├── examples/
│   ├── extension/                                   # Extension modules
│   │   ├── adbc/
│   │   ├── algo/
│   │   ├── azure/
│   │   ├── CMakeLists.txt
│   │   ├── delta/
│   │   ├── duckdb/
│   │   ├── extension_config.cmake
│   │   ├── fts/
│   │   ├── httpfs/
│   │   ├── iceberg/
│   │   ├── json/
│   │   ├── llm/
│   │   ├── neo4j/
│   │   ├── postgres/
│   │   ├── sqlite/
│   │   ├── unity_catalog/
│   │   └── vector/
│   ├── LICENSE
│   ├── logo/
│   ├── Makefile                                     # Build frontend (413 lines)
│   ├── pixi.toml                                    # Conda/pixi env config
│   ├── README.md
│   ├── scripts/
│   ├── SECURITY.md
│   ├── security/
│   ├── src/                                         # C++ source = mirror of root src/
│   │   ├── antlr4/
│   │   ├── binder/
│   │   ├── c_api/
│   │   ├── catalog/
│   │   ├── CMakeLists.txt
│   │   ├── common/
│   │   ├── expression_evaluator/
│   │   ├── extension/
│   │   ├── function/
│   │   ├── graph/
│   │   ├── include/
│   │   ├── main/
│   │   ├── optimizer/
│   │   ├── parser/
│   │   ├── planner/
│   │   ├── processor/
│   │   ├── storage/
│   │   └── transaction/
│   ├── test/
│   ├── third_party/
│   │   ├── alp/
│   │   ├── antlr4_cypher/
│   │   ├── antlr4_runtime/
│   │   ├── brotli/
│   │   ├── CMakeLists.txt
│   │   ├── cppjieba/
│   │   ├── fast_float/
│   │   ├── fastpfor/
│   │   ├── glob/
│   │   ├── httplib/
│   │   ├── lz4/
│   │   ├── mbedtls/
│   │   ├── miniz/
│   │   ├── parquet/
│   │   ├── pcg/
│   │   ├── pkg/
│   │   ├── ports/
│   │   ├── pybind11/
│   │   ├── pyparse/
│   │   ├── re2/
│   │   ├── roaring_bitmap/
│   │   ├── simsimd/
│   │   ├── snappy/
│   │   ├── spdlog/
│   │   ├── taywee_args/
│   │   ├── thrift/
│   │   ├── utf8proc/
│   │   ├── versions.txt
│   │   ├── yyjson/
│   │   └── zstd/
│   └── tools/
│       ├── benchmark/
│       ├── CMakeLists.txt
│       ├── dev/
│       ├── java_api/
│       ├── nodejs_api/
│       ├── python_api/
│       ├── rust_api/                                # `lbug` crate (CXX FFI to C++)
│       ├── shell/
│       ├── wal_dump/
│       └── wasm/
├── LICENSE
├── Makefile                                          # Build frontend (Kuzu C++, 388 lines)
├── MIGRATION.md
├── README.md                                         # Vela Partners fork README
├── scripts/
│   ├── antlr4/
│   ├── check-include-guards.sh
│   ├── check-no-std-assert.sh
│   ├── collect-extensions.py
│   ├── collect-single-file-header.py
│   ├── export-dbs.py
│   ├── export-import-test.py
│   ├── extension_version.py
│   ├── extension/
│   ├── generate_binary_demo.sh
│   ├── generate_binary_ldbc-sf01.sh
│   ├── generate_binary_tinysnb.sh
│   ├── generate-cpp-docs/
│   ├── generate-tinysnb.py
│   ├── get-clangd-diagnostics.py
│   ├── headers.txt
│   ├── migrate-kuzu-db.py
│   ├── multiplatform-test-helper/
│   ├── pip-package/                                 # Python wheel packaging
│   ├── preserve-extension-registry.py
│   ├── run-clang-format.py
│   ├── setup-extension-repo.py
│   ├── simd-dispatch-test.cypher
│   ├── test-simsimd-dispatch.py
│   └── update-nightly-build-version.py
├── src/                                              # C++ SOURCE ROOT (Kuzu original)
│   ├── antlr4/
│   ├── binder/
│   │   ├── bind/
│   │   ├── ddl/
│   │   ├── expression/
│   │   ├── query/
│   │   ├── rewriter/
│   │   └── visitor/
│   ├── c_api/                                        # C API (11 files)
│   ├── catalog/
│   │   └── catalog_entry/
│   ├── CMakeLists.txt                                # Builds kuzu (static) + kuzu_shared (dynamic)
│   ├── common/
│   │   ├── arrow/
│   │   ├── data_chunk/
│   │   ├── enums/
│   │   ├── exception/
│   │   ├── file_system/
│   │   ├── serializer/
│   │   ├── signal/
│   │   ├── task_system/
│   │   ├── types/
│   │   └── vector/
│   ├── expression_evaluator/
│   ├── extension/
│   ├── function/                                     # 42 files (aggs, arithmetic, array, cast, etc.)
│   │   ├── aggregate/
│   │   ├── arithmetic/
│   │   ├── array/
│   │   ├── cast/
│   │   ├── date/
│   │   ├── export/
│   │   ├── gds/
│   │   ├── internal_id/
│   │   ├── list/
│   │   ├── map/
│   │   ├── path/
│   │   ├── pattern/
│   │   ├── sequence/
│   │   ├── string/
│   │   ├── struct/
│   │   ├── table/
│   │   ├── timestamp/
│   │   ├── union/
│   │   ├── utility/
│   │   └── uuid/
│   ├── graph/
│   ├── include/                                      # Public C++ headers (mirrors src/ structure)
│   │   ├── binder/
│   │   ├── c_api/
│   │   ├── catalog/
│   │   ├── common/
│   │   ├── expression_evaluator/
│   │   ├── extension/
│   │   ├── function/
│   │   ├── graph/
│   │   ├── main/
│   │   ├── optimizer/
│   │   ├── parser/
│   │   ├── planner/
│   │   ├── processor/
│   │   ├── storage/
│   │   └── transaction/
│   ├── main/                                         # Database, Connection, QueryResult
│   ├── optimizer/                                    # 16 optimizer passes
│   ├── parser/
│   │   ├── antlr_parser/
│   │   ├── expression/
│   │   ├── transform/
│   │   └── visitor/
│   ├── planner/
│   │   ├── join_order/
│   │   ├── operator/
│   │   └── plan/
│   ├── processor/                                    # Query execution engine
│   │   ├── map/
│   │   ├── operator/
│   │   │   ├── aggregate/
│   │   │   ├── ddl/
│   │   │   ├── hash_join/
│   │   │   ├── intersect/
│   │   │   ├── macro/
│   │   │   ├── order_by/
│   │   │   ├── persistent/ (csv, npy, parquet r/w)
│   │   │   ├── scan/
│   │   │   ├── simple/
│   │   │   └── table_scan/
│   │   └── result/
│   ├── storage/                                      # Columnar disk storage
│   │   ├── buffer_manager/
│   │   ├── compression/
│   │   ├── index/
│   │   ├── local_storage/
│   │   ├── predicate/
│   │   ├── stats/
│   │   ├── table/
│   │   └── wal/
│   └── transaction/
├── test/
│   ├── answers/
│   ├── api/
│   ├── binder/
│   ├── c_api/
│   ├── CMakeLists.txt
│   ├── common/
│   ├── copy/
│   ├── graph_test/
│   ├── gtest/
│   ├── include/
│   ├── optimizer/
│   ├── planner/
│   ├── runner/
│   ├── statements/
│   ├── storage/
│   ├── test_files/
│   ├── test_helper/
│   ├── test_runner/
│   └── transaction/
├── third_party/                                      # 28 bundled C/C++ libraries
│   ├── alp/
│   ├── antlr4_cypher/
│   ├── antlr4_runtime/
│   ├── brotli/
│   ├── CMakeLists.txt
│   ├── cppjieba/
│   ├── fast_float/
│   ├── fastpfor/
│   ├── glob/
│   ├── httplib/
│   ├── lz4/
│   ├── mbedtls/
│   ├── miniz/
│   ├── nlohmann_json/
│   ├── parquet/
│   ├── pcg/
│   ├── pybind11/
│   ├── pyparse/
│   ├── re2/
│   ├── roaring_bitmap/
│   ├── simsimd/
│   ├── snappy/
│   ├── spdlog/
│   ├── taywee_args/
│   ├── thrift/
│   ├── utf8proc/
│   ├── yyjson/
│   └── zstd/
└── tools/
    ├── benchmark/
    ├── CMakeLists.txt
    ├── java_api/
    ├── nodejs_api/
    ├── python_api/                                  # Python bindings (pybind11)
    ├── rust_api/                                    # `kuzu` crate (Rust bindings to C++)
    ├── shell/                                       # Interactive CLI shell
    ├── stress/
    └── wasm/                                        # WebAssembly bindings
```

---

## 2. ALL Cargo.toml FILES (Rust Project Structure)

### 2a. Kuzu-Core Workspace (`kuzu-core/Cargo.toml`)

**Workspace root** with 30 member crates. All share workspace version, edition 2024, MIT license.

```toml
[workspace]
resolver = "2"
members = [
    "kuzu-common", "kuzu-storage", "kuzu-transaction", "kuzu-catalog",
    "kuzu-parser", "kuzu-binder", "kuzu-planner", "kuzu-optimizer",
    "kuzu-processor", "kuzu-function", "kuzu-graph", "kuzu-extension",
    "kuzu-json", "kuzu-fts", "kuzu-vector", "kuzu-httpfs",
    "kuzu-duckdb", "kuzu-algo", "kuzu-neo4j", "kuzu-llm", "kuzu-sqlite",
    "kuzu-delta", "kuzu-iceberg", "kuzu-azure", "kuzu-postgres",
    "kuzu-unity-catalog", "kuzu-main", "kuzu-cli", "kuzu-wasm", "kuzu-migrate", "kuzu-c",
]

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "MIT"
repository = "https://github.com/kuzudb/kuzu"
description = "Kùzu graph database — pure Rust embedded graph database"
```

Key workspace dependencies: `serde`, `serde_json`, `tracing`, `rayon`, `regex`, `hashbrown`, `ahash`, `arrow` (59.1.0), `parquet` (59.1.0), `csv`, `uuid`, `ureq`, `time`, `rust_decimal`, `md-5`, `sha2`, `base64`, `indicatif`.

### 2b. Kuzu-Core Crate Dependencies Graph

```
kuzu-common         ← dependency-free (serde, rayon, arrow, flate2, indicatif)
kuzu-transaction    ← dependency-free (tracing only)
kuzu-catalog        ← depends on kuzu-common
kuzu-vector         ← depends on kuzu-common, kuzu-function, kuzu-extension
kuzu-parser         ← depends on pest/pest_derive
kuzu-binder         ← depends on kuzu-common, kuzu-parser, kuzu-catalog
kuzu-function       ← depends on kuzu-common
kuzu-graph          ← depends on kuzu-common, kuzu-storage
kuzu-planner        ← depends on kuzu-common, kuzu-binder, kuzu-parser, kuzu-catalog
kuzu-optimizer      ← depends on kuzu-common, kuzu-planner, kuzu-storage, kuzu-binder, kuzu-parser
kuzu-storage        ← depends on kuzu-common, kuzu-catalog, kuzu-transaction, kuzu-vector
kuzu-extension      ← depends on kuzu-common, kuzu-function, kuzu-catalog
kuzu-fts            ← depends on kuzu-common, kuzu-function, kuzu-extension
kuzu-json           ← depends on kuzu-common, kuzu-function, kuzu-extension
kuzu-httpfs         ← depends on kuzu-common, kuzu-function, kuzu-extension, ureq
kuzu-algo           ← depends on kuzu-common, kuzu-function, kuzu-extension, kuzu-graph
kuzu-neo4j          ← depends on kuzu-function, kuzu-extension
kuzu-llm            ← depends on kuzu-common, kuzu-function, kuzu-extension
kuzu-duckdb         ← depends on kuzu-common, kuzu-function, kuzu-extension, duckdb (optional)
kuzu-sqlite         ← depends on kuzu-function, kuzu-extension, rusqlite (optional)
kuzu-delta          ← depends on kuzu-common, kuzu-function, kuzu-extension, kuzu-duckdb (optional)
kuzu-iceberg        ← depends on kuzu-common, kuzu-function, kuzu-extension, kuzu-duckdb (optional)
kuzu-azure          ← depends on kuzu-common, kuzu-function, kuzu-extension, kuzu-duckdb (optional)
kuzu-postgres       ← depends on kuzu-function, kuzu-extension, tokio/tokio-postgres (optional)
kuzu-unity-catalog  ← depends on kuzu-common, kuzu-function, kuzu-extension, kuzu-duckdb (optional)
kuzu-processor      ← depends on kuzu-common, kuzu-catalog, kuzu-planner, kuzu-function, kuzu-storage, kuzu-parser, kuzu-fts
kuzu-main           ← depends on ALL core crates + optional extensions (feature-gated)
kuzu-cli            ← depends on kuzu-main, kuzu-common, kuzu-binder, kuzu-catalog, rustyline
kuzu-wasm           ← depends on kuzu-main, kuzu-common, wasm-bindgen
kuzu-c              ← depends on kuzu-main, kuzu-common, libc (cdylib + staticlib)
kuzu-migrate        ← depends on kuzu-main, kuzu-storage, kuzu-common, clap
```

### 2c. Fuzz Target (`kuzu-core/fuzz/Cargo.toml`)

```toml
[package]
name = "kuzu-fuzz"
version = "0.0.0"
# Fuzz targets: cypher_query, expression_eval, copy_from_csv
# Uses libfuzzer-sys + kuzu-main
```

### 2d. Rust API Crate (`tools/rust_api/Cargo.toml`)

```toml
[package]
name = "kuzu"
version = "0.12.0"
description = "An in-process property graph database management system"
# External-facing crate that wraps kuzu-main + kuzu-common
# Published on crates.io
```

### 2e. Ladybug Rust API (`ladybug/tools/rust_api/Cargo.toml`)

```toml
[package]
name = "lbug"
version = "0.17.0"
# Rust FFI bindings to Ladybug C++ via cxx (version =1.0.138)
# Build-time: cmake + cxx-build
```

### 2f. Ladybug Rust Examples (`ladybug/tools/rust_api/examples/Cargo.toml`)

```toml
[package]
name = "lbug-rust-example"
dependencies: lbug, arrow (optional)
```

### 2g. Kuzu Rust Example (`examples/rust/Cargo.toml`)

```toml
[package]
name = "kuzu-rust-example"
dependencies: kuzu (from tools/rust_api)
```

### Complete List of All Cargo.toml Files (37 total)

| # | Path |
|---|------|
| 1 | `kuzu-core/Cargo.toml` |
| 2 | `kuzu-core/kuzu-common/Cargo.toml` |
| 3 | `kuzu-core/kuzu-storage/Cargo.toml` |
| 4 | `kuzu-core/kuzu-transaction/Cargo.toml` |
| 5 | `kuzu-core/kuzu-catalog/Cargo.toml` |
| 6 | `kuzu-core/kuzu-parser/Cargo.toml` |
| 7 | `kuzu-core/kuzu-binder/Cargo.toml` |
| 8 | `kuzu-core/kuzu-planner/Cargo.toml` |
| 9 | `kuzu-core/kuzu-optimizer/Cargo.toml` |
| 10 | `kuzu-core/kuzu-processor/Cargo.toml` |
| 11 | `kuzu-core/kuzu-function/Cargo.toml` |
| 12 | `kuzu-core/kuzu-graph/Cargo.toml` |
| 13 | `kuzu-core/kuzu-extension/Cargo.toml` |
| 14 | `kuzu-core/kuzu-json/Cargo.toml` |
| 15 | `kuzu-core/kuzu-fts/Cargo.toml` |
| 16 | `kuzu-core/kuzu-vector/Cargo.toml` |
| 17 | `kuzu-core/kuzu-httpfs/Cargo.toml` |
| 18 | `kuzu-core/kuzu-duckdb/Cargo.toml` |
| 19 | `kuzu-core/kuzu-algo/Cargo.toml` |
| 20 | `kuzu-core/kuzu-neo4j/Cargo.toml` |
| 21 | `kuzu-core/kuzu-llm/Cargo.toml` |
| 22 | `kuzu-core/kuzu-sqlite/Cargo.toml` |
| 23 | `kuzu-core/kuzu-delta/Cargo.toml` |
| 24 | `kuzu-core/kuzu-iceberg/Cargo.toml` |
| 25 | `kuzu-core/kuzu-azure/Cargo.toml` |
| 26 | `kuzu-core/kuzu-postgres/Cargo.toml` |
| 27 | `kuzu-core/kuzu-unity-catalog/Cargo.toml` |
| 28 | `kuzu-core/kuzu-main/Cargo.toml` |
| 29 | `kuzu-core/kuzu-cli/Cargo.toml` |
| 30 | `kuzu-core/kuzu-wasm/Cargo.toml` |
| 31 | `kuzu-core/kuzu-c/Cargo.toml` |
| 32 | `kuzu-core/kuzu-migrate/Cargo.toml` |
| 33 | `kuzu-core/fuzz/Cargo.toml` |
| 34 | `tools/rust_api/Cargo.toml` |
| 35 | `examples/rust/Cargo.toml` |
| 36 | `ladybug/tools/rust_api/Cargo.toml` |
| 37 | `ladybug/tools/rust_api/examples/Cargo.toml` |

---

## 3. CMakeLists.txt FILES (C++ Project Structure)

There are **~140+ CMakeLists.txt** files across both the root Kuzu project and the `ladybug/` fork. Key ones:

### 3a. Root Kuzu CMake (`/CMakeLists.txt`) — 449 lines
- **Project**: Kuzu v0.12.0, C++20, C17
- **Build options**: SINGLE_THREADED, ENABLE_WERROR, ENABLE_ASAN/TSAN/UBSAN, ENABLE_LTO, BUILD_TESTS, BUILD_PYTHON, BUILD_JAVA, BUILD_NODEJS, BUILD_WASM, BUILD_SHELL, BUILD_EXTENSIONS, etc.
- **Key features**: Antlr4 grammar auto-generation, ccache/sccache support, WASM/emscripten support, cpptrace backtrace support
- **Third-party includes**: antlr4_cypher, antlr4_runtime, brotli, fast_float, mbedtls, parquet, snappy, thrift, miniz, nlohmann_json, pybind11, pyparse, re2, alp, spdlog, utf8proc, zstd, httplib, pcg, lz4, roaring_bitmap, simsimd
- **Subdirectories**: `third_party/`, `extension/`, `src/`, `test/`, `tools/`

### 3b. Ladybug Root CMake (`ladybug/CMakeLists.txt`) — 561 lines
- **Project**: Lbug v0.18.0, C++20, C11
- **Build options**: Similar to Kuzu but with LBUG_ prefix. Adds BUILD_WAL_DUMP, BUILD_SHARED_LBUG, BUILD_STATIC_LBUG, LBUG_API_USE_PRECOMPILED_LIB
- **Key difference**: Uses modern CMake target `lbug_link_deps` interface library; has `BundleStaticLibrary.cmake` module. Supports precompiled library linking for language bindings.

### 3c. Source Build (`src/CMakeLists.txt`) — 103 lines
- Builds `kuzu` (static) and `kuzu_shared` (dynamic) libraries
- Links to: antlr4_cypher, antlr4_runtime, brotlidec, brotlicommon, fast_float, utf8proc, re2, fastpfor, parquet, snappy, thrift, yyjson, zstd, miniz, mbedtls, lz4, roaring_bitmap, simsimd, OpenSSL
- Generates single-file header `kuzu.hpp` via `scripts/collect-single-file-header.py`

### 3d. Tools Build (`tools/CMakeLists.txt`) — 18 lines
Conditionally builds: shell, java_api, nodejs_api, python_api, benchmark, wasm

### 3e. Extension Build (`extension/CMakeLists.txt`) — 94 lines
- Supports 14 extensions: duckdb, postgres, sqlite, delta, iceberg, azure, unity_catalog, json, fts, vector, llm, httpfs, neo4j, algo
- Each extension can be built as shared (`.kuzu_extension`) or statically linked
- Apple uses `-undefined dynamic_lookup`; Windows links against kuzu

### 3f. Third-party Build (`third_party/CMakeLists.txt`) — 28 lines
Builds all 26 bundled libraries including ANTLR-based Cypher parser, compression libs, Parquet, etc.

### 3g. Test Build (`test/CMakeLists.txt`) — 17 lines
Builds GoogleTest-based tests for: api, binder, copy, c_api, common, graph_test, optimizer, planner, runner, storage, test_helper, test_runner, transaction

### Complete list of all CMakeLists.txt paths (~140+ total across both projects)

**Root Kuzu CMakeLists.txt files:**
- `C:\Users\anjan\dev\memory\kuzu\CMakeLists.txt`
- `C:\Users\anjan\dev\memory\kuzu\src\CMakeLists.txt` + all subdirectories (binder, c_api, catalog, common, expression_evaluator, function, graph, main, optimizer, parser, planner, processor, storage/*, transaction, extension)
- `C:\Users\anjan\dev\memory\kuzu\tools\CMakeLists.txt` + shell, shell/printer, python_api, nodejs_api, java_api, benchmark, wasm
- `C:\Users\anjan\dev\memory\kuzu\test\CMakeLists.txt` + all test subdirectories (api, binder, c_api, common, copy, graph_test, gtest, optimizer, planner, runner, storage, test_helper, test_runner, transaction)
- `C:\Users\anjan\dev\memory\kuzu\extension\CMakeLists.txt`
- `C:\Users\anjan\dev\memory\kuzu\third_party\CMakeLists.txt` + all third_party libs
- `C:\Users\anjan\dev\memory\kuzu\examples\c\CMakeLists.txt`, `examples\cpp\CMakeLists.txt`

**Ladybug CMakeLists.txt files (mostly parallel structure):**
- `C:\Users\anjan\dev\memory\kuzu\ladybug\CMakeLists.txt`
- `C:\Users\anjan\dev\memory\kuzu\ladybug\src\CMakeLists.txt` + all subdirectories (same as Kuzu)
- `C:\Users\anjan\dev\memory\kuzu\ladybug\tools\CMakeLists.txt` + rust_api/src, shell, shell/printer, python_api, nodejs_api, wasm, wal_dump
- `C:\Users\anjan\dev\memory\kuzu\ladybug\test\CMakeLists.txt` + all subdirectories
- `C:\Users\anjan\dev\memory\kuzu\ladybug\extension\CMakeLists.txt`
- `C:\Users\anjan\dev\memory\kuzu\ladybug\third_party\CMakeLists.txt` + all third_party libs

---

## 4. BUILD SCRIPTS AND CI CONFIG

### 4a. Root Makefile (`Makefile`) — 388 lines
CMake frontend with targets:
- `release`, `debug`, `relwithdebinfo` — build types
- `all`, `allconfig`, `alldebug` — full builds with all components
- `python`, `java`, `nodejs` — language API builds
- `wasm`, `wasmtest` — WebAssembly builds
- `rusttest` — runs `cargo test` in `tools/rust_api`
- `test`, `test-build` — C++ test suite
- `benchmark`, `example` — misc builds
- `tidy`, `tidy-analyzer`, `clangd-diagnostics` — linting
- `install`, `clean`

### 4b. Ladybug Makefile (`ladybug/Makefile`) — 413 lines
Similar structure with additions:
- `shell`, `shell-debug`, `shell-test` — shell-specific builds
- `extension-*` — richer extension support with `PGEMBED_FIXTURE`
- `test-build-release` — separate test build target

### 4c. Python Packaging (`scripts/pip-package/setup.py`)
Builds and packages Kuzu Python wheels using CMake.

### 4d. CI Workflows (GitHub Actions)

**Kuzu CI (`.github/workflows/`):**
| File | Purpose |
|------|---------|
| `build-and-release.yml` | Builds Python wheels (manylinux x86_64/arm64, macOS, Windows) + releases |
| `build-extension-registry.yml` | Builds extension registry |
| `rust-ci.yml` | Rust workspace fmt, clippy, build, test on Ubuntu/macOS/Windows + WASM |
| `rust-release.yml` | Creates GitHub release with CLI binaries for Linux/macOS/Windows |
| `fuzz-ci.yml` | Runs `cargo-fuzz` targets (PR: 10min, nightly: 30min) |

**Ladybug CI (`ladybug/.github/workflows/`):**
| File | Purpose |
|------|---------|
| `ci-workflow.yml` | Main CI (433 lines): builds, tests on Linux/macOS/Windows |
| `build-and-deploy.yml` | Build and deploy pipeline |
| `build-extensions.yml` | Extension building |
| `python-wheel-workflow.yml` | Python wheel builds |
| `wasm-workflow.yml` | WASM-specific builds |
| `java-workflow.yml`, `nodejs-workflow.yml` | Language-specific CI |
| `ci-musl-test-workflow.yml` | musl libc testing |
| `codeql.yml` | CodeQL security analysis |
| `benchmark-workflow.yml`, `lsqb-benchmark-workflow.yml` | Performance benchmarks |
| `precompiled-bin-workflow.yml` | Precompiled binary releases |
| `release-artifacts.yml` | Release artifact generation |
| `zizmor.yml` | Workflow linting |

### 4e. Key Shell Scripts
- `scripts/run-clang-format.py` — code formatting
- `scripts/collect-single-file-header.py` — generates `kuzu.hpp`
- `scripts/generate_binary_*.sh` — binary dataset generation
- `ladybug/scripts/download-liblbug.sh` — downloads precompiled Ladybug lib
- `ladybug/scripts/run-clang-format-docker.sh` — formatting via Docker

### 4f. Environment Config
- `ladybug/pixi.toml` — Conda/pixi environment for ADBC (Apache Arrow Database Connectivity)
- `.cargo/` — empty, no custom config
- `extension/extension_config.cmake` — extension build configuration

---

## 5. KEY SOURCE DIRECTORIES

### 5a. C++ Source (Kuzu original) — `src/`

| Directory | Purpose |
|-----------|---------|
| `src/antlr4/` | ANTLR4 Cypher grammar files |
| `src/binder/` | SQL statement binder (semantic analysis) |
| `src/c_api/` | C language bindings (11 files) |
| `src/catalog/` | Catalog management (tables, properties) |
| `src/common/` | Core types, utilities, vectors, serialization, file system |
| `src/expression_evaluator/` | Expression evaluation engine |
| `src/extension/` | Extension loading, installation, management |
| `src/function/` | 40+ files: scalar/aggregate/table functions |
| `src/graph/` | Graph data structures |
| `src/include/` | Public headers (mirrors src/ structure) |
| `src/main/` | Database, Connection, QueryResult, Settings |
| `src/optimizer/` | 16 optimizer passes (filter push-down, join ordering, etc.) |
| `src/parser/` | ANTLR-based Cypher parser, AST transformation |
| `src/planner/` | Logical query planner, join order enumeration |
| `src/processor/` | Query execution engine (physical operators) |
| `src/processor/operator/` | 15+ operator families (aggregate, hash_join, order_by, scan, persistent CSV/npy/Parquet read/write, etc.) |
| `src/storage/` | Columnar disk storage, buffer manager, WAL, compression, indexing |
| `src/transaction/` | ACID transaction management |

### 5b. Rust Source (Kuzu rewrite) — `kuzu-core/`

| Crate | Purpose |
|-------|---------|
| `kuzu-common/` | Core data types, Arrow integration, serialization |
| `kuzu-storage/` | Columnar storage engine (features: parquet, csv) |
| `kuzu-transaction/` | Transaction state machine |
| `kuzu-catalog/` | Schema catalog |
| `kuzu-parser/` | PEG-based Cypher parser (pest crate) |
| `kuzu-binder/` | Semantic binding of parsed statements |
| `kuzu-planner/` | Logical query planning |
| `kuzu-optimizer/` | Query optimization |
| `kuzu-processor/` | Physical query execution (6 benchmark targets) |
| `kuzu-function/` | Built-in functions (hash, regex, time, crypto) |
| `kuzu-graph/` | Graph traversal structures |
| `kuzu-extension/` | Extension framework |
| `kuzu-main/` | **Core library** — aggregates all crates, feature-gated extensions |
| `kuzu-cli/` | Interactive CLI shell |
| `kuzu-c/` | C FFI bindings (cdylib + staticlib) |
| `kuzu-wasm/` | WASM bindings (wasm-bindgen) |
| `kuzu-migrate/` | Database migration tool |
| Extension crates: `kuzu-json`, `kuzu-fts`, `kuzu-vector`, `kuzu-httpfs`, `kuzu-duckdb`, `kuzu-algo`, `kuzu-neo4j`, `kuzu-llm`, `kuzu-sqlite`, `kuzu-delta`, `kuzu-iceberg`, `kuzu-azure`, `kuzu-postgres`, `kuzu-unity-catalog` |

### 5c. Ladybug C++ Source — `ladybug/src/`

Mirrors the Kuzu `src/` directory structure exactly (18 subdirectories). The key difference is C++ API headers use `lbug` prefix internally and the extension framework has additional ADBC support.

### 5d. Language Bindings (Tools)

| Path | Language | Technology | Library |
|------|----------|------------|---------|
| `tools/python_api/` | Python | pybind11 | `kuzu` PyPI package |
| `tools/nodejs_api/` | Node.js | N-API | `kuzu` npm package |
| `tools/rust_api/` | Rust | Direct Cargo | `kuzu` crates.io crate |
| `tools/java_api/` | Java | JNI | `kuzu` Maven package |
| `tools/wasm/` | WebAssembly | Emscripten | Browser JS bindings |
| `tools/shell/` | CLI | C++ | Interactive shell (linenoise) |
| `tools/stress/` | Stress test | Python | Agent memory concurrency testing |
| `tools/benchmark/` | Benchmark | C++ | Performance benchmarking |
| `ladybug/tools/rust_api/` | Rust (Ladybug) | cxx FFI | `lbug` crate (C++ interop) |
| `ladybug/tools/wal_dump/` | WAL dump | C++ | Write-ahead log inspector |

---

## 6. README AND DOCUMENTATION FILES

### Root Level Documentation
- `README.md` — Vela Partners fork README (concurrent multi-writer, AI agent focus)
- `CONTRIBUTING.md` — Contribution guide
- `CODE_OF_CONDUCT.md` — Code of conduct
- `CLA.md` — Contributor License Agreement
- `LICENSE` — MIT License
- `MIGRATION.md` — Migration guide

### Kuzu-Core Documentation
- `kuzu-core/README.md` — Rust workspace overview
- `kuzu-core/implementation_plan.md` — Rust port design decisions
- `kuzu-core/RELEASE.md` — Release process
- `kuzu-core/STATUS.md` — Project status
- `kuzu-core/BENCHMARK_COMPARISON.md` — Benchmark results
- `kuzu-core/MIGRATION.md` — Migration notes
- Each `kuzu-*` subcrate has its own `README.md`

### Ladybug Documentation
- `ladybug/README.md` — LadybugDB overview
- `ladybug/CONTRIBUTING.md` — Contribution guide
- `ladybug/SECURITY.md` — Security policy
- `ladybug/AGENTS.md` — **Agent-specific build/test guide** (build commands, test commands, code style)
- `ladybug/docs/` — Developer documentation:
  - `build_tips.md` — Build tips
  - `cpp_style.md` — C++ style guide
  - `extensions.md` — Extension development
  - `grammar.md` — Cypher grammar editing
  - `python.md` — Python development
  - `shell.md` — Shell development
  - `testing.md` — Testing patterns
  - `icebug-disk.md` — Iceberg disk format
  - `incidents/README.md` — Incident documentation
- `ladybug/examples/README.md` — Examples overview
- `ladybug/kuzu-core/README.md` — Kuzu-core within Ladybug

### Additional Doc Files
- `examples/README.md` — C/C++ examples overview
- `tools/python_api/README.md` — Python API docs
- `tools/nodejs_api/README.md` — Node.js API docs
- `tools/java_api/README.md` — Java API docs
- `tools/wasm/README.md` — WASM bindings docs
- `benchmark/click/README.md`, `benchmark/lsqb/README.md` — Benchmark docs
- `src/antlr4/README.md` — Grammar generation notes
- `dataset/*/README.md` — Dataset descriptions
- `third_party/*/README.md` — Third-party library docs

---

## SUMMARY

This is a **dual-project repository**:

| Aspect | KuzuDB (root) | LadybugDB (`ladybug/`) |
|--------|---------------|----------------------|
| **Language** | C++20 (original), Rust 2024 (`kuzu-core/`) | C++20 |
| **Version** | 0.12.0 (C++ cmake), 0.1.0 (Rust workspace) | 0.18.0 |
| **Rust crates** | 30-workspace pure Rust rewrite | 1 crate (`lbug`) via CXX FFI |
| **Extensions** | 14 C/C++ extensions | 15 (adds ADBC) |
| **CI workflows** | 7 (including Rust CI/release) | 20 |
| **Third-party** | 28 C/C++ libs | 28 C/C++ libs (similar) |
| **Build system** | CMake + Make + pip | CMake + Make + pixi |
| **Language bindings** | Python, Node.js, Rust, Java, C, WASM | Python, Node.js, Rust, Java, C, WASM, Go, Swift |

The Rust rewrite (`kuzu-core/`) is a pure-Rust implementation with no C++ dependency, while the Ladybug Rust API (`ladybug/tools/rust_api/`) wraps the C++ core via `cxx` FFI.