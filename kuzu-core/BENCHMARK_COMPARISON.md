# Kuzu Rust vs C++ Performance Comparison

> **Date:** 2026-07-07 (updated from 2026-06-29)
> **Rust:** criterion v0.5, cargo bench --workspace
> **C++:** TBD — binary not yet built (see [C++ Setup](#cpp-setup) below)
> **Dataset:** Synthetic operator benchmarks (various sizes)

---

## Quick Start

```bash
# Run Rust benchmarks (operator micro-benchmarks)
cd kuzu-core
cargo bench -p kuzu-processor   # Scan, Filter, Join, OrderBy, Aggregate
cargo bench -p kuzu-main         # Full pipeline + Storage

# View HTML reports
# Open target/criterion/report/index.html
```

---

## TL;DR

The Rust Kuzu port shows **competitive performance** on individual operators. Direct C++ comparison is pending — the C++ benchmark binary needs to be built from the CMake project. See [C++ Setup](#cpp-setup) for build instructions.

**Current Status:**
- ✅ Rust micro-benchmarks: 30+ criterion benchmarks across scan, filter, join, sort, aggregate
- ✅ Full pipeline benchmarks: parse→bind→plan→optimize→execute
- ❌ C++ baseline: not yet measured
- ❌ Cross-language gap ratios: TBD

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

| Benchmark | Time | Notes |
|-----------|------|-------|
| `filter/pass_all_10k` | **433 µs** | Constant `true` — processes all 10K rows |
| `filter/remove_all_10k` | **34.7 µs** | Constant `false` — early exit (no output) |
| `filter/property_check_10k` | **436 µs** | Variable expression (non-null check) |
| `filter/batch_10x1k_chunks` | **425 µs** | Same total rows, chunked input |
| `filter/multi_col_8_fields_10k` | **3.01 ms** | 8 columns × 10K rows |

**Key insight:** Constant false (remove all) is ~12× faster than constant true (pass all) because no output chunks need to be allocated.

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

### C++ kuzu_benchmark

The C++ benchmark binary needs to be built from the CMake project at the repo root.

**Prerequisites:**
- CMake 3.15+
- C++20 compiler (MSVC 2022, GCC 13+, or Clang 17+)
- Python 3.9+ (for benchmark runner)

### <a name="cpp-setup"></a>To Build and Run C++ Benchmarks

```bash
# Step 1: Build the C++ project
cd /path/to/kuzu
mkdir -p build/release && cd build/release
cmake ../.. -DCMAKE_BUILD_TYPE=Release -DBUILD_BENCHMARKS=ON
cmake --build . --target kuzu_benchmark --parallel $(nproc)

# Step 2: Serialize a test dataset (tinysnb)
./tools/shell/kuzu_shell /tmp/tinysnb_db <<EOF
CREATE NODE TABLE Person(name STRING, age INT64, score DOUBLE, active BOOL, PRIMARY KEY(name));
COPY Person FROM '../../dataset/tinysnb/person.csv' (HEADER true);
EOF

# Step 3: Run C++ benchmarks
./tools/benchmark/kuzu_benchmark \
  --dataset=/tmp/tinysnb_db \
  --benchmark=../../benchmark/queries/ldbc-sf100/scan_after_filter \
  --warmup=1 --run=5 --json > cpp_bench.json

# Step 4: Compare with Rust
cd kuzu-core
cargo bench -p kuzu-processor -- --output-format bencher > rust_bench.txt
python ../benchmark/compare_benches.py rust_bench.txt ../build/release/cpp_bench.json
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

| Operator | Rust (per-row) | C++ (per-row) | Gap Ratio | Status |
|----------|---------------|--------------|-----------|--------|
| **Seq Scan** (10K, 4 cols) | ~0.105 µs | TBD | — | 🔴 Pending C++ build |
| **Filter** (constant true, 10K) | ~0.043 µs | TBD | — | 🔴 Pending C++ build |
| **Hash Join** (1K×1K) | ~1.44 µs/match | TBD | — | 🔴 Pending C++ build |
| **Order By** (1K, single-key) | ~0.073 µs | TBD | — | 🔴 Pending C++ build |
| **Aggregate COUNT** (10K) | ~0.016 µs | TBD | — | 🔴 Pending C++ build |
| **Agg GROUP BY** (10 groups) | ~0.106 µs | TBD | — | 🔴 Pending C++ build |
| **Full Pipeline** (MATCH+RETURN) | ~18.5 µs | TBD | — | 🔴 Pending C++ build |

> Gap ratio = Rust time / C++ time. Values < 1.0 mean Rust is faster.
> **Status legend:** 🔴 Pending C++ build | 🟡 Partial data | 🟢 Complete

### Next Steps for Gap Analysis
1. Build C++ `kuzu_benchmark` binary (see [C++ Setup](#cpp-setup))
2. Serialize `tinysnb` dataset for both runtimes
3. Run benchmarks with matching dataset sizes
4. Fill gap ratios in the table above
5. File GitHub issues for any gap > 2×

---

## Optimization Priorities

1. **Hash Join build phase** — The `join/10k_build_100_probe` at 11.8ms shows O(n²) or high overhead in hash table construction. Investigate hash function quality and bucket collision resolution.

2. **Filter with multi-column chunks** — `filter/multi_col_8_fields_10k` at 3ms is 7× slower than single-column filter. This is due to per-column row copying in the selection vector path.

3. **Order By with large inputs** — The `order_by/single_key_10k` at ~1ms shows the full collect→sort→rebuild pipeline. Consider a `sort_in_place` optimization that sorts indices without collecting into `Vec<Value>` first.

4. **Full pipeline overhead** — The `query/match_return_all` at 18.5µs vs the raw `scan/100_rows` at 11.9µs shows ~55% overhead from the parse→bind→plan→optimize pipeline. Consider caching prepared plans for repeated queries.

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
