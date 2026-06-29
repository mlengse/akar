# Kuzu Rust Port — Benchmark Baseline

> **Date:** 2026-06-29
> **Workspace:** kuzu-core workspace, edition 2024, 16+ crates
> **Criterion:** v0.5, HTML reports at `target/criterion/`
> **Compiler:** rustc (stable) — release profile, LTO enabled

---

## Overview

This document establishes the **baseline performance** of the pure-Rust Kuzu port after Phases 1–5 (Columnar Storage, COPY FROM, Cypher Expansion, Operator Generalization, Benchmark Infrastructure). Future runs should compare against these numbers to detect regressions and track improvements.

### How to Re-run

```bash
cd kuzu-core

# Full pipeline (end-to-end queries)
cargo bench -p kuzu-main

# Individual operator micro-benchmarks
cargo bench -p kuzu-processor

# Everything
cargo bench --workspace
```

HTML reports with interactive charts open at `target/criterion/report/index.html`.

---

## 1. Full Pipeline Benchmarks (`kuzu-main`)

These measure the complete parse → bind → plan → optimize → execute pipeline via `Connection::query()`.

### 1.1 Query Pipeline

| Benchmark | Query | Median Time |
|-----------|-------|-------------|
| `query/match_return_all` | `MATCH (n:Person) RETURN n.name, n.age, n.score` | **18.5 µs** |
| `query/match_order_by` | `MATCH (n:Person) RETURN n.name ORDER BY n.age` | **13.9 µs** |
| `query/match_limit` | `MATCH (n:Person) RETURN n.name LIMIT 3` | **12.4 µs** |

**Dataset:** 5 Person nodes (name, age, score, active). Loaded via COPY FROM CSV.

### 1.2 Storage Micro-Benchmarks

| Benchmark | Description | Median Time |
|-----------|-------------|-------------|
| `buffer/pin_unpin` | BufferManager: pin + unpin single page | **77.3 ns** |
| `scan/small_100_rows` | Full table scan via Connection, 100 rows | **20.0 µs** |
| `scan/medium_1k_rows` | Full table scan via Connection, 1K rows | **86.5 µs** |

---

## 2. Operator Benchmarks (`kuzu-processor`)

### 2.1 Scan Throughput

PhysicalScan: column-major table data → DataChunk.

| Benchmark | Rows | Columns | Median Time | Row Throughput |
|-----------|------|---------|-------------|----------------|
| `scan/100_rows` | 100 | 4 | **11.9 µs** | 33.6 M rows/s |
| `scan/1k_rows` | 1,000 | 4 | **87.1 µs** | 45.9 M rows/s |
| `scan/10k_rows` | 10,000 | 4 | **1.05 ms** | 38.1 M rows/s |
| `scan/10k_selective_2_of_4_cols` | 10,000 | 2 (projected) | **168 µs** | — |

**Observation:** Scans scale linearly with row count. Column projection gives ~6× speedup (2/4 cols).

### 2.2 Filter Throughput

PhysicalFilter: evaluates expression against each row, produces selection vector.

| Benchmark | Selectivity | Rows | Median Time | Notes |
|-----------|-------------|------|-------------|-------|
| `filter/pass_all_10k` | 100% (constant true) | 10,000 | **433 µs** | Full copy of all rows |
| `filter/remove_all_10k` | 0% (constant false) | 10,000 | **34.7 µs** | Early exit — no output |
| `filter/property_check_10k` | 100% (non-null) | 10,000 | **436 µs** | Variable expression eval |
| `filter/batch_10x1k_chunks` | 100% | 10 × 1,000 | **425 µs** | Chunked input |
| `filter/multi_col_8_fields_10k` | 100% | 10,000 × 8 cols | **3.01 ms** | Multi-column overhead |

**Observation:** Remove-all is ~12× faster than pass-all (no output allocation). Multi-column filter is ~7× slower due to per-column row copying.

### 2.3 Hash Join Throughput

PhysicalHashJoin: hash-build on first chunk, probe on second. Value-keyed with `value_hash()` + `PartialEq`.

| Benchmark | Build Size | Probe Size | Matches | Median Time |
|-----------|-----------|-----------|---------|-------------|
| `join/100_build_100_probe` | 100 | 100 | 100 | **137 µs** |
| `join/1k_build_1k_probe` | 1,000 | 1,000 | 1,000 | **1.44 ms** |
| `join/10k_build_100_probe` | 10,000 | 100 | 100 | **11.8 ms** |
| `join/100_build_10k_probe` | 100 | 10,000 | 100 | **1.94 ms** |
| `join/1k_multi_col_build_1k_probe` | 1,000 (2 cols) | 1,000 | 1,000 | **1.60 ms** |
| `join/1k_no_match` | 1,000 | 1,000 | 0 | **1.07 ms** |

**Observations:**
- Build phase dominates (hash table construction): 10K build + 100 probe = **11.8 ms**
- No-match is faster than match (no output materialization)
- Multi-column build has ~11% overhead
- Per-row cost at 1K scale: ~1.4 µs/match

### 2.4 Sort Throughput

PhysicalOrderBy: collect all values → sort indices → rebuild sorted chunks.

| Benchmark | Size | Keys | Direction | Median Time |
|-----------|------|------|-----------|-------------|
| `order_by/single_key_100` | 100 | 1 | ASC | **6.0 µs** |
| `order_by/single_key_1k` | 1,000 | 1 | ASC | **73.4 µs** |
| `order_by/single_key_10k` | 10,000 | 1 | ASC | **983 µs** |
| `order_by/multi_key_1k` | 1,000 | 2 (Int64 ASC, Double DESC) | Mixed | **209 µs** |
| `order_by/descending_1k` | 1,000 | 1 | DESC | **93.4 µs** |

**Observations:**
- Scaling: O(n log n) — 10K rows is ~170× slower than 100 rows (vs 100× for O(n))
- Multi-key is ~2.8× slower than single-key (composite comparison overhead)
- ASC vs DESC are comparable

### 2.5 Aggregate Throughput

PhysicalAggregate: scalar (no GROUP BY) and hash-based GROUP BY.

#### Scalar Aggregates

| Benchmark | Function | Rows | Median Time |
|-----------|----------|------|-------------|
| `aggregate/count_100` | COUNT | 100 | **1.98 µs** |
| `aggregate/count_10k` | COUNT | 10,000 | **158 µs** |
| `aggregate/sum_10k` | SUM | 10,000 | **623 µs** |
| `aggregate/avg_10k` | AVG | 10,000 | **296 µs** |
| `aggregate/multi_func_10k` | COUNT+SUM+AVG+MIN+MAX | 10,000 | **1.55 ms** |

**Observation:** COUNT is fastest (simple counter). SUM is slower (arithmetic). Multi-function is ~10× COUNT (5 funcs, each processing all rows).

#### GROUP BY Aggregates

| Benchmark | Groups | Size | Median Time | Notes |
|-----------|--------|------|-------------|-------|
| `aggregate/group_by_10_groups_10k` | 10 | 10,000 | **1.06 ms** | Few groups |
| `aggregate/group_by_1k_groups_10k` | 1,000 | 10,000 | **1.07 ms** | Many groups |
| `aggregate/multi_key_group_by_10k` | ~100 | 10,000 | **2.27 ms** | 2-key composite |
| `aggregate/group_by_string_key_10k` | 10 | 10,000 | **2.23 ms** | String keys |

**Observation:** GROUP BY cost dominated by hash table lookup, not group count.
String-key and multi-key are ~2× slower than integer key.

---

## 3. Gap Analysis: Rust vs C++

### 3.1 Methodology

- **Rust numbers:** Real criterion v0.5 measurements (100 samples each, 3s warmup)
- **C++ numbers:** TBD — requires `kuzu_benchmark.exe` with serialized dataset
- **Gap ratio:** `Rust_time / C++_time`. Values < 1.0 mean Rust is faster.
- C++ baseline: `kuzu_benchmark` at `build/release/tools/benchmark/kuzu_benchmark.exe`

### 3.2 Gap Table

| Category | Rust Query | Rust Time | C++ Equivalent | C++ Time | Gap Ratio | Notes |
|----------|-----------|-----------|---------------|----------|-----------|-------|
| **Seq Scan** | `scan/10k_rows` | 1.05 ms | `q23` (fixed_size_seq_scan, SF-100) | TBD | — | Different datasets |
| **Filter** | `filter/pass_all_10k` | 433 µs | `q14` (length < 3, SF-100) | TBD | — | Different selectivity |
| **Hash Join** | `join/1k_build_1k_probe` | 1.44 ms | `q29` (1-hop knows, SF-100) | TBD | — | Different sizes |
| **Order By** | `order_by/single_key_1k` | 73.4 µs | `q25` (sort length, SF-100) | TBD | — | |
| **Agg COUNT** | `aggregate/count_10k` | 158 µs | `q24` (count, SF-100) | TBD | — | |
| **Agg GROUP BY** | `aggregate/group_by_10_groups_10k` | 1.06 ms | `q24` (group by, SF-100) | TBD | — | |
| **Limit** | `query/match_limit` | 12.4 µs | `limit/push-down-limit-into-distinct` | TBD | — | |
| **Full Pipeline** | `query/match_return_all` | 18.5 µs | Various MATCH+RETURN | TBD | — | |

> **Note:** Direct comparison requires running `kuzu_benchmark` against the LDBC SF-100 or tinysnb datasets. See `BENCHMARK_COMPARISON.md` for setup instructions.

### 3.3 C++ Benchmark Catalog (for future comparison)

The C++ `kuzu_benchmark` suite covers **84+ benchmark files** across 5 datasets:

| Dataset | Size | Query Categories | Files |
|---------|------|-----------------|-------|
| **click** | ~100M hits | Aggregation, filter, group-by | 43 |
| **ldbc-sf100** | ~220M comments | Scan, filter, join, order, agg, limit, recursive | ~30 |
| **soc-livejournal** | ~5M nodes, 69M edges | PageRank, WCC, SCC, K-Core, Louvain | 6 |
| **datagen-sf10k** | ~100M nodes | PageRank, WCC, SCC, K-Core, WShortest | 6 |
| **graph500-27** | ~134M nodes | PageRank, WCC, SCC, K-Core | 5 |

---

## 4. Performance Budget

Estimated per-operation budget for real-time (< 100 ms) interactive queries on 10K-row datasets:

| Operation | Budget | Current (10K) | Status |
|-----------|--------|---------------|--------|
| Seq Scan (4 cols) | < 10 ms | **1.05 ms** | ✅ |
| Filter (100% pass) | < 5 ms | **0.43 ms** | ✅ |
| Hash Join (1K×1K) | < 10 ms | **1.44 ms** | ✅ |
| Order By (1 key) | < 10 ms | **0.98 ms** | ✅ |
| Aggregate COUNT | < 5 ms | **0.16 ms** | ✅ |
| Aggregate GROUP BY | < 10 ms | **1.06 ms** | ✅ |
| Full Pipeline | < 50 ms | **0.02 ms** | ✅ |

All operators are **well within budget** at 10K scale.

---

## 5. Improvement Tracking

Use this section to track performance changes over time.

### Baseline: 2026-06-29

| Benchmark | Baseline | Run 2 | Run 3 | Trend |
|-----------|----------|-------|-------|-------|
| `scan/10k_rows` | 1.05 ms | — | — | — |
| `filter/pass_all_10k` | 433 µs | — | — | — |
| `join/1k_build_1k_probe` | 1.44 ms | — | — | — |
| `order_by/single_key_1k` | 73.4 µs | — | — | — |
| `aggregate/count_10k` | 158 µs | — | — | — |
| `aggregate/group_by_10_groups_10k` | 1.06 ms | — | — | — |
| `query/match_return_all` | 18.5 µs | — | — | — |
| `buffer/pin_unpin` | 77.3 ns | — | — | — |

### How to Update

```bash
# Re-run all benchmarks
cargo bench --workspace

# Criterion compares against previous run automatically
# See target/criterion/report/index.html for "Change" columns
```

Criterion automatically computes **regression/speedup percentages** between runs. Key thresholds:
- **p < 0.05** = statistically significant change
- **> 5% change** = warrants investigation

---

## 6. Known Limitations

1. **Small datasets** — Benchmarks use synthetic data (100–10K rows). Real LDBC SF-100 workloads may reveal different scaling characteristics.
2. **No multi-threading** — All benchmarks run single-threaded. Multi-threaded parallelism is not yet implemented in the Rust processor.
3. **In-memory only** — Data lives in `Vec<Vec<Value>>`, not on-disk columnar pages. Phase 1 columnar storage will change I/O patterns.
4. **No C++ comparison data** — C++ `kuzu_benchmark` requires a serialized database. See `BENCHMARK_COMPARISON.md` for build instructions.
5. **No network/IO benchmarks** — COPY FROM CSV/Parquet, extension calls, and external storage not benchmarked.

---

## 7. Optimization Roadmap

Based on baseline data, prioritize:

| Priority | Area | Current | Target | Expected Gain |
|----------|------|---------|--------|---------------|
| 🔴 P0 | Hash Join build phase (10K) | 11.8 ms | < 5 ms | 2× |
| 🔴 P0 | Multi-column filter (8 cols) | 3.01 ms | < 1 ms | 3× |
| 🟡 P1 | Order By sort (10K) | 983 µs | < 500 µs | 2× |
| 🟡 P1 | GROUP BY string key | 2.23 ms | < 1 ms | 2× |
| 🔵 P2 | Full pipeline overhead | 18.5 µs | < 10 µs | 2× |
| 🔵 P2 | Buffer pin/unpin | 77 ns | < 50 ns | 1.5× |

---

## 8. System Information

Recorded by criterion:

| Property | Value |
|----------|-------|
| CPU | _(auto-detected by criterion)_ |
| Cores | _(auto-detected by criterion)_ |
| RAM | _(auto-detected by criterion)_ |
| OS | Windows |
| Rust | edition 2024, stable toolchain |
| Profile | `bench` (equivalent to `release` with LTO) |
| Criterion | v0.5.1, plotters backend (Gnuplot not found) |
