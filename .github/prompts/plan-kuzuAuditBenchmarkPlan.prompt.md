# Plan: Audit & Benchmark — Kuzu Rust Port vs C++

## TL;DR

The Rust port has **28 crates, ~5,400 LOC, 203 tests** passing, but has **critical gaps** vs the C++ original (~2,074 files, ~750+ .cpp core files). Most notably: **no real table scan** (PhysicalScan generates fake data), **flat sequential pipeline** (no DAG/parallel execution), **factorized table not ported**, **csv/parquet IO not ported**, and **storage engine is in-memory only**. Benchmarking requires: (1) building C++ benchmark tool, (2) adding Rust criterion benches, (3) creating a common query suite.

---

## Phase 1: C++ Build Environment Setup

**Goal**: Build Kuzu C++ with benchmark tool on Windows (MinGW + Ninja).

### Steps

1. **Verify prerequisites**
   - MinGW-w64 gcc 14.2.0 (exists at `C:\mingw64`)
   - CMake (`winget install Kitware.CMake` if missing)
   - Ninja (`pip install ninja` if missing)

2. **Build Kuzu C++ release with benchmark**
   ```
   mkdir build\release
   cmake -B build\release -G Ninja ^
       -DCMAKE_BUILD_TYPE=Release ^
       -DBUILD_BENCHMARK=TRUE ^
       -DBUILD_SHELL=OFF ^
       -DBUILD_SINGLE_FILE_HEADER=OFF ^
       -DAUTO_UPDATE_GRAMMAR=OFF .
   cmake --build build\release
   ```
   *depends on step 1*

3. **Verify benchmark binary**
   - Check `build\release\tools\benchmark\kuzu_benchmark.exe` exists
   - Run a smoke test with `dataset/tinysnb`:
     ```
     build\release\tools\benchmark\kuzu_benchmark ^
         --dataset=dataset/tinysnb ^
         --benchmark=tools/benchmark/example/example.benchmark ^
         --warmup=1 --run=3
     ```
   *depends on step 2*

**Verification**: `kuzu_benchmark.exe` runs and outputs timing for example benchmark.

---

## Phase 2: Rust Benchmark Infrastructure Setup

**Goal**: Add criterion benchmarking to `kuzu-main` and `kuzu-processor`.

### Steps

4. **Add criterion + pprof dev-dependencies** (`parallel`)
   - `kuzu-main/Cargo.toml`: `criterion`, `pprof`
   - `kuzu-processor/Cargo.toml`: `criterion` (dev-dep)

5. **Create `kuzu-core/kuzu-main/benches/` directory** (`depends on 4`)
   - `query_pipeline.rs` — full pipeline benchmark
   - `micro_ops.rs` — individual physical operator benchmarks
   - `storage.rs` — buffer_manager pin/unpin throughput

6. **Create `kuzu-core/kuzu-processor/benches/`** (`depends on 4`)
   - `physical_scan.rs` — scan throughput
   - `physical_filter.rs` — filter selectivity + throughput
   - `physical_aggregate.rs` — aggregate throughput
   - `physical_hash_join.rs` — join throughput
   - `physical_order_by.rs` — sort throughput

7. **Add `[[bench]]` harness config** to `Cargo.toml` if criterion not auto-detected (`depends on 4`)

**Verification**: `cargo bench --workspace` runs successfully with all benches.

---

## Phase 3: Benchmark Query Suite — C++ Baseline

**Goal**: Capture C++ performance numbers as ground truth.

### Steps

8. **Select representative query suite** (parallel with Phase 2)
   - Use `dataset/tinysnb` (smallest dataset, ~8 Person nodes)
   - Select queries from `benchmark/queries/ldbc-sf100/` categories that Rust can also run
   - Create new `benchmark/queries/rust-compare/` directory with hand-picked queries:
     - Filter: `MATCH (n:Person) WHERE n.age > 25 RETURN n.name, n.age`
     - Aggregation: `MATCH (n:Person) RETURN count(*)`
     - Join (2-hop): `MATCH (a:Person)-[r:Knows]->(b:Person) RETURN a.name, b.name`
     - Graph algo: (WCC, PageRank if Rust implements them)
     - Order By: `MATCH (n:Person) RETURN n.name ORDER BY n.age LIMIT 3`

9. **Run C++ benchmarks** (`depends on 3, 8`)
   - For each query category:
     - Warmup: 3 runs
     - Measure: 10 runs
     - Record: avg latency, min, max, stddev
   - Save results to `kuzu-core/BENCHMARK_CXX.md`

10. **Run C++ benchmark for tinysnb dataset** (`depends on 3`)
    - Full suite of tinysnb-compatible queries
    - Record wall-clock time for each physical operator type

**Verification**: `BENCHMARK_CXX.md` contains C++ latency numbers for all query categories.

---

## Phase 4: Rust Performance Baseline

**Goal**: Measure current Rust implementation performance and compare to C++. Note: Rust currently generates synthetic data (no real storage reads), so results will be synthetic throughput, not apples-to-apples.

### Steps

11. **Run Rust criterion benchmarks** (`depends on 5-7`)
    - Measure full pipeline (parse→bind→plan→optimize→execute)
    - Measure individual operators in isolation:
      - PhysicalScan (100 rows, 1000 rows, 10000 rows)
      - PhysicalFilter (10%, 50%, 90% selectivity)
      - PhysicalHashJoin (10 vs 100 rows, 100 vs 1000 rows)
      - PhysicalOrderBy (100, 1000, 10000 rows)
      - PhysicalAggregate (COUNT, SUM, AVG with/without GROUP BY)
    - Record to `kuzu-core/BENCHMARK_RUST.md`

12. **Create Rust integration benchmark tests** (`depends on 11`)
    - End-to-end query pipeline with known inputs
    - Measure throughput (queries/sec) for each query category
    - Compare against C++ numbers from Phase 3

13. **Compare Rust vs C++** (`depends on 9, 12`)
    - Create comparison table per query category
    - Identify gap ratio: Rust / C++
    - Flag operators where Rust is > 10x slower

**Verification**: `BENCHMARK_RUST.md` with all numbers, comparison table identifying critical gaps.

---

## Phase 5: Gap Analysis — Full Crate Audit

**Goal**: Comprehensive mapping of what exists in Rust vs C++ per component.

### Steps (all parallel — independent crate-level audits)

14. **Common & Types audit** — Compare `src/common/` (~150 files) vs `kuzu-common` (9 files)
    - What types are missing (UUID, Blob, Interval, JSON internal types)?
    - File system variants not ported
    - Profiler, signal handling, UTF-8 ops not ported
    - Cast operations coverage

15. **Storage engine audit** — Compare `src/storage/` (~152 files) vs `kuzu-storage` (11 files)
    - Column types: struct, list, string, dictionary not ported
    - CSR node groups not ported
    - Overflow file, disk_array, free_space_manager not ported
    - LocalStorage details, Checkpointer not ported
    - **Critical**: No columnar data layout → full table scan reads from nowhere

16. **Processor audit** — Compare `src/processor/` (~224 files) vs `kuzu-processor` (3 files)
    - FactorizedTable (10 files) — **CORE MISSING DATA STRUCTURE**
    - HashJoin internals, OrderBy internals
    - CSV/Parquet reader/writer (~38 files)
    - Persistent operators (INSERT/UPDATE/DELETE/MERGE/COPY) — ~15 files
    - Recursive extend, path property probe, semi masker
    - **Critical**: No parallel execution despite TaskSystem existing

17. **Planner audit** — Compare `src/planner/` (~113 files) vs `kuzu-planner` (4 files)
    - 30+ append_* plan construction files
    - Plan subquery, plan update, plan copy, plan_read variants
    - Join order enumeration depth

18. **Binder audit** — Compare `src/binder/` (~102 files) vs `kuzu-binder` (3 files)
    - Expression binder variants (property, parameter, function, case, lambda)
    - Bind reading/writing clause
    - Import/export database binding
    - Rewriter/visitor patterns

19. **Function audit** — Compare `src/function/` (~236 files) vs `kuzu-function` (3 files)
    - 40+ vector_*_function files (string, date, cast, arithmetic, boolean, etc.)
    - Built-in function coverage
    - Cast function completeness

20. **Optimizer audit** — Compare `src/optimizer/` (~30 files) vs `kuzu-optimizer` (4 files)
    - C++ has 15 passes → Rust has 8 (missing: agg_key_dependency, acc_hash_join, correlated_subquery_unnest, limit_push_down, remove_factorization_rewriter, schema_populator, remove_unnecessary_join)

21. **Extensions audit** — Compare C++ extensions vs Rust extension stubs
    - kuzu-algo: C++ has 15+ algo files → Rust has stub
    - kuzu-vector: C++ has HNSW → Rust has partial 170 LOC
    - kuzu-httpfs, kuzu-llm, kuzu-neo4j: stubs only
    - kuzu-duckdb, kuzu-sqlite, kuzu-postgres, kuzu-delta, kuzu-iceberg, kuzu-azure, kuzu-unity-catalog: partial via DuckDB delegation

**Verification**: Complete `GAP_ANALYSIS.md` with per-component tables showing Rust coverage %, missing features list, and criticality rating.

---

## Phase 6: Improvement Plan

**Goal**: Prioritized roadmap for closing the most impactful gaps.

### Steps

22. **Triage gaps by impact** (depends on 14-21)
    - P0 (blocks usable product): Real table scan, FactorizedTable, CSV/Parquet IO, persistent operators
    - P1 (major performance): Parallel execution, columnar storage, HashJoin/OrderBy internals
    - P2 (correctness): Expression evaluator completeness, multi-type support in operators
    - P3 (completeness): Remaining optimization passes, extension depth, CLI features

23. **Define Rust benchmark targets** (depends on 13)
    - For each P0 gap: define acceptance criteria (e.g., "PhysicalScan reads from StorageManager with < 1ms latency for 100 rows")
    - Performance targets per query category (update from BENCHMARK_BASELINE.md)

24. **Create staged implementation plan** (depends on 22, 23)
    - Stage 1 (P0): Real storage I/O, FactorizedTable, CSV reader, persistent operators
    - Stage 2 (P1): DAG-based parallel execution pipeline, columnar storage access
    - Stage 3 (P2): Expression evaluator depth, full type support in physical operators
    - Stage 4 (P3): Remaining optimizer passes, extension parity, CLI improvements

**Verification**: `IMPROVEMENT_ROADMAP.md` with staged plan, acceptance criteria, and effort estimates.

---

## Summary — All Files Created

| File | Phase | Purpose |
|------|-------|---------|
| `kuzu-core/BENCHMARK_CXX.md` | 3 | C++ baseline performance numbers |
| `kuzu-core/BENCHMARK_RUST.md` | 4 | Rust baseline performance numbers |
| `kuzu-core/GAP_ANALYSIS.md` | 5 | Per-component gap analysis vs C++ |
| `kuzu-core/IMPROVEMENT_ROADMAP.md` | 6 | Prioritized improvement plan |
| `kuzu-core/kuzu-main/benches/query_pipeline.rs` | 2 | Full pipeline criterion bench |
| `kuzu-core/kuzu-main/benches/micro_ops.rs` | 2 | Micro-benchmarks for operators |
| `kuzu-core/kuzu-main/benches/storage.rs` | 2 | Storage engine benchmarks |
| `kuzu-core/kuzu-processor/benches/*.rs` | 2 | Per-operator micro-benchmarks |
| `benchmark/queries/rust-compare/*.benchmark` | 3 | Shared query suite for Rust vs C++ comparison |

## Decisions

- **C++ benchmark tool**: Will build with MinGW + Ninja. If build fails, fall back to: (a) using existing C++ binary if available, or (b) documenting C++ build steps for the user to run manually.
- **Rust benchmarks**: criterion crate with `cargo bench`. No flamegraphs in initial pass — add `pprof` later for hot-spot analysis.
- **Dataset**: Start with `dataset/tinysnb` (small, fast, both Rust and C++ can consume it). Later scale to LDBC SF-1 for realistic comparisons.
- **Query suite**: Hand-picked queries that both implementations can execute. Focus on categories that Rust currently supports: Filter, Projection, Aggregation, 2-hop Join, OrderBy, Limit.
- **Rust API tests currently generate synthetic data** — benchmarks will measure pipeline throughput, not real I/O. Real I/O comparison comes after Phases 5-6 improvements.

## Further Considerations

1. **C++ build on Windows/MinGW**: May encounter compilation errors due to header differences or missing dependencies. Plan includes a fallback: manually document benchmark queries and let user build C++ separately.
2. **Rust `PhysicalScan` generates fake data**: This means Phase 4 Rust numbers won't be comparable to C++ for scan-heavy queries. We'll need to flag this clearly and create "adjusted" benchmarks after the real scan is implemented.
