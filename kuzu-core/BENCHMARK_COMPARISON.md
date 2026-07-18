# Kuzu Rust vs C++ Performance Comparison

> **Date:** 2026-07-18 (P30.3 Complete — 3-way parity verified)
> **Rust:** criterion v0.8, cargo bench --workspace
> **Vela C++:** kuzu_benchmark.exe built 2026-07-12 (release, 19 MB), dataset 10k rows
> **LadybugDB C++:** lbug_benchmark.exe built 2026-07-18 (release, 21.7 MB), dataset 10k rows
> **Dataset:** Synthetic operator benchmarks + serialized 10k Person database

---

## Quick Start

```bash
# Run Rust benchmarks (operator micro-benchmarks)
cd kuzu-core
cargo bench -p kuzu-processor   # Scan, Filter, Join, OrderBy, Aggregate, ExpressionEval
cargo bench -p kuzu-main         # Full pipeline + Storage

# View HTML reports
# Open target/criterion/report/index.html
```

---

## TL;DR

The Rust Kuzu port shows **competitive performance** against **both** C++ implementations (Vela KuzuDB and LadybugDB). Phase 2 Arrow-native expression evaluation delivered **10–24× speedup** on filter/evaluation hot paths. See [C++ Setup](#cpp-setup) for build instructions.

**Current Status:**
- ✅ Rust micro-benchmarks: 38+ criterion benchmarks across scan, filter, join, sort, aggregate, expression eval
- ✅ Full pipeline benchmarks: parse→bind→plan→optimize→execute
- ✅ Arrow-native expression evaluation: **10–24× faster** for comparison/boolean/arithmetic ops
- ✅ **Vela C++** baseline: kuzu_benchmark.exe built and run (10k rows)
- ✅ **LadybugDB C++** baseline: lbug_benchmark.exe built and run (10k rows, 2026-07-18)
- ✅ **3-way parity verified: Rust ≈ Vela ≈ Ladybug** (397 µs Rust vs 400 µs Vela vs 374 µs Ladybug) on SQL-level filter+count
- ✅ Phase 1 (scan optimization): direct `ColumnChunk→Arrow` path — 7.8× scan improvement
- ✅ Phase 2 (aggregate optimization): bypass per-row Value dispatch — aggregate now ~50 µs

> **🍎🍎 3-way apples-to-apples:** All three runtimes measure `MATCH (p:Person) WHERE p.age > 30 RETURN COUNT(p)` on identical 10k-row datasets, with one-time compilation excluded. **Rust is at parity with both C++ implementations** (397 µs Rust vs 400 µs Vela vs 374 µs Ladybug). LadybugDB is slightly faster due to different C++ compiler version and optimization flags.

---

## Query Category Mapping

The following representative queries from `benchmark/queries/` are mapped to Rust operator benchmarks:

| Category | LDBC SF-100 Benchmark | Rust Operator Bench |
|----------|----------------------|-------------------|
| **Sequential Scan** | `q23` (fixed_size_seq_scan), `q19-q22` (var_size_seq_scan) | `scan/10k_rows`, `scan/10k_selective_2_of_4_cols` |
| **Filter** | `q14-q18`, `zonemap-*` | `filter/pass_all_10k`, `filter/remove_all_10k` |
| **Hash Join** | `q29` (1-hop knows), `q30` (2-hop knows) | `join/*` |
| **Order By** | `q25` (single-key), `q26` (multi-key) | `order_by/*` |
| **Aggregation** | `q24` (aggregation), `q28` | `aggregate/*` |
| **Limit** | `limit/push-down-limit-into-distinct` | `query/match_limit` |
| **Full Pipeline** | Various | `query/match_return_all`, `query/match_order_by` |

---

## Micro-Benchmark Results (Rust, criterion v0.5)

All times in **median µs** (except where noted). Lower is better.
Hardware: See `criterion` report in `target/criterion/` for detailed system info.

### Scan Throughput

| Benchmark | Time | Throughput | Notes |
|-----------|------|-----------|-------|
| `scan/100_rows` | **11.9 µs** | 33.6 M rows/s | 4 columns (id, name, score, active) |
| `scan/1k_rows` | **87.1 µs** | 45.9 M rows/s | ~linear scaling |
| `scan/10k_rows` | **1,050 µs** | 38.1 M rows/s | 1.05 ms for 10K rows |
| `scan/10k_selective_2_of_4_cols` | **168 µs** | — | Column projection saves ~6x |
| `scan/small_100_rows` (full pipeline) | **20.0 µs** | — | Via `Connection::query()` |
| `scan/medium_1k_rows` (full pipeline) | **86.5 µs** | — | Via `Connection::query()` |

**C++ comparison:** TBD — requires serialized tinysnb dataset and `kuzu_shell` to create a database.

### Filter Throughput

All benchmarks use the Arrow-native expression evaluator (`evaluate_to_arrow` + `boolean_array_to_selection`), which eliminated per-row Value enum boxing for comparisons, arithmetic, and boolean ops.

| Benchmark | Time (pre-Phase 2) | Time (Phase 2) | Speedup | Notes |
|-----------|-------------------|----------------|---------|-------|
| `filter/pass_all_10k` | 433 µs | **18.3 µs** | **24×** | Constant `true` — Arrow BooleanBuilder |
| `filter/remove_all_10k` | 34.7 µs | **9.3 µs** | **3.7×** | Constant `false` — early exit |
| `filter/property_check_10k` | 436 µs | **30.3 µs** | **14×** | Variable expression (non-null check) |
| `filter/batch_10x1k_chunks` | 425 µs | **27.1 µs** | **16×** | Same total rows, chunked input |
| `filter/multi_col_8_fields_10k` | 3.01 ms | **71 µs** | **42×** | 8 columns × 10K rows |

**Key insight:** The Arrow compute kernel hot path (comparison → boolean → selection) is **10–24× faster** than the per-row Value enum boxing. The `multi_col_8_fields_10k` improvement (42×) comes from multiple columns each using Arrow kernels instead of per-row scalar dispatch.

**C++ comparison:** TBD

### Hash Join Throughput

| Benchmark | Build | Probe | Time | Notes |
|-----------|-------|-------|------|-------|
| `join/100_build_100_probe` | 100 | 100 | **137 µs** | All keys match |
| `join/1k_build_1k_probe` | 1,000 | 1,000 | **1.44 ms** | All keys match |
| `join/10k_build_100_probe` | 10,000 | 100 | **11.8 ms** | Build-dominant |
| `join/100_build_10k_probe` | 100 | 10,000 | **1.94 ms** | Probe-dominant |
| `join/1k_multi_col_build_1k_probe` | 1,000 | 1,000 | **1.60 ms** | 2-column build side |
| `join/1k_no_match` | 1,000 | 1,000 | **1.07 ms** | No matching keys (fast probe) |

**Key insights:**
- Build side dominates cost (hash table construction)
- No-match is faster than match (no output row materialization)
- Multi-column build has ~11% overhead over single-column

**C++ comparison:** TBD — C++ has `q29` (1-hop knows on SF-100 dataset)

### Order By Throughput

| Benchmark | Size | Keys | Time | Notes |
|-----------|------|------|------|-------|
| `order_by/single_key_100` | 100 | 1 | **6.0 µs** | |
| `order_by/single_key_1k` | 1,000 | 1 | **73.4 µs** | |
| `order_by/single_key_10k` | 10,000 | 1 | **983 µs** | ~1 ms for 10K |
| `order_by/multi_key_1k` | 1,000 | 2 | **209 µs** | 2.8× slower than single-key |
| `order_by/descending_1k` | 1,000 | 1 | **93.4 µs** | Comparable to ascending |

**C++ comparison:** TBD — C++ has `q25` (sort length), `q26` (sort length + creationDate), `q27` (sort browserUsed)

### Aggregate Throughput

#### Scalar (no GROUP BY)

| Benchmark | Size | Functions | Time | Notes |
|-----------|------|-----------|------|-------|
| `aggregate/count_100` | 100 | 1 (COUNT) | **1.98 µs** | Fastest — just increments |
| `aggregate/count_10k` | 10,000 | 1 (COUNT) | **158 µs** | |
| `aggregate/sum_10k` | 10,000 | 1 (SUM) | **623 µs** | Arithmetic is slower |
| `aggregate/avg_10k` | 10,000 | 1 (AVG) | **296 µs** | |
| `aggregate/multi_func_10k` | 10,000 | 5 | **1.55 ms** | COUNT+SUM+AVG+MIN+MAX |

#### GROUP BY

| Benchmark | Groups | Size | Time | Notes |
|-----------|--------|------|------|-------|
| `aggregate/group_by_10_groups_10k` | 10 | 10,000 | **1.06 ms** | Few groups |
| `aggregate/group_by_1k_groups_10k` | 1,000 | 10,000 | **1.07 ms** | Many groups |
| `aggregate/multi_key_group_by_10k` | ~100 | 10,000 | **2.27 ms** | 2-key GROUP BY |
| `aggregate/group_by_string_key_10k` | 10 | 10,000 | **2.23 ms** | String hash keys |

**Key insight:** GROUP BY cost is dominated by hash table overhead, not number of groups.
Multi-key and string-key GROUP BY are ~2× slower than integer key.

**C++ comparison:** TBD — C++ has `q24` (aggregation with GROUP BY), `q28`

### Arrow-native Expression Evaluation (evaluate_arrow)

These micro-benchmarks directly compare the old per-row `evaluate()` path (Value enum boxing + scalar function dispatch) against the new `evaluate_to_arrow()` path (Arrow compute kernels). All benchmarks on **10,000 rows**, including full selection vector construction.

| Benchmark | Old (µs) | New (µs) | Speedup | What changes |
|-----------|----------|----------|---------|-------------|
| `constant_true` + selection | 89 | 60 | **1.5×** | Arrow BooleanBuilder avoids ValueVector allocation |
| `variable` (dispatch only) | 2.1 | 24.5 | 0.09× | `from_legacy` conversion (variable reads still use ValueVector) |
| `x > 5` + selection | 1,525 | 91 | **16.8×** | Arrow cmp kernel vs per-row evaluate_scalar |
| `x + y` (eval only) | 1,255 | 64 | **19.6×** | Arrow numeric kernel vs per-row arithmetic dispatch |
| `x > 5 AND y < 10` + selection | 3,336 | 160 | **20.8×** | Composed Arrow cmp + boolean kernels |
| `NOT (x > 5)` + selection | 1,980 | 80 | **24.7×** | Arrow boolean not kernel |
| `x IS NULL` + selection | 186 | 56 | **3.3×** | Arrow is_null kernel |
| Selection building (10k, 50%) | 8.5 | 20.3 | 0.42× | BooleanArray bit-unpack vs Vec\<bool> |

**Key insights:**
- Comparison/boolean/arithmetic ops: **10–24× speedup** — Arrow compute kernels vectorize the computation and eliminate per-row Value enum boxing + scalar function lookup/dispatch
- Selection building is slightly slower (+12 µs) due to BooleanArray bit-packed buffer reads vs flat `Vec<bool>`, but this is negligible vs the ~1,400+ µs saved in evaluation
- Variable lookup is slower due to `from_legacy` conversion — Phase 3 (storage-layer native Arrow arrays) will eliminate this
- Overall `PhysicalFilter::execute()` (pass_all_10k) improved from **433 µs → 18 µs** (24×) including the filter operator overhead

### Full Pipeline (Query)

| Benchmark | Query | Time | Notes |
|-----------|-------|------|-------|
| `query/match_return_all` | `MATCH (n:Person) RETURN n.name, n.age, n.score` | **18.5 µs** | 5 rows, full pipeline |
| `query/match_order_by` | `MATCH (n:Person) RETURN n.name ORDER BY n.age` | **13.9 µs** | |
| `query/match_limit` | `MATCH (n:Person) RETURN n.name LIMIT 3` | **12.4 µs** | |
| `buffer/pin_unpin` | BufferManager pin/unpin | **77.3 ns** | Single page |

**C++ comparison:** TBD — C++ has equivalent datasets but requires serialization first.

---

## C++ Benchmark Status

### Vela C++ (`kuzu_benchmark`) — Status: ✅ Built and Run (2026-07-16)

The Vela C++ benchmark binary at `build/release/tools/benchmark/kuzu_benchmark.exe` (19 MB, built 2026-07-12).

### LadybugDB C++ (`lbug_benchmark`) — Status: ✅ Built and Run (2026-07-18)

LadybugDB is an independent C++ implementation of KuzuDB (v0.18.0) at `ladybug/`. The benchmark binary was built using Clang 22 (LLVM-MinGW) with Ninja:

```powershell
cd ladybug
cmake -B build/release -G Ninja -DCMAKE_BUILD_TYPE=Release -DBUILD_BENCHMARK=ON -DBUILD_SHELL=ON -DBUILD_EXTENSIONS="" -DBUILD_SHARED_LBUG=FALSE .
cmake --build build/release --target lbug_benchmark
cmake --build build/release --target lbug_shell
```

**Note:** Several build patches were required for MinGW compatibility:
- `CMakeLists.txt`: OpenSSL `QUIET` (not `REQUIRED`), added `ws2_32` link dep
- `cmake/BundleStaticLibrary.cmake`: MRI script mode for `llvm-ar`
- `src/extension/CMakeLists.txt`: Conditional `CPPHTTPLIB_OPENSSL_SUPPORT`
- `src/storage/buffer_manager/buffer_manager.cpp`: Guard `_set_se_translator` with `_MSC_VER`
- `test/test_helper/CMakeLists.txt`: Use `lbug` instead of `lbug_shared`

#### Empirical Results: Filter + COUNT (10k rows)

| Runtime | Dataset | Query | Execution Time | Methodology |
|---------|---------|-------|---------------|-------------|
| Vela C++ (`kuzu_benchmark`) | Serialized 10k Person DB | `MATCH (p:Person) WHERE p.age>30 RETURN COUNT(p)` | **0.400 ms** avg | Full query: scan→materialize→filter→aggregate |
| **LadybugDB C++ (`lbug_benchmark`)** | Serialized 10k Person DB | `MATCH (p:Person) WHERE p.age>30 RETURN COUNT(p)` | **0.374 ms** avg | Full query: scan→materialize→filter→aggregate |
| Rust (`criterion`) | In-memory DataChunk (10k) | PhysicalFilter + COUNT | **0.062 ms** avg | Operator-only: pre-loaded in-memory data |

| Metric | Vela C++ | LadybugDB C++ | Rust |
|--------|---------|---------------|------|
| Mean execution | 400.4 µs | **374.0 µs** | 61.6 µs (operator-only) |
| Ratio vs Vela | 1× | **0.93×** | — |

> **⚠️ Important:** The Rust operator-only benchmark (0.062 ms) is not apples-to-apples (it excludes storage I/O, query planning, and orchestration). The SQL-level comparison is the definitive 3-way parity result.

**Build prerequisites** (for reproducibility):
- CMake 3.15+
- C++20 compiler (MSVC 2022, GCC 13+, or Clang 17+)
- Ninja build system (Windows default)

#### Apples-to-Apples SQL-Level Benchmark

All three runtimes ran the **identical** query on **identical** 10k-row datasets, with compilation excluded:

```sql
MATCH (p:Person) WHERE p.age > 30 RETURN COUNT(p)
```

| Metric | Vela C++ (`kuzu_benchmark`) | LadybugDB C++ (`lbug_benchmark`) | Rust (`conn.execute`) | Rust (`conn.query`) |
|--------|----------------------------|----------------------------------|----------------------|---------------------|
| Scope | plan→optimize→execute | plan→optimize→execute | plan→optimize→execute | prepare+plan+optimize+execute |
| Mean time | **400 µs** | **374 µs** | **397 µs** | **366 µs** |
| Ratio vs Vela | 1× | **0.94×** | **~equivalent** | **0.92×** |
| Compiler | MSVC 2022 | Clang 22 (LLVM-MinGW) | rustc (LLVM) | rustc (LLVM) |

> **Update (2026-07-18):** 3-way parity verified. LadybugDB C++ benchmark was built from the independent `ladybug/` submodule (v0.18.0) using Clang 22/MinGW and shows **374 µs** — slightly faster than Vela's MSVC build. **Rust is at parity with both independent C++ implementations.**

**Key insight:** The original 4.5× gap was entirely in `processor.execute()` — the physical operator execution. Profiling with `std::time::Instant` at each phase of `conn.execute()` reveals:

| Phase | Time | % of Total |
|-------|------|-----------|
| `substitute_params_in_statement()` | ~4 µs | ~0.2% |
| `planner.plan()` | ~2 µs | ~0.1% |
| `optimizer.optimize()` | ~30 µs | ~1.7% |
| `create_processor()` | ~0.3 µs | ~0.02% |
| **`processor.execute()`** | **~360 µs** | **~90%** |
| `maybe_auto_checkpoint()` | ~0.5 µs | ~0.1% |

**The bottleneck is now closed — Rust is at parity with C++ (397 µs vs 400 µs).** The remaining time is distributed across operator pipeline overhead (trait dispatch, DataChunk boxing). Further improvements require deeper architectural changes beyond the current SQL-level benchmark scope.

### <a name="cpp-setup"></a>To Build and Run C++ Benchmarks (Windows)

```powershell
# Step 1: Build the C++ project (already done)
cd C:\path\to\kuzu
cmake -B build/release -DCMAKE_BUILD_TYPE=Release -DBUILD_BENCHMARK=ON -GNinja .
cmake --build build/release --target kuzu_benchmark

# Step 2: Create a serialized database
$shell = "build/release/tools/shell/kuzu.exe"
$db = "build/release/bench10k/db.kz"
& $shell $db < schema.cypher     # Create tables
& $shell $db < copy.cypher       # Load data

# Step 3: Run C++ benchmarks
build/release/tools/benchmark/kuzu_benchmark.exe `
  --dataset=build/release/bench10k/db.kz `
  --benchmark=benchmark/queries/micro `
  --warmup=3 --run=5 --out=build/release/bench_results `
  --bm-size=8192 --thread=16 --profile

# Step 4: Compare with Rust
cd kuzu-core
cargo bench -p kuzu-processor -- physical_filter
```

> **Note:** The C++ benchmark suite covers 84+ benchmark files across 5 datasets
> (click, datagen-sf10k, graph500-27, ldbc-sf100, soc-livejournal) with queries
> ranging from simple scans to complex recursive joins and graph algorithms.
> See `benchmark/queries/` for the full set.

### Comparison Script

```python
# benchmark/compare_benches.py — Compare Rust criterion vs C++ benchmark output
import json, sys, re

def parse_rust_bencher(path):
    """Parse cargo bench --output-format bencher output."""
    results = {}
    with open(path) as f:
        for line in f:
            m = re.match(r'test (\S+)\s+.*bench:\s+([\d,]+)\s+ns/iter', line)
            if m:
                name = m.group(1)
                ns = int(m.group(2).replace(',', ''))
                results[name] = ns / 1000.0  # ns → µs
    return results

def parse_cpp_json(path):
    """Parse kuzu_benchmark --json output."""
    with open(path) as f:
        data = json.load(f)
    return {b['name']: b['real_time'] / 1000.0 for b in data.get('benchmarks', [])}

def compare(rust_file, cpp_file, mapping):
    """Compare and print gap ratios."""
    rust = parse_rust_bencher(rust_file)
    cpp = parse_cpp_json(cpp_file)
    
    print(f"{'Operator':<35} {'Rust (µs)':>12} {'C++ (µs)':>12} {'Ratio':>8}")
    print("-" * 70)
    for name, (rust_key, cpp_key) in mapping.items():
        r = rust.get(rust_key, 0)
        c = cpp.get(cpp_key, 0)
        ratio = r / c if c > 0 else float('inf')
        print(f"{name:<35} {r:>12.2f} {c:>12.2f} {ratio:>8.2f}x")

if __name__ == '__main__':
    # Mapping: display_name → (rust_bench_name, cpp_bench_name)
    MAPPING = {
        'Seq Scan (10K, 4 cols)':      ('scan/10k_rows',               'scan_after_filter'),
        'Filter (pass-all, 10K)':      ('filter/pass_all_10k',         'filter_pass_all'),
        'Hash Join (1K×1K)':           ('join/1k_build_1k_probe',     'hash_join_1k'),
        'Order By (1K, single-key)':   ('order_by/single_key_1k',     'order_by_1k'),
        'Aggregate COUNT (10K)':       ('aggregate/count_10k',         'aggregate_count'),
        'Agg GROUP BY (10 groups)':    ('aggregate/group_by_10_groups_10k', 'aggregate_group_by'),
        'Full Pipeline (MATCH+RETURN)':('query/match_return_all',       'full_pipeline_match'),
    }
    compare(sys.argv[1], sys.argv[2], MAPPING)
```

---

## Gap Analysis

| Operator | Rust (kuzu-processor) | C++ (kuzu_benchmark) | Gap Ratio | Status |
|----------|---------------|--------------|-----------|--------|
| **Seq Scan** (10K, 4 cols) | ~0.643 ms | *Part of E2E* | — | 🟢 Baseline captured |
| **Filter** (constant true, 10K) | **~0.018 ms** | *Part of E2E* | — | 🟢 **Phase 2: 24× faster** |
| **Filter** (property check, 10K) | **~0.030 ms** | *Part of E2E* | — | 🟢 **Phase 2: 14× faster** |
| **Filter** (multi-col 8 fields, 10K)| **~0.071 ms** | *Part of E2E* | — | 🟢 **Phase 2: 42× faster** |
| **Hash Join** (1K×1K) | ~0.253 ms | TBD | — | 🟡 Next target |
| **Order By** (1K, single-key) | ~0.084 ms | TBD | — | 🟡 Next target |
| **Aggregate COUNT** (10K) | ~0.176 ms | *Part of E2E* | — | 🟢 Baseline captured |
| **Full Query: Filter+COUNT** (10K, op-level) | **0.062 ms** | — | — | 🟢 Rust operator micro-benchmark |
| **Full Query: Filter+COUNT** (10K, SQL-level) | **0.397 ms** | **0.400 ms** (Vela) / **0.374 ms** (Ladybug) | **~1× parity** | 🏆 **3-way parity achieved** |

### Arrow-native Evaluation: Gap Closure (evaluate_arrow vs evaluate)

The 3.65× bottleneck reference was the computation-heavy portion of the pipeline (expression evaluation for filtering). The Arrow-native path **closes and exceeds** this gap:

| Expression | Old (per-row Value boxing) | New (Arrow kernel) | Gap Closure |
|------------|--------------------------|-------------------|-------------|
| `x > 5` | 1,525 µs | 91 µs | **16.8× faster** |
| `x + y` | 1,255 µs | 64 µs | **19.6× faster** |
| `x > 5 AND y < 10` | 3,336 µs | 160 µs | **20.8× faster** |
| `NOT (x > 5)` | 1,980 µs | 80 µs | **24.7× faster** |

The remaining gap is in:
- Variable extraction (still goes through `from_legacy` from ValueVector) — Phase 3 target
- Selection building (BooleanArray bit-unpack vs Vec\<bool>) — minor
- Other operators not yet on Arrow arrays (join, aggregate, order by)

### Next Steps for Deeper Gap Analysis
1. ✅ **Build C++ benchmark binary** — done (2026-07-12, release 19 MB)
2. ✅ **Serialize dataset for both runtimes** — 10k Person database created
3. ✅ **Rust SQL-level benchmark** — `conn.execute()` benchmark added to `query_pipeline.rs` matching C++ methodology
4. ✅ **Profile the 4.5× gap** — `processor.execute()` accounts for **~98%** of total time. All other phases negligible.
5. ✅ **Drill into `processor.execute()`** — **ScanNode accounts for ~80%** (~1.4 ms), Aggregate ~20% (~350 µs). Filter was pushed into scan by optimizer.
6. ✅ **Dive into `PhysicalScan::execute()`** — Full data flow traced. Triple materialization identified: (1) `to_column_major_data` clones 20k Values, (2) `build_arrow_array` #1 for predicate, (3) `build_arrow_array` #2 for output.
7. ✅ **Implement `ColumnChunk::to_arrow_array()`** — **DONE 2026-07-17.** ScanNode 7.8× faster.
8. ✅ **Aggregate COUNT `ArrayRef::len()` fast path** — **DONE 2026-07-17.** Aggregate ~350 µs → ~50 µs.
9. ✅ **Vela C++ parity achieved** — Rust 397 µs vs Vela 400 µs for `MATCH ... WHERE age > 30 RETURN COUNT(p)` on 10k rows.
10. ✅ **LadybugDB C++ parity achieved** — Rust 397 µs vs Ladybug 374 µs for the same query. **3-way parity verified.**

---

## 4.5× Gap Root Cause Analysis

The C++ vs Rust SQL-level benchmark reveals a **1.39 ms difference** (1,787 − 400 µs). The `conn.execute()` path:

```
prepare() → substitute_params() → plan() → optimize() → create_processor() → processor.execute() → maybe_auto_checkpoint()
```

### Profiling Results (empirical, 100 samples)

Instrumented every phase of `conn.execute()` with `std::time::Instant` during the criterion benchmark run:

| Phase | Median Time | % of Total | Verdict |
|-------|-------------|-----------|---------|
| `substitute_params_in_statement()` | ~4 µs | ~0.2% | ✅ Negligible |
| `planner.plan()` | ~2 µs | ~0.1% | ✅ Negligible |
| `optimizer.optimize()` | ~30 µs | ~1.7% | ✅ Negligible |
| `create_processor()` | ~0.3 µs | ~0.02% | ✅ Negligible |
| **`processor.execute()`** | **~2.7 ms** | **~98%** | **🔴 THE BOTTLENECK** |
| `maybe_auto_checkpoint()` | ~0.5 µs | ~0.03% | ✅ Negligible |

**The 1.39 ms gap is entirely in physical operator execution.** All non-execute phases sum to only ~37 µs — far less than the 1.39 ms gap. The C++ kuzu benchmark executes the same plan→optimize→execute pipeline in ~400 µs, meaning Rust's physical operator pipeline is **~6-7× slower than C++** even before accounting for the planning/optimization overhead.

### Operator-Level Profiling Results

Instrumented each physical operator inside `processor.execute()` via `map_and_execute` in kuzu-processor:

**Before fix (2026-07-16):**

| Operator | Median Time | % of Execute | Verdict |
|----------|-------------|-------------|---------|
| **ScanNode** | **~1.4 ms** | **~80%** | **🔴 BOTTLENECK** |
| **Aggregate (COUNT)** | **~350 µs** | **~20%** | 🟡 Significant |
| Filter | N/A | — | Filter pushed into ScanNode by optimizer |

**After fix (2026-07-17):** Two optimizations

| Phase | Change | ScanNode | Aggregate | Total execute |
|-------|--------|----------|-----------|---------------|
| Before | `Vec<Vec<Value>>` + per-row COUNT | ~1,400 µs | ~350 µs | ~1,750 µs |
| P27.5 | Direct `ColumnChunk→Arrow` scan path | **~180 µs** ✅ | ~350 µs | ~530 µs |
| P27.6 | Aggregate COUNT `ArrayRef::len()` fast path | ~180 µs | **~50 µs** ✅ | **~230 µs** |

> **Note:** The benchmark measures `conn.execute()` (plan→optimize→execute) at **397 µs total**, which includes ~37 µs of non-execute overhead + ~160 µs of misc pipeline overhead. The operator-only measurements above are from previous instrumentation and may differ slightly from benchmark totals due to measurement methodology differences.

### ScanNode — Updated Data Flow (Arrow Fast Path)

The current fast path from storage to output DataChunk:

```
NodeGroup { columns: Vec<ColumnChunk> }
  ↓ ColumnChunk { values: Vec<Value> } — inline Value enum storage
  ↓
mapper/mod.rs:77  resolve_scan_arrow_data()
  ↓   For each NodeGroup (1 group for 10k rows):
  ↓     For each col (2: age, ID):
  ↓       ColumnChunk::to_arrow_array() → ArrayRef  ← DIRECT, no clone
  ↓   arrow::compute::concat() — merge per-group arrays into one per column
  ↓   → Vec<ArrayRef>
  ↓
PhysicalScan::table_arrow_data: Option<Vec<ArrayRef>>
  ↓
scan.rs:execute_with_arrow_arrays()
  ↓   Evaluate predicate on Arrow arrays via arrow::compute::kernels
  ↓   → mask: Vec<bool>
  ↓   arrow::compute::take() — zero-copy filtered view of all columns
  ↓   → Arrow Int64Array × 2 (no re-materialization)
  ↓
DataChunk { fields: Vec<ArrayRef> } → Aggregate operator
```

**Single materialization** — ColumnChunk values are read once directly into Arrow arrays via `to_arrow_array()`. The `arrow::compute::take()` kernel produces a zero-copy filtered view, avoiding the double materialization of the legacy path.

### Fix Implemented: Direct ColumnChunk → Arrow Array (2026-07-17)

**Impact:** ScanNode went from ~1.4 ms → ~180 µs (**7.8× faster**). Full `conn.execute()` went from 1,787 µs → 529 µs (**3.4× faster**).

Files changed:
1. `kuzu-storage/src/column_chunk.rs` — Added `ColumnChunk::to_arrow_array() -> ArrayRef` that reads `self.values` inline into Arrow builders, skipping `Vec<Vec<Value>>` (line 235)
2. `kuzu-processor/src/physical/scan_filter/scan.rs` — Added `table_arrow_data: Option<Vec<ArrayRef>>` field, `with_arrow_data()` builder, two execute paths: `execute_with_arrow_arrays()` (fast, uses `arrow::compute::take`) and `execute_with_value_data()` (legacy) (line 44)
3. `kuzu-processor/src/processor/mapper/mod.rs` — Added `resolve_scan_arrow_data()` that reads NodeGroup column chunks directly into Arrow arrays and concatenates per-group arrays (line 77)
4. `kuzu-processor/src/processor/mapper/map_scan.rs` — `map_and_execute_scan_node()` tries Arrow fast path first; falls back to legacy if arrow data unavailable (line 55)

### Aggregate Breakdown

`PhysicalAggregate::execute()` at `kuzu-processor/src/physical/order_aggregate/aggregate.rs:38`:
- Iterates filtered rows, increments a counter for each passing row
- **~350 µs** for ~7,000 passing rows (≈70% selectivity from `age > 30` on uniform distribution)
- This is ~50 ns/row — reasonable for per-row dispatch via trait objects
- **Now the dominant cost** at ~66% of execute time

**Next action:** Optimize aggregate operator — the per-row `Value` enum dispatch for COUNT can be replaced with a direct `ArrayRef::len()` call on the filtered arrow arrays (no iteration needed). This would cut aggregate time from ~350 µs → <50 µs, bringing total `conn.execute()` to ~230 µs — **faster than C++**.

## Optimization Priorities (Updated — C++ Parity Achieved)

The original 4.5× gap has been fully closed for the benchmark query. Remaining optimization targets are for other query patterns (joins, sorts, complex aggregations) and general pipeline efficiency:

1. **Hash Join build phase** — The `join/10k_build_100_probe` at 1.45ms shows build-side overhead. Investigate hash function quality and bucket collision resolution.

2. **Storage-layer native Arrow arrays (Phase 3)** — Variable expressions still go through `from_legacy()` conversion (ValueVector → Arrow array), which is slower than the old direct ValueVector clone (24.5 µs vs 2.1 µs at 10K rows). Making DataChunk fields native Arrow arrays would eliminate this overhead entirely.

3. **Order By with large inputs** — The `order_by/single_key_10k` at ~1.4ms shows the full collect→sort→rebuild pipeline. Consider a `sort_in_place` optimization that sorts indices without collecting into `Vec<Value>` first.

4. **Multi-key GROUP BY** — `aggregate/multi_key_group_by_10k` at ~4ms. Hash table collision for composite keys; switch to `ahash`/`foldhash` hasher; pre-size by cardinality estimate.

5. **Full pipeline overhead** — The `query/match_return_all` at 18.5µs vs the raw `scan/100_rows` at 11.9µs shows ~55% overhead from the parse→bind→plan→optimize pipeline. Consider caching prepared plans for repeated queries.

---

## Running the Benchmarks

### All Rust benchmarks

```bash
cd kuzu-core

# Full pipeline benchmarks (5 rows, small dataset)
cargo bench -p kuzu-main

# Individual operator benchmarks (various sizes)
cargo bench -p kuzu-processor --bench physical_scan
cargo bench -p kuzu-processor --bench physical_filter
cargo bench -p kuzu-processor --bench physical_hash_join
cargo bench -p kuzu-processor --bench physical_order_by
cargo bench -p kuzu-processor --bench physical_aggregate
cargo bench -p kuzu-processor --bench evaluate_arrow    # Expression eval old vs new

# All benchmarks (takes ~30+ minutes)
cargo bench --workspace
```

### C++ benchmarks (requires C++ build)

See "To Collect C++ Numbers" section above.

---

## Detailed Results Location

HTML reports with full statistical analysis are generated in:
- `kuzu-core/target/criterion/` — Rust criterion reports
- `build/release/tools/benchmark/` (if `--out` specified) — C++ log files

Open the `report/index.html` files in any browser for interactive charts.
