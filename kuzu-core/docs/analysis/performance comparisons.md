# Search for performance comparisons 17/07/2026

## Comprehensive Search Results Report

### 1. Files Mentioning "benchmark", "perf", "performance", "bench" in Filenames

**Benchmark infrastructure (C++ tools):**
- `C:\Users\anjan\dev\memory\kuzu\tools\benchmark\include\benchmark.h` -- C++ benchmark class definition
- `C:\Users\anjan\dev\memory\kuzu\tools\benchmark\include\benchmark_runner.h` -- Benchmark runner header
- `C:\Users\anjan\dev\memory\kuzu\tools\benchmark\include\benchmark_parser.h` -- Benchmark parser header
- `C:\Users\anjan\dev\memory\kuzu\tools\benchmark\include\benchmark_config.h` -- Benchmark config header
- `C:\Users\anjan\dev\memory\kuzu\tools\benchmark\benchmark_runner.cpp` -- Runner implementation
- `C:\Users\anjan\dev\memory\kuzu\tools\benchmark\benchmark_parser.cpp` -- Parser implementation
- `C:\Users\anjan\dev\memory\kuzu\tools\benchmark\benchmark.cpp` -- Benchmark implementation
- `C:\Users\anjan\dev\memory\kuzu\tools\benchmark\example\example.benchmark` -- Example benchmark file

**Rust criterion benchmarks (9 benchmark suites across 3 crates):**
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-processor\benches\physical_scan.rs` -- Scan throughput benchmarks
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-processor\benches\physical_filter.rs` -- Filter throughput (Arrow-native)
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-processor\benches\physical_hash_join.rs` -- Hash join benchmarks
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-processor\benches\physical_order_by.rs` -- Order by benchmarks
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-processor\benches\physical_aggregate.rs` -- Aggregate benchmarks
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-processor\benches\evaluate_arrow.rs` -- Arrow-native vs per-row eval comparison
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-main\benches\query_pipeline.rs` -- End-to-end SQL pipeline (includes C++ comparison)
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-main\benches\storage_bench.rs` -- Storage benchmarks
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\benches\hybrid_eval.rs` -- Hybrid evaluation benchmarks

**Performance data/output files:**
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\rust_bench.txt` -- **Empty file** (placeholder for Rust benchmark results)
- `C:\Users\anjan\dev\memory\kuzu\cpp_bench.json` -- **Binary file** (JSON output from C++ benchmark binary)

**Performance documentation:**
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\BENCHMARK_COMPARISON.md` -- **Main Rust vs C++ comparison document** (504 lines, comprehensive)
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\implementation_plan.md` -- Contains P26.4 Performance Profiling section
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\STATUS.md` -- Comprehensive status including performance metrics

**Benchmark query files (.benchmark):**
- ~84+ `.benchmark` files across 5 datasets in `C:\Users\anjan\dev\memory\kuzu\benchmark\queries\`:
  - `ldbc-sf100/` -- 40+ benchmarks (filter, join, scan, aggregation, order_by, etc.)
  - `click/` -- 43 benchmarks (click analytics queries)
  - `soc-livejournal/` -- 6 benchmarks (graph algorithms: wcc, scc, pr, louvain, kcore)
  - `graph500-27/` -- 5 benchmarks (graph algorithms)
  - `datagen-sf10k/` -- 6 benchmarks (graph algorithms)
  - `micro/` -- 2 benchmarks (simple micro queries)

---

### 2. "ladybug", "vela", "drop-in" in File Contents (Case Insensitive)

**"ladybug":**
The repository contains a `ladybug/` submodule at `C:\Users\anjan\dev\memory\kuzu\ladybug\` -- this is an embedded submodule of the C++ LadybugDB project (`https://github.com/mlengse/ladybug`). It contains:
- Full C++ source code (`src/`, `test/`, `tools/`, `extension/`, `third_party/`)
- Python API (`tools/python_api/`)
- Rust API (`tools/rust_api/`) -- the Rust bindings to the C++ library
- Node.js API (`tools/nodejs_api/`)
- WASM bindings (`tools/wasm/`)
- Benchmark scripts and queries (`benchmark/`)
- Shell tool (`tools/shell/`)
- The C++ LadybugDB is the reference implementation being ported to Rust

**"vela":**
- `C:\Users\anjan\dev\memory\kuzu\.github\workflows\build-and-release.yml` -- References `@vela-engineering` npm scope for publishing packages
- `C:\Users\anjan\dev\memory\kuzu\tools\nodejs_api\package.json` -- Package name is `@vela-engineering/kuzu`
- `C:\Users\anjan\dev\memory\kuzu\tools\python_api\test\test_extension.py` -- Downloads extensions from `vela-engineering.github.io`

**"drop-in":**
- `C:\Users\anjan\dev\memory\kuzu\third_party\nlohmann_json\json.hpp` -- Describes itself as "drop-in replacement for C++14's `std::integer_sequence`"
- `C:\Users\anjan\dev\memory\kuzu\third_party\miniz\miniz.hpp` and `C:\Users\anjan\dev\memory\kuzu\ladybug\third_party\miniz\miniz.hpp` -- miniz described as "drop-in replacement for the subset of zlib"

---

### 3. Benchmark Scripts (.py, .sh, .js)

**Python:**
- `C:\Users\anjan\dev\memory\kuzu\benchmark\benchmark_runner.py` -- Main benchmark runner (serializes dataset, runs `kuzu_benchmark`, uploads results)
- `C:\Users\anjan\dev\memory\kuzu\benchmark\lsqb\benchmark_runner.py` -- LSQB (LDBC Social Network Benchmark) runner
- `C:\Users\anjan\dev\memory\kuzu\benchmark\serializer.py` -- Dataset serialization script
- `C:\Users\anjan\dev\memory\kuzu\benchmark\version.py` -- Version helper for benchmarks
- `C:\Users\anjan\dev\memory\kuzu\benchmark\click\query.py` -- Click benchmark queries
- `C:\Users\anjan\dev\memory\kuzu\benchmark\click\load.py` -- Click benchmark data loading
- `C:\Users\anjan\dev\memory\kuzu\benchmark\lsqb\serializer.py` -- LSQB serializer
- `C:\Users\anjan\dev\memory\kuzu\benchmark\lsqb\results_reporter.py` -- LSQB results reporter
- `C:\Users\anjan\dev\memory\kuzu\ladybug\tools\benchmark\navix\navix_paper_benchmark.py` -- NaviX filtered vector search benchmark
- `C:\Users\anjan\dev\memory\kuzu\scripts\export-import-test.py` -- Export/import test script

**Shell:**
- `C:\Users\anjan\dev\memory\kuzu\benchmark\click\benchmark.sh` -- Click benchmark orchestration (downloads data, runs queries, formats results)
- `C:\Users\anjan\dev\memory\kuzu\benchmark\click\run.sh` -- Click queries runner
- `C:\Users\anjan\dev\memory\kuzu\ladybug\tools\benchmark\navix\run_navix_vs_vanilla_efs96.sh` -- NaviX vs vanilla HNSW filtered search benchmark
- `C:\Users\anjan\dev\memory\kuzu\ladybug\benchmark\click\benchmark.sh` -- Ladybug version of click benchmark
- `C:\Users\anjan\dev\memory\kuzu\ladybug\benchmark\click\run.sh` -- Ladybug version of click runner

**.js:**
- No standalone benchmark scripts in JS; the WASM and Node.js API test files are functional tests, not benchmarks.

---

### 4. Results/Output Files Containing Performance Numbers

**Key performance data files:**
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\BENCHMARK_COMPARISON.md` -- **Primary comparison document** with extensive performance numbers for Rust vs C++ across scan, filter, hash join, order by, aggregate, and expression evaluation. Key finding: Rust reached **C++ parity** (397 us vs 400 us) on the filter+COUNT benchmark.
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\implementation_plan.md` -- Contains **P26.4 Performance Profiling Report** with 8 benchmark suite results including scan (3.5-6.3x faster than old baseline), filter (Arrow-native 17-122x speedup), hash join (7.5-11.8x faster), order by, aggregate, and expression evaluation data.
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\STATUS.md` -- Contains performance summary metrics including the 4.5x gap closure timeline and operator-level breakdown.
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\rust_bench.txt` -- **Empty file** (placeholder)
- `C:\Users\anjan\dev\memory\kuzu\cpp_bench.json` -- **Binary/JSON** C++ benchmark results
- `C:\Users\anjan\dev\memory\kuzu\ladybug\tools\benchmark\navix\navix_vs_vanilla_efs96.md` -- NaviX filtered vector search comparison results (recall and latency at different selectivities)

---

### 5. C++ Compatibility or FFI References

**Rust FFI to C++ (ladybug rust_api bindings):**
- `C:\Users\anjan\dev\memory\kuzu\ladybug\tools\rust_api\src\native.rs` -- "This module provides the same public API as the C++ FFI backend"
- `C:\Users\anjan\dev\memory\kuzu\ladybug\tools\rust_api\src\value.rs` -- Extensive FFI bindings: `use crate::ffi::ffi;`, `impl TryFrom<&ffi::Value>`, dozens of `ffi::value_get_*` function calls mapping C++ value types to Rust
- `C:\Users\anjan\dev\memory\kuzu\ladybug\tools\rust_api\src\lib.rs` -- Build script that downloads and links against C++ Ladybug library
- `C:\Users\anjan\dev\memory\kuzu\ladybug\tools\rust_api\build.rs` -- Downloads ladybug source from GitHub, builds C++ library, links via cxx bridge

**"Port of C++" references in Rust kuzu-core:**
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\art_node.rs` -- "Port of C++ `ArtPrimaryKeyIndex::Node`"
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\art_key.rs` -- "Port of C++ `ArtKey` from `ladybug/src/storage/index/art_index.cpp`"
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\art_index.rs` -- "Port of C++ `ArtPrimaryKeyIndex`" (multiple porting references throughout)
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-function\src\registry.rs` -- "Hash functions -- ported from C++", "Interval constructor functions -- ported from C++"
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-planner\src\logical_operator.rs` -- "Simplified Rust port of C++ `LogicalSemiMasker`"
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-processor\src\physical\write_ops\recursiveextend.rs` -- "port of C++ `WeightedSPPathsFunction`"

**`extern "C"` blocks:**
- Found extensively in `C:\Users\anjan\dev\memory\kuzu\extension\fts\third_party\snowball\` (stemmer library) and in `C:\Users\anjan\dev\memory\kuzu\tools\shell\include\linenoise.h`, `C:\Users\anjan\dev\memory\kuzu\third_party\yyjson\src\yyjson.h`

---

### 6. Test Files That Compare Outputs Across Implementations

**Property-based equivalence tests:**
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-main\tests\test_proptest.rs` -- Three proptest properties:
  - `test_roundtrip_i64` -- Round-trip integer storage correctness
  - `test_join_associativity` -- Verifies join results match manual Rust computation
  - `test_filter_pushdown_equivalence` -- Verifies filter+join results match manual evaluation

**Cypher value equivalence test:**
- `C:\Users\anjan\dev\memory\kuzu\ladybug\tools\rust_api\src\value.rs` (line 1568) -- `test_cypher_value_equivalence()` -- Tests that values returned from Cypher queries match equivalent Rust `Value` variants

**C++ vs Rust cross-comparison (in benchmarks):**
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-main\benches\query_pipeline.rs` (line 135) -- "Apples-to-apples comparison with C++ kuzu_benchmark" -- benchmark function `bench_query_filter_count_10k_vs_cpp` measures `conn.execute()` after `conn.prepare()`, matching C++ methodology of measuring plan->optimize->execute (excluding parse+bind)

---

### 7. Serialization Format References (serde, protobuf, flatbuffers, capnp, messagepack, bincode)

**serde / serde_json:**
- Used extensively throughout `kuzu-core`:
  - `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\Cargo.toml` -- `serde` and `serde_json` dependencies
  - `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\spiller.rs` -- Uses `serde_json` for spill-to-disk serialization
  - `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-storage\src\column_chunk.rs` -- Uses `serde_json` for "simple portable binary representation"
  - `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-json\src\lib.rs` -- JSON type handling with `serde_json::Value`
  - `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-llm\src\lib.rs` -- LLM extension uses `serde_json` for API calls
  - `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-wasm\Cargo.toml` -- `serde-wasm-bindgen` for WASM bindings
  - `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-migrate\src\main.rs` -- Uses `serde_json::Value` for schema migration
  - `C:\Users\anjan\dev\memory\kuzu\ladybug\tools\rust_api\Cargo.toml` -- `serde_json = "1"`
  - `C:\Users\anjan\dev\memory\kuzu\ladybug\tools\rust_api\src\value.rs` -- `Json(serde_json::Value)` variant and conversions
- All `Cargo.lock` files in the workspace reference `serde`, `serde_core`, `serde_derive`, `serde_json`

**flatbuffers:**
- Referenced in `Cargo.lock` files across the Rust workspace (`kuzu-core/Cargo.lock`, `tools/rust_api/Cargo.lock`, `kuzu-core/fuzz/Cargo.lock`, `examples/rust/Cargo.lock`)
- Used in Node.js/WASM packages: `C:\Users\anjan\dev\memory\kuzu\ladybug\tools\nodejs_api\package-lock.json` and `C:\Users\anjan\dev\memory\kuzu\ladybug\tools\wasm\examples\browser_persistent\package-lock.json`

**protobuf:**
- Only found as comments in `C:\Users\anjan\dev\memory\kuzu\third_party\re2\re2.cpp` and `C:\Users\anjan\dev\memory\kuzu\third_party\httplib\httplib.h` (citing protobuf PR, MIME type handling) -- not used as a serialization format

**capnp, messagepack, bincode:** -- **Not found** anywhere in the repository.

---

### 8. API Comparison or Equivalence Tests

**Rust native vs C++ FFI API equivalence:**
- `C:\Users\anjan\dev\memory\kuzu\ladybug\tools\rust_api\src\native.rs` (line 3) -- "This module provides the same public API as the C++ FFI backend"

**Cypher value equivalence test:**
- `C:\Users\anjan\dev\memory\kuzu\ladybug\tools\rust_api\src\value.rs` (line 1568) -- `test_cypher_value_equivalence()` tests that RETURN 42 produces `Value::Int64(42)`, RETURN 3.14 produces `Value::Double(3.14)`, etc.

**Property-based equivalence (kuzu-core Rust port):**
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-main\tests\test_proptest.rs` -- Three properties testing correctness across the full pipeline

**End-to-end benchmark comparison:**
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-main\benches\query_pipeline.rs` -- benchmark `bench_query_filter_count_10k_vs_cpp` directly compares Rust `conn.execute()` against C++ `kuzu_benchmark.exe` using identical SQL queries and datasets
- The Python comparison script embedded in `BENCHMARK_COMPARISON.md` (lines 267-313) provides a framework for comparing Rust criterion output vs C++ benchmark JSON output

**Ladybug C++ Parity Gap Analysis:**
- `C:\Users\anjan\dev\memory\kuzu\kuzu-core\STATUS.md` (Section 8, lines 669-812) -- Comprehensive per-layer comparison of Rust `kuzu-core` vs C++ LadybugDB:
  - **Overall parity: ~88%**
  - Parser: 100% (58 vs 30+ statements, Rust exceeds)
  - Binder: 100% (43 vs 30+ bound statements, Rust exceeds)
  - Planner: 100% (51 vs 38 logical ops, Rust exceeds)
  - Processor: ~100% (45 vs 67 physical ops, gap is split-phase only)
  - Optimizer: 100% (22 vs 17 passes, Rust exceeds by 5)
  - Functions: ~100% (234 unique vs 607 registrations, gap is overloads only)
  - Storage: 100% (27 features)
  - GDS: 100% (15 algorithms)
  - Types: 100% (36 types, Rust exceeds)

---

### Summary of Key Performance Findings

1. **Rust has achieved C++ parity** on the benchmark query `MATCH (p:Person) WHERE p.age > 30 RETURN COUNT(p)` at **397 us (Rust) vs 400 us (C++)** -- a 4.5x gap was closed through two optimizations:
   - **P27.5** -- Direct `ColumnChunk->Arrow` scan path (7.8x scan improvement)
   - **P27.6** -- Aggregate COUNT fast path using `ArrayRef::len()` (7x aggregate improvement)

2. **9 Rust criterion benchmark suites** across 3 crates (processor, main, storage) measure operator-level performance

3. **84+ C++ benchmark files** across 5 datasets in `benchmark/queries/` provide the reference workload

4. **LadybugDB C++ submodule** at `ladybug/` is the reference implementation; the `kuzu-core/` Rust port targets full parity

5. **`serde_json`** is the primary serialization format in the Rust port (no protobuf, capnp, messagepack, or bincode usage)
