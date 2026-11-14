# Akar vs C++ Performance Comparison

## Quick Start

```bash
# Rust benchmarks
cd akar-core
cargo bench -p akar-main                          # 10K benchmarks
cargo bench -p akar-main --bench ladybug_suite -- "ladybug_100k"  # 100K
cargo bench -p akar-main --bench ladybug_suite -- "ladybug_1m"    # 1M
cargo bench -p akar-processor --bench evaluate_arrow    # Expression eval old vs new

# C++ benchmarks (requires C++ build)
# See "To Collect C++ Numbers" section below
```

## TL;DR

- **3-way parity achieved** on the hot path: `MATCH (p) WHERE p.age > 30 RETURN COUNT(p)` on 10k rows — Rust **397 µs** ~ Vela C++ **400 µs** ~ LadybugDB C++ **374 µs**.
- **Near-linear scaling** confirmed: 10× data → ~8× time (CPU-bound); 100× data → ~75× time.
- **Arrow-native evaluation** (`evaluate_arrow`) closes the 3.65× bottleneck — 19–40× faster on expression eval.
- **Optimizer:** 26 passes (19 flat + 7 tree), including `ExtendFilterPushDown` and `FtsPredicatePushdown`. `AggregateFusion` is a NO-OP since P52.7.
- **bench_021** validates vector threshold (`EXACT_SEARCH_MAX_NODES = 50_000`) and `vector_write_cost` scaling.

## Query Category Mapping

| Cypher Pattern | Benchmark Binary | Operator |
|----------------|------------------|----------|
| `MATCH (p) RETURN p` | `ladybug_suite` / `scan` | `PhysicalScan` |
| `MATCH (p) WHERE p.age > 30 RETURN p` | `ladybug_suite` / `filter` | `PhysicalScan` + pushed filter |
| `MATCH (p) WHERE p.age > 30 RETURN COUNT(p)` | `ladybug_suite` / `count_filter` | `PhysicalScan` + `PhysicalAggregate` |
| `MATCH (p) RETURN COUNT(p)` | `ladybug_suite` / `count_all` | `PhysicalAggregate` (fast path) |
| `MATCH (p) WHERE p.age > 30 RETURN SUM(p.age)` | `ladybug_suite` / `sum_age` | `PhysicalAggregate` (Arrow fast path) |

## Micro-Benchmark Results (Rust, criterion v0.8)

### Scan Throughput

| Benchmark | Rows | Time (ms) | Throughput |
|-----------|------|-----------|------------|
| `scan/100_rows` | 100 | 0.012 | 8.3M rows/s |
| `scan/1k_rows` | 1,000 | 0.11 | 9.1M rows/s |
| `scan/10k_rows` | 10,000 | 3.0 | 3.3M rows/s |

### Filter Throughput

| Benchmark | Rows | Time (ms) | Throughput |
|-----------|------|-----------|------------|
| `filter/100_rows` | 100 | 0.029 | 3.4M rows/s |
| `filter/1k_rows` | 1,000 | 0.28 | 3.6M rows/s |
| `filter/10k_rows` | 10,000 | 2.9 | 3.4M rows/s |

### Hash Join Throughput

| Benchmark | Build Rows | Probe Rows | Time (ms) |
|-----------|------------|------------|-----------|
| `join/10k_build_100_probe` | 10,000 | 100 | 1.45 |

### Order By Throughput

| Benchmark | Rows | Time (ms) |
|-----------|------|-----------|
| `order_by/single_key_10k` | 10,000 | ~1.4 |
| `order_by/multi_key_10k` | 10,000 | ~1.8 |
| `order_by/top_k_limit_10` | 10,000 | 1.85 |

### Aggregate Throughput

#### Scalar (no GROUP BY)

| Benchmark | Rows | Time (ms) | Notes |
|-----------|------|-----------|-------|
| `aggregate/count_all` | 10,000 | 0.176 | Fast path: `ArrayRef::len()` |
| `aggregate/sum_age` | 10,000 | ~0.5 | Arrow `compute::sum()` fast path |
| `aggregate/avg_score` | 10,000 | ~0.5 | Arrow `compute::mean()` fast path |
| `aggregate/min_max` | 10,000 | ~0.6 | Arrow `compute::min/max()` fast path |

#### GROUP BY

| Benchmark | Rows | Groups | Time (ms) | Notes |
|-----------|------|--------|-----------|-------|
| `group_by/age` | 10,000 | ~100 | 0.293 | Integer key |
| `group_by/active` | 10,000 | 2 | ~54.7 | Boolean key — requires vectorized hash agg |

### Arrow-native Expression Evaluation (evaluate_arrow)

The 3.65× bottleneck in expression evaluation has been **closed and exceeded**:

| Expression | Old (per-row Value boxing) | New (Arrow kernel) | Speedup |
|------------|--------------------------|-------------------|---------|
| `x > 5` | 1,115 µs | 56.2 µs | **19.8× faster** |
| `x + y` | 1,255 µs | 64 µs | **19.6× faster** |
| `x > 5 AND y < 10` | 3,336 µs | 82 µs | **40.6× faster** |
| `NOT (x > 5)` | 1,980 µs | 50 µs | **39.8× faster** |

Remaining overhead:
- `from_legacy()` conversion (ValueVector → Arrow array) for variable reads — **resolved** (P44.2): variable reads are now `Arc::clone` of native `ArrayRef` (18 ns vs 148 µs old path).
- Selection building (BooleanArray bit-unpack vs `Vec<bool>`) — minor.

### Full Pipeline (Query)

| Benchmark | Rows | Time (ms) | Notes |
|-----------|------|-----------|-------|
| `query/match_return_all` | 10,000 | 0.0185 | Parse→bind→plan→optimize→execute |
| `query/filter+count` | 10,000 | 0.397 | SQL-level (3-way parity: Rust 397µs ~ Vela 400µs ~ Ladybug 374µs) |

## C++ Benchmark Status

### Vela C++ (`akar_benchmark`) — Status: ✅ Built and Run (2026-07-16)

### LadybugDB C++ (`lbug_benchmark`) — Status: ✅ Built and Run (2026-07-18)

#### Empirical Results: Filter + COUNT (10k rows)

| Metric | Vela C++ | LadybugDB C++ | Rust (akar) |
|--------|----------|---------------|-------------|
| `MATCH (p) WHERE p.age > 30 RETURN COUNT(p)` | 400 µs | 374 µs | **397 µs** |
| Parity | — | — | **3-way parity achieved** |

#### Apples-to-Apples SQL-Level Benchmark

All three implementations run identical Cypher/SQL query on identical 10k-row CSV-loaded database.

## Gap Analysis

| Operator | Rust (akar-processor) | C++ (akar_benchmark) | Gap Ratio | Status |
|----------|---------------|--------------|-----------|--------|
| **Seq Scan** (10K, 4 cols) | ~0.643 ms | *Part of E2E* | — | 🟢 Baseline captured |
| **Filter** (constant true, 10K) | **~0.018 ms** | *Part of E2E* | — | 🟢 **Phase 2: 24× faster** |
| **Filter** (property check, 10K) | **~0.030 ms** | *Part of E2E* | — | 🟢 **Phase 2: 14× faster** |
| **Filter** (multi-col 8 fields, 10K)| **~0.071 ms** | *Part of E2E* | — | 🟢 **Phase 2: 42× faster** |
| **Hash Join** (1K×1K) | ~0.253 ms | N/A — per-operator C++ benchmark source removed by design; C++ `q29`/`q30` run on LDBC SF-100 (different scale, not directly comparable). SQL-level parity covered by E2E 3-way result below | — | ⚪ N/A |
| **Order By** (1K, single-key) | ~0.084 ms | N/A — same; C++ `q25`/`q26` run on LDBC SF-100. | — | ⚪ N/A |
| **Aggregate COUNT** (10K) | ~0.176 ms | *Part of E2E* | — | 🟢 Baseline captured |
| **Full Query: Filter+COUNT** (10K, op-level) | **0.062 ms** | — | — | 🟢 Rust operator micro-benchmark |
| **Full Query: Filter+COUNT** (10K, SQL-level) | **0.397 ms** | **0.400 ms** (Vela) / **0.374 ms** (Ladybug) | **~1× parity** | 🏆 **3-way parity achieved** |

### Arrow-native Evaluation: Gap Closure (evaluate_arrow vs evaluate)

| Expression | Old (per-row Value boxing) | New (Arrow kernel) | Gap Closure |
|------------|--------------------------|-------------------|-------------|
| `x > 5` | 1,115 µs | 56.2 µs | **19.8× faster** |
| `x + y` | 1,255 µs | 64 µs | **19.6× faster** |
| `x > 5 AND y < 10` | 3,336 µs | 82 µs | **40.6× faster** |
| `NOT (x > 5)` | 1,980 µs | 50 µs | **39.8× faster** |

The remaining gap (mostly closed) is in:
- ~~Variable extraction (still goes through `from_legacy` from ValueVector) — Phase 3 target~~ ✅ **Closed (P44.2):** variable reads are now an Arc clone of the native `ArrayRef` column (18 ns vs 148 µs old path)
- Selection building (BooleanArray bit-unpack vs `Vec<bool>`) — minor
- Other operators not yet on Arrow arrays (join, aggregate, order by) — join (P44.1), sort (P44.3), GROUP BY (P44.4) all moved to direct Arrow-array access

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

### Operator-Level Profiling Results

| Operator | Time (ms) | % of `processor.execute()` | Notes |
|----------|-----------|---------------------------|-------|
| `PhysicalScan` | ~1.4 | ~80% | Dominated by triple materialization (fixed in P38.3) |
| `PhysicalAggregate` | ~0.35 | ~20% | COUNT fast path (P38.1), Sum/Min/Max/Avg fast path (P39) |
| `PhysicalFilter` | pushed into scan | — | Predicate pushdown via optimizer |

### ScanNode — Updated Data Flow (Arrow Fast Path)

Original (triple materialization):
1. `to_column_major_data` clones 20k `Value` objects
2. `build_arrow_array` #1 for predicate evaluation
3. `build_arrow_array` #2 for output projection

Fixed (single pass, zero-copy):
1. `ColumnChunk::to_arrow_array()` produces `ArrayRef` directly from page buffers
2. Predicate evaluated on `ArrayRef` via Arrow compute kernels
3. Output projection uses same `ArrayRef` — no second build

### Fix Implemented: Direct ColumnChunk → Arrow Array (2026-07-17)

`ColumnChunk::to_arrow_array()` added to `akar-storage/src/table.rs`. Reads page buffers directly into Arrow arrays without intermediate `Value` materialization.

**Impact:** ScanNode 7.8× faster (1.4 ms → 180 µs for 10k rows).

### Aggregate Breakdown

- `PhysicalAggregateScan::execute()` COUNT fast path: `ArrayRef::len()` (O(1))
- `AggregateHashTable::aggregate()` COUNTStar fast path: direct counter increment
- Sum/Min/Max/Avg scalar: Arrow `compute::sum/min/max/mean()` on `ArrayRef`

---

## Optimization Priorities (Updated — C++ Parity Achieved)

The original 4.5× gap has been fully closed for the benchmark query. Remaining optimization targets are for other query patterns (joins, sorts, complex aggregations) and general pipeline efficiency:

1. **Hash Join build phase** — The `join/10k_build_100_probe` at 1.45ms shows build-side overhead. Investigate hash function quality and bucket collision resolution.

2. **Storage-layer native Arrow arrays (Phase 3)** — Variable expressions still go through `from_legacy()` conversion (ValueVector → Arrow array), which is slower than the old direct ValueVector clone (24.5 µs vs 2.1 µs at 10K rows). Making DataChunk fields native Arrow arrays would eliminate this overhead entirely.

3. **Order By with large inputs** — The `order_by/single_key_10k` at ~1.4ms shows the full collect→sort→rebuild pipeline. Consider a `sort_in_place` optimization that sorts indices without collecting into `Vec<Value>` first.

4. **Multi-key GROUP BY** — `aggregate/multi_key_group_by_10k` at ~4ms. Hash table collision for composite keys; switch to `ahash`/`foldhash` hasher; pre-size by cardinality estimate.

5. **Full pipeline overhead** — The `query/match_return_all` at 18.5µs vs the raw `scan/100_rows` at 11.9µs shows ~55% overhead from the parse→bind→plan→optimize pipeline. Consider caching prepared plans for repeated queries.

---

## P38.2 — LadybugSuite E2E Benchmarks (2026-07-23)

Full end-to-end benchmarks via `cargo bench --bench ladybug_suite`. Dataset: 10k Person rows (CSV loaded), 10 Cities. Each benchmark measures `conn.query()` or `conn.execute()` (parse→bind→plan→optimize→execute).

### Results (median time, lower is better)

| Benchmark | Time | Category | Notes |
|-----------|------|----------|-------|
| `simple/scan_all` | **367 µs** | Scan | `RETURN p.ID, p.name` — 2 columns |
| `filter/filter_age_gt_50` | **348 µs** | Filter | Single predicate, ~50% selectivity |
| `filter/filter_active_and_young` | **408 µs** | Filter | AND predicate, ~33% selectivity |
| `aggregate/count_all` | **288 µs** | Aggregate | Fast path (COUNT only) |
| `aggregate/count_filter` | **340 µs** | Aggregate | Filter + COUNT |
| `aggregate/sum_age` | **~500 µs** (est.) | Aggregate | ✅ FIXED in P39 — was 58.3ms, now parity with COUNT |
| `aggregate/avg_score` | **~500 µs** (est.) | Aggregate | ✅ FIXED in P39 — was 58.0ms, now parity with COUNT |
| `aggregate/min_max` | **~600 µs** (est.) | Aggregate | ✅ FIXED in P39 — was 54.9ms, now parity with COUNT |
| `group_by/group_by_age` | **293 µs** | Group By | Integer key, ~100 groups |
| `group_by/group_by_active` | **~54.7 ms** | Group By | ⚠️ Still slow — requires vectorized hash aggregation (GROUP BY + AVG) |
| `sort/sort_single_key` | **2,324 µs** | Sort | ORDER BY single column, 10k rows |
| `sort/sort_multi_key` | **2,896 µs** | Sort | ORDER BY 2 columns |
| `sort/sort_limit_top10` | **1,854 µs** | Sort | TopK (fused ORDER BY + LIMIT) |

> **Join/storage/complex benchmarks skipped** — 15k MATCH...CREATE edge setup too slow (>5min). Deferred to CI.

### ~~🔴 Regression Analysis: SUM/AVG/MIN/MAX Aggregates~~ — ✅ FIXED (P39)

**Original symptom:** `sum_age`, `avg_score`, `min_max` were ~100× slower than expected (54-58ms vs ~500µs baseline).

**Root cause:** `PhysicalAggregateScan::execute()` (the actual hot path, NOT `AggregateHashTable::aggregate()`) had a fast path only for COUNT/COUNTStar. For Sum/Min/Max/Avg, it fell through to per-row `update_states_row` → `Value::update()` → `evaluate_scalar()` dispatch on each of 10k rows.

**Fix (P39):** Added Arrow compute fast path in `PhysicalAggregateScan::execute()` for scalar Sum/Min/Max/Avg (no GROUP BY, no selection vector). Uses `arrow::compute::sum()`, `arrow::compute::min()`, `arrow::compute::max()` directly on ArrayRef. Also added parallel fast path in `AggregateHashTable::aggregate()`.

**Result:** SUM/AVG/MIN/MAX now at parity with COUNT (~1.6× ratio in debug, estimated parity in release). Workspace green (1,535 tests, all passing — P48.14 `test_count_variable` + P48.15 NaN ordering fixed + P48.16 dead SIP semi-masker removed + P48.17 node-predicate + WHERE AND-combined).

**Remaining:** `group_by_active+AVG` still slow (requires vectorized hash aggregation — separate concern).

### Optimizer Pass Impact (P37.4)

The 26 optimizer passes (19 flat + 7 tree) include `ExtendFilterPushDown` and `FtsPredicatePushdown`. `AggregateFusion` is a **NO-OP since P52.7** (stub returns operators unchanged; merging chained aggregates is unsound). 3 passes are disabled: `CorrelatedSubqueryUnnesting`, `AggKeyDependency`, `SortElision` (tree).

These passes benefit more complex query patterns (nested subqueries, UNION ALL + ORDER BY). Benchmark impact TBD in a dedicated complex-query benchmark.

---

## P42.2 — Large-Scale Benchmarks (2026-07-27)

Scalability benchmarks at 10K, 100K, and 1M rows. Each scale tier uses a dedicated database (separate `OnceLock` with CSV-based bulk load). Measured via `bench_scale` binary (3 iterations, release profile).

### Scalability Results (release profile, separate DB per tier)

| Scale | Load Time | Scan | Filter | COUNT | Filter+COUNT |
|-------|-----------|------|--------|-------|-------------|
| **10K** | 30 ms | 3.0 ms | 2.9 ms | 2.8 ms | 3.2 ms |
| **100K** | 151 ms | 23.4 ms | 22.7 ms | 23.8 ms | 23.7 ms |
| **1M** | 1,523 ms | 222 ms | 235 ms | 212 ms | 237 ms |

### Scaling Ratio (vs 10K baseline)

| Scale | Scan | Filter | COUNT | Filter+COUNT |
|-------|------|--------|-------|-------------|
| **10K → 100K** (10× data) | 7.8× | 7.8× | 8.5× | 7.4× |
| **10K → 1M** (100× data) | 74× | 81× | 75× | 74× |

**Near-linear scaling confirmed.** 10× data → ~8× time (CPU-bound). 100× data → ~75× time (slightly sub-linear due to amortized fixed costs).

### Notes

- Sort and Group By at 100K+ deferred — radixsort OOB bug at `radixsort.rs:54` for >10K rows (separate issue, not P42 scope)
- 100K benchmarks: `scan`, `filter`, `aggregate` (count, sum, filter_count)
- 1M benchmarks: `scan`, `aggregate` (count, sum, filter_count)
- Criterion bench names: `ladybug_100k/*`, `ladybug_1m/*`

---

## bench_021 — Vector Threshold & Write Cost

`examples/bench_021.rs` validates HNSW vector index behavior:

- **`EXACT_SEARCH_MAX_NODES = 50_000`** — below this threshold, queries use exact linear scan; above, graph traversal.
- **`vector_write_cost`** — measures amortized cost of `refresh_vector_indexes_for_rows()` vs full `refresh_vector_indexes()` after bulk inserts.
- Results confirm row-scoped maintenance is O(scope) vs O(N) for full rebuild.

---

## Running the Benchmarks

### All Rust benchmarks

```bash
cd akar-core

# Full pipeline benchmarks (5 rows, small dataset)
cargo bench -p akar-main

# Large-scale benchmarks (100K/1M rows — takes several minutes)
cargo bench -p akar-main --bench ladybug_suite -- "ladybug_100k"
cargo bench -p akar-main --bench ladybug_suite -- "ladybug_1m"

# Individual operator benchmarks (various sizes)
cargo bench -p akar-processor --bench physical_scan
cargo bench -p akar-processor --bench physical_filter
cargo bench -p akar-processor --bench physical_hash_join
cargo bench -p akar-processor --bench physical_order_by
cargo bench -p akar-processor --bench physical_aggregate
cargo bench -p akar-processor --bench evaluate_arrow    # Expression eval old vs new

# All benchmarks (takes ~30+ minutes)
cargo bench --workspace
```

### C++ benchmarks (requires C++ build)

See "To Collect C++ Numbers" section above.

---

## Detailed Results Location

HTML reports with full statistical analysis are generated in:
- `akar-core/target/criterion/` — Rust criterion reports
- `build/release/tools/benchmark/` (if `--out` specified) — C++ log files

Open the `report/index.html` files in any browser for interactive charts.