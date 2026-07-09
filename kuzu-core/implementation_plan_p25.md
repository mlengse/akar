# P25: Technical Debt Closure

> **Status:** 🆕 PLANNED | **Target:** 2026-07-18
> **Prerequisites:** P24 ✅
> **Audit:** `cargo test --workspace` → 960+ passed, 0 failed

---

## Overview

Menutup item-item technical debt yang sudah di-defer dari fase sebelumnya:
- P10.3 — STANDALONE_CALL refactor (deferred sejak P10)
- P-MOD2B residual — `processor.rs` execute_internal masih ~900 lines
- P9.3 — C++ benchmark binary
- P9.5 — NPM publish
- P9.1 — Release workflow

---

## 🔴 P25.1 — STANDALONE_CALL Pipeline Refactor

**Deferred sejak:** P10.3 (2026-07-07)
**Rationale originally:** "CALL already works through string matching — defer for architectural purity."

**Now:** Sudah waktunya refactor karena:
- CALL makin banyak fungsinya (14+ table functions, GDS CALL, export_csv/parquet)
- String matching di `ddl.rs` makin panjang → maintenance burden
- Pipeline yang proper memungkinkan error handling lebih baik

### Plan

- `[ ]` **Parser** — `Statement::StandaloneCall` baru (bedakan dari `Statement::Call`)
  - `[ ]` Rule `standalone_call` di `cypher.pest`
  - `[ ]` `CALL func(args)` → `StandaloneCall { name, args }`
  - `[ ]]` `CALL func(args) RETURN *` → `StandaloneCallReturn { name, args, return_all }`
- `[ ]]` **Binder** — `BoundStandaloneCall { function_name, args, return_all }`
  - `[ ]` Resolve function name di FunctionRegistry saat binding
  - `[ ]` Validasi arg count + types
- `[ ]` **Planner** — `LogicalOperator::StandaloneCall(LogicalStandaloneCall)`
- `[ ]` **Processor** — `PhysicalStandaloneCall`
  - `[ ]` Dispatch melalui `FunctionRegistry::execute_table_function()`
  - `[ ]` Support RETURN clause
- `[ ]]` **Hapus** string matching `handle_call()` di `ddl.rs`
- `[ ]` **Tests:** `test_standalone_call_pipeline`:
  - `[ ]` `CALL show_tables()`
  - `[ ]` `CALL table_info('Person')`
  - `[ ]` `CALL show_functions()`
  - `[ ]]` Error: unknown function
- **Effort:** 5 SP | **Risk:** 🟡 Medium

---

## 🟡 P25.2 — processor.rs execute_internal Refactor

**Current state:** `processor/mod.rs` = 2,430 lines. `execute_internal()` saja ~900 lines.
Helper modules sudah dipisah (chunk_helpers, join_helpers, etc.) tapi match block masih satu fungsi besar.

### Plan

**Strategy:** Pisahkan mapping tiap LogicalOperator → implementasi fisik ke file terpisah
(mengikuti pola C++ yang punya `processor/map/map_*.cpp` ~50 file).

- `[ ]` Buat `processor/map/` direktori
- `[ ]` `processor/map/mod.rs` — trait `PhysicalMapper { fn map(&self, op, input) → Result }`
- `[ ]` `processor/map/scan.rs` — `map_scan_node()`, `map_scan_rel()`, `map_fts_scan()`
- `[ ]` `processor/map/join.rs` — `map_hash_join()`, `map_semi_join()`, `map_anti_join()`, `map_cross_product()`, `map_intersect()`
- `[ ]` `processor/map/agg.rs` — `map_aggregate()`, `map_topk()`, `map_orderby()`
- `[ ]` `processor/map/dml.rs` — `map_create()`, `map_set()`, `map_delete()`, `map_merge()`, `map_foreach()`, `map_unwind()`
- `[ ]` `processor/map/ddl.rs` — semua DDL mapping
- `[ ]]` `processor/map/expr.rs` — `map_expressions_scan()`, `map_projection()`, `map_filter()`
- `[ ]` Refactor `execute_internal()` → panggil mapper trait
- **Effort:** 5 SP | **Risk:** 🟢 Low (compiler-guided, no behavior change)

---

## 🟡 P25.3 — CALL Dispatch String Matching → Proper Trait

**Current state:** `connection/ddl.rs` punya `handle_call()` dengan match string pattern:
```rust
"table_info" => ...
"show_tables" => ...
"show_functions" => ...
// ~14 arms total
```

### Plan
- `[ ]` Ubah `TableFunction` di FunctionRegistry jadi trait-based:
  ```rust
  trait TableFunctionExec: Send + Sync {
      fn name(&self) -> &str;
      fn execute(&self, args: &[Value]) -> Result<Vec<DataChunk>, String>;
  }
  ```
- `[ ]]` Registrasikan semua table function sebagai struct implementor trait
- `[ ]` Hapus match arms string di `handle_call()` → ganti dengan registry lookup
- **Effort:** 3 SP | **Risk:** 🟢 Low

---

## 🟢 P25.4 — C++ Benchmark Binary

**Deferred sejak:** P9.3
**Purpose:** Benchmark comparison Rust vs C++ untuk 7 kategori (scan, filter, hash join, order by, aggregate, GDS, storage)

### Plan
- `[ ]` Build C++ binary: `cmake --build build/release --target kuzu_benchmark`
- `[ ]` Run C++ benchmarks → `cpp_bench.json`
- `[ ]]` Run Rust benchmarks → `rust_bench.txt`
- `[ ]` Update `BENCHMARK_COMPARISON.md` dengan gap ratios
- **Effort:** 3 SP | **Risk:** 🟡 Medium (C++ build environment)

---

## 🟢 P25.5 — Release & Publish

**Deferred sejak:** P9.1 (release workflow), P9.5 (NPM publish)

### Plan
- `[ ]` **NPM publish** — `kuzu-wasm/pkg/` → npm registry
  - `[ ]]` Setup npm account + access token di CI secrets
  - `[ ]` `npm publish` step di release workflow
- `[ ]` **crates.io publish** — publish `kuzu` (tools/rust_api) + `kuzu-main`
  - `[ ]]` Setup crates.io token
  - `[ ]` `cargo publish` workflow
- `[ ]` **GitHub Release** — automated release with changelog
- **Effort:** 2 SP | **Risk:** 🟢 Low

---

## P25.6 — Summary

| Item | SP | Risk |
|------|----|------|
| P25.1 STANDALONE_CALL refactor | 5 | 🟡 Medium |
| P25.2 processor.rs refactor | 5 | 🟢 Low |
| P25.3 CALL dispatch trait | 3 | 🟢 Low |
| P25.4 C++ benchmark binary | 3 | 🟡 Medium |
| P25.5 Release & publish | 2 | 🟢 Low |
| **Total P25** | **18** | |

---

## Verification

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo build --workspace --release
```
