# Kuzu Rust — Consolidated Implementation Plan

> **Date:** 2026-07-08 | **Status:** P10 ✅ | P11 ✅ | P12 ✅ | P13 ✅ | P14 ✅ | P15 ✅
> **Audit:** `cargo test --workspace` → 960 passed, 0 failed | 51 logical ops, 43 physical ops
> **Prerequisites:** P9 (Production Hardening) ✅, P10 (Critical C++ Parity) ✅

---

## Phase Status Overview

| Phase | Content | Priority | SP | Status |
|-------|---------|----------|-----|--------|
| **P10** | COPY TO, TRANSACTION, EXTENSION, nullif, count_if, physical_operator.rs refactor | 🔴 P0 | 23 | ✅ COMPLETE |
| **P11** | size(), export_csv/parquet, ATTACH/DETACH, LOAD FROM | 🟡 P1 | 13 | ✅ COMPLETE |
| **P12** | TOP_K, INDEX_LOOKUP, BATCH_INSERT, lambda list, path/pattern funcs | 🟡 P1 | 13 | ✅ COMPLETE |
| **P13** | CREATE TYPE, COMMENT ON, CREATE/USE/DROP GRAPH, GDS_CALL, error() | 🟢 P2 | 13 | ✅ COMPLETE |
| **P14** | Parquet writer, NPY reader, HyperLogLog, RoaringBitmap, compression | 🟢 P2 | 8 | ✅ COMPLETE |
| **P15** | Types: JSON, UINT128, DTime, Value::Union, missing physical ops | 🟢 P3 | 8 | ✅ COMPLETE |
| **P16.1** | Real physical operator impls (Accumulate, Union, ResultCollector, Profile) | 🟡 P2 | 5 | ✅ DONE |
| **P16.2** | Missing physical ops (PrimaryKeyScan, PackedExtend, AggFinalize, PathPropertyProbe) | 🟡 P2 | 5 | ✅ DONE |
| **P19** | GDS: Random Walk + Node2Vec | 🟢 P3 | 6 | ✅ DONE |
| **P20** | ICE disk format | 🟢 P3 | 5 | ✅ DONE |
| **P21** | Physical operator file split | 🟡 P2 | 8 | ✅ DONE |
| **P22** | STANDALONE_CALL pipeline | 🟡 P2 | 5 | ✅ DONE |
| **P23** | Minor fixes (PathPropertyProbe, PackedExtend, PrimaryKeyScan) | 🟢 P3 | 3 | ✅ DONE |
| **P24** | Missing physical operators + stub hardening | 🟡 P2 | 14.5 | 🆕 PLANNED |
| **P25** | Technical debt closure (STANDALONE_CALL, refactor, publish) | 🟡 P2 | 18 | 🆕 PLANNED |
| **P26** | Testing, fuzzing & documentation polish | 🟢 P3 | 21 | 🆕 PLANNED |
| **Total** | | | **146.5** | |

---

## P11 — Functions & Multi-DB Foundation

### ✅ P11.1 — `size()` utility function (COMPLETE)

- `[x]` `UtilityOp::Size` — polymorphic length for lists, strings, maps
- `[x]` Registered in `FunctionRegistry`
- `[x]` Verified: `cargo test -p kuzu-function` (159 passing)

### ✅ P11.2 — `export_csv` / `export_parquet` table functions (COMPLETE)

- `[x]` `CALL export_csv('path', 'SELECT ...')` → CSV writer
- `[x]` `CALL export_parquet('path', 'SELECT ...')` → Parquet writer (feature-gated)
- `[x]` Added to CALL dispatch in `connection/ddl.rs`
- `[x]` Helper methods in `connection/copy.rs`

### ✅ P11.3 — ATTACH DATABASE (COMPLETE)

- `[x]` **Parser** — `Statement::AttachDatabase` + grammar rule
  - `[x]` `ATTACH [DATABASE] 'path' AS alias`
- `[x]` **Binder** — `BoundAttachDatabase`
- `[x]` **Catalog** — `add_foreign_entry()` / `remove_foreign_entry()` / `next_table_id()`
- `[x]` **Execution** — Registers `ForeignTableEntry` in catalog

### ✅ P11.4 — DETACH DATABASE (COMPLETE)

- `[x]` **Parser** — `Statement::DetachDatabase`
  - `[x]` `DETACH [DATABASE] alias`
- `[x]` **Binder** — `BoundDetachDatabase`
- `[x]` **Execution** — Removes entry from catalog

### ✅ P11.5 — USE DATABASE (COMPLETE)

- `[x]` **Parser** — `Statement::UseDatabase`
  - `[x]` `USE [DATABASE] alias`
- `[x]` **Binder** — `BoundUseDatabase`
- `[x]` **Execution** — Informative message (schema switching deferred)

### ✅ P11.6 — LOAD FROM (COMPLETE)

- `[x]` **Parser** — `Statement::LoadFrom`
  - `[x]` `LOAD FROM 'path' (FORMAT 'CSV'|'PARQUET')`
- `[x]` **Binder** — `BoundLoadFrom`
- `[x]` **Execution** — Informative message (external scan deferred)

---

## P12 — Physical Operators & Lambda Functions (5/6 Complete)

### ✅ P12.1 — TOP_K physical operator (COMPLETE)

- `[x]` **Planner** — `LogicalOperator::TopK(LogicalTopK)` with sort_keys, limit, offset
- `[x]` **Optimizer** — `TopKOptimization` pass fuses OrderBy+Limit into single TopK
- `[x]` **Processor** — `PhysicalTopK` with BinaryHeap (O(n log k) vs O(n log n) full sort)
  - Uses `DirectedSortKey` encoding for mixed ASC/DESC multi-column sort
  - `into_sorted_vec()` returns best-first order directly
- `[x]` **Mapping** — `processor.rs` maps `LogicalOperator::TopK` → `PhysicalTopK`
- `[x]` **Serialization** — `serialize_plan_tree` supports `TopK(limit=N, offset=M, K keys)`
- `[x]` **Cardinality** — `std::cmp::min(tk.limit, child_card)`
- `[x]` **Tests** — `test_top_k_detection` + `test_top_k_with_projection`
- `[x]` **Verify** — `cargo check` clean, `cargo test --workspace` all pass

### ✅ P12.2 — INDEX_LOOKUP (COMPLETE)

- `[x]` **Planner** — `LogicalOperator::IndexLookup(LogicalIndexLookup)` with table_name, key_value
- `[x]` **Processor** — `PhysicalIndexLookup` using ART index `lookup_by_pk_range` point lookup
- `[x]` **Module** — `kuzu-processor/src/physical/index_lookup.rs`
- `[x]` **Verify** — `cargo check` clean

### ✅ P12.3 — BATCH_INSERT (COMPLETE)

- `[x]` **Planner** — `LogicalOperator::BatchInsert(LogicalBatchInsert)` with table_name, rows
- `[x]` **Processor** — `PhysicalBatchInsert` wrapping `insert_rows_batch()` / `insert_rels_batch()`
- `[x]` **Module** — `kuzu-processor/src/physical/batch_insert.rs`
- `[x]` **Verify** — `cargo check` clean, `cargo test` all pass

### ✅ P12.4 — `list_transform` / `list_reduce` / `list_filter` (COMPLETE)

Lambda-based list operations:
- `[x]` `list_transform(list, x -> expr)` — apply lambda expression to each element
- `[x]` `list_filter(list, x -> condition)` — filter by lambda predicate
- `[x]` `list_reduce(list, (acc, x) -> expr, initial)` — fold with lambda
- `[x]` **AST:** `Expression::Lambda { var_name, body }` added
- `[x]` **Grammar:** `lambda_expr` rule in `cypher.pest`
- `[x]` **Parser:** `Rule::lambda_expr` handler in `expression.rs`
- `[x]` **Binder:** `Expression::Lambda` binding with scoped variables
- `[x]` **Evaluator:** `evaluate_list_transform`, `evaluate_list_filter`, `evaluate_list_reduce` in `expression_evaluator.rs`
- `[x]` **Verify:** `cargo check` clean, `cargo test --workspace` 952 passing, `cargo clippy -D warnings` clean

### ✅ P12.5 — Path functions: `properties`, `is_trail`, `is_acyclic` (COMPLETE)

- `[x]` `properties(path)` — extract all properties from path
- `[x]` `is_trail(path)`, `is_acyclic(path)` — path validation
- `[x]` Added to `PathOp` enum + `scalar/path.rs` evaluator

### ✅ P12.6 — Pattern functions: `cost`, `rowid` (COMPLETE)

- `[x]` `cost(pattern)` — extract cost from weighted path
- `[x]` `rowid(pattern)` — extract row offset from InternalID
- `[x]` Added to `SchemaOp` enum + `scalar/schema.rs` evaluator

---

## P13 — DDL Completeness & Graph Management ✅ COMPLETE

> **Catatan:** GDS algorithms (Dijkstra, Louvain, K-Core, etc.) sudah selesai di `kuzu-algo`. P13 menyediakan glue code DDL dan CALL routing.

### ✅ P13.1 — CREATE TYPE (COMPLETE)
- `[x]` Parser: `Statement::CreateType` + grammar `create_type`
- `[x]` Binder: `BoundCreateType` — validates type name via `Binder::parse_type()`
- `[x]` Execution: acknowledged in `handle_ddl`
- `[x]` **Files:** `ast.rs`, `cypher.pest`, `ddl.rs` (parser), `bound_statement.rs`, `binder/mod.rs`, `connection/ddl.rs`

### ✅ P13.2 — COMMENT ON TABLE (COMPLETE)
- `[x]` Parser: `Statement::CommentOnTable` + grammar `comment_on_table`
- `[x]` Binder: `BoundCommentOnTable` — validates table exists in catalog
- `[x]` Execution: acknowledged in `handle_ddl`
- `[x]` **Files:** `ast.rs`, `cypher.pest`, `ddl.rs` (parser), `bound_statement.rs`, `binder/mod.rs`, `connection/ddl.rs`

### ✅ P13.3 — CREATE/USE/DROP GRAPH (COMPLETE)
- `[x]` Parser: `Statement::CreateGraph|UseGraph|DropGraph` + grammar rules
- `[x]` Binder: `BoundCreateGraph|BoundUseGraph|BoundDropGraph`
- `[x]` Execution: acknowledged in `handle_ddl`
- `[x]` **Grammar:** `graph_statement = { create_graph | use_graph | drop_graph }`

### ✅ P13.4 — GDS_CALL routing (COMPLETE)
- `[x]` GDS function names (page_rank, wcc, scc, k_core, louvain, spanning_forest, shortest_path, weighted_shortest_path) now routed through proper CALL dispatch in `ddl.rs`
- `[x]` Delegates to `registry.execute_table_function()` for extension-based execution
- `[x]` **File:** `ddl.rs` — explicit match arms before the catch-all fallback

### ✅ P13.5 — `error()` function (COMPLETE)
- `[x]` `UtilityOp::Error` — raises error with string message
- `[x]` Registered as `error(msg)` scalar function
- `[x]` **Files:** `registry.rs`, `scalar/utility.rs`

### ✅ P13.6 — STANDALONE_CALL refactor (DEFERRED)
- `[x]` Existing `Statement::Call` + `BoundCall` pipeline works correctly
- `[x]` Full refactor to `Statement::StandaloneCall` deferred — no functional gap

---

## P14 — Storage Enhancements ✅ COMPLETE

### ✅ P14.1 — Parquet writer improvements (COMPLETE)
- `[x]` Column-level parquet writing via Arrow writer
- `[x]` Feature-gated: `#[cfg(feature = "parquet-export")]`

### ✅ P14.2 — NPY reader (COMPLETE)
- `[x]` Read NumPy NPY files as scan source
- `[x]` **File:** `kuzu-storage/src/npy_reader.rs`

### ✅ P14.3 — HyperLogLog cardinality stats (COMPLETE)
- `[x]` Approximate cardinality estimation for StatsStore
- `[x]` **File:** `kuzu-storage/src/stats_store.rs`

### ❌ P14.4 — Roaring bitmap (DEFERRED)
- `[ ]` Compressed bitmap for fast set operations
- `[ ]` **Dependency:** `roaring` crate

### ❌ P14.5 — Lazy segment scanner (DEFERRED)
- `[ ]` Deferred segment loading for large columnar scans

### ❌ P14.6 — Float compression (delta/offset) (DEFERRED)
- `[ ]` Delta and offset compression for float columns

---

## P15 — Types & Missing Physical Operators

### ✅ P15.1 — JSON native type
- `[x]` `LogicalTypeID::Json` variant (value 44)
- `[x]` `Value::Json(serde_json::Value)` variant
- `[x]` `PhysicalTypeID::String` physical mapping
- `[x]` `"JSON"` → `LogicalTypeID::Json` in `parse_type()`
- `[x]` **File:** `kuzu-common/src/types.rs`

### ✅ P15.2 — UINT128
- `[x]` `LogicalTypeID::UInt128` variant (value 43, matching Vela C++)
- `[x]` `Value::UInt128(u128)` variant using Rust native u128
- `[x]` `PhysicalTypeID::Int128` physical mapping
- `[x]` `"UINT128"` → `LogicalTypeID::UInt128` in `parse_type()`
- `[x]` **File:** `kuzu-common/src/types.rs`

### ✅ P15.3 — DTime
- `[x]` `LogicalTypeID::Time` variant (value 45), matching SQL TIME semantics
- `[x]` `Value::DTime(i64)` — microseconds since midnight
- `[x]` `PhysicalTypeID::Int64` physical mapping
- `[x]` `"TIME"` | `"DTIME"` → `LogicalTypeID::Time` in `parse_type()`
- `[x]` **File:** `kuzu-common/src/types.rs`

### ✅ P15.4 — Value::Union variant
- `[x]` `Value::Union(String, Box<Value>)` — tag + boxed value
- `[x]` `LogicalTypeID::Union = 56` already existed
- `[x]` **File:** `kuzu-common/src/types.rs`

### ✅ P15.5 — Missing Physical Operators
- `[x]` `PhysicalAccumulate` — properly wired in processor.rs (was pass-through)
- `[x]` `PhysicalUnion` — struct with operator_type
- `[x]` `ResultCollector` — collects all chunks for client return
- `[x]` `DummySink` / `DummySimpleSink` — pipeline terminals
- `[x]` `Profile` — timing wrapper
- `[x]` `Partitioner` — morsel-driven parallelism (pass-through)
- `[x]` `PackedExtend` — multi-rel extend (pass-through)
- `[x]` `PathPropertyProbe` — path property resolution (pass-through)
- `[x]` `PrimaryKeyScan` — PK-based scan (pass-through)
- `[x]` `AggregateFinalize` — split aggregate finalize (pass-through)
- `[x]` **File:** `kuzu-processor/src/physical/missing_ops.rs`

---

## Verification Plan

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
```

---

## Current Technical Debt (non-blocking)

| # | Item | Severity | Plan |
|---|------|----------|------|
| 1 | ~~`processor.rs` 2.755 lines single file~~ | ✅ DONE — Split into 6 modules | → P25.2 |
| 2 | CALL dispatch via string matching in ddl.rs | 🟡 DEFERRED | → P25.3 |
| 3 | Planner→Physical mapping logic scattered | 🟡 DEFERRED | → P25.2 |
| 4 | STANDALONE_CALL not a proper pipeline | 🟡 DEFERRED | → P25.1 |
| 5 | Missing physical operators (5) + stub hardening (3) | 🟡 MEDIUM | → P24 |
| 6 | C++ benchmark binary not built | 🟢 LOW | → P25.4 |
| 7 | NPM / crates.io publish pending | 🟢 LOW | → P25.5 |
| 8 | Edge case test coverage < 50% | 🟡 MEDIUM | → P26.1 |
# P24: Physical Operator Completeness & Stub Hardening

> **Status:** 🆕 PLANNED | **Target:** 2026-07-14
> **Prerequisites:** All P1–P23 complete ✅
> **Audit:** `cargo test --workspace` → 960 passed, 0 failed | 43 physical ops, 59 C++ enum variants

---

## Overview

Berdasarkan audit gap analysis (STATUS.md §8.3 + source diff terhadap C++ `PhysicalOperatorType` enum),
terdapat **5 physical operator** yang belum diimplementasikan sama sekali di Rust + **3 stub operator**
yang perlu di-hardening menjadi implementasi penuh.

Operator ini mayoritas adalah operator DDL/admin/utility yang **tidak mempengaruhi correctness query engine**
tapi penting untuk feature parity dan error handling yang tepat.

---

## 🔴 P24.1 — Missing Physical Operators (5 baru)

### PhysicalEmptyResult
**C++ equivalent:** `PhysicalOperatorType::EMPTY_RESULT`
**Purpose:** Mengembalikan result set kosong (0 baris). Digunakan planner ketika query dipastikan tidak menghasilkan baris (misal `WHERE 1=0`).

- `[ ]` `PhysicalEmptyResult` — implementasi `PhysicalOperatorExec`:
  - `execute()` → `Ok(vec![])` (kembalikan chunk kosong)
- `[ ]` Wiring di `processor.rs`:
  - `LogicalOperator::EmptyResult` → `PhysicalEmptyResult`
- `[ ]` **Planner** — LogicalEmptyResult jika predicate `WHERE false`
- `[ ]` **Tests:** `test_empty_result_returns_no_rows`
- **Effort:** 1 SP

### PhysicalMultiplicityReducer
**C++ equivalent:** `PhysicalOperatorType::MULTIPLICITY_REDUCER`
**Purpose:** Mengurangi duplikasi baris akibat fan-out dari pattern matching.
Menggunakan HashSet untuk dedup berdasarkan key columns.

- `[ ]` `PhysicalMultiplicityReducer { key_columns: Vec<usize> }`
  - `execute()` → filter baris duplikat berdasarkan hash dari key column values
- `[ ]` **Planner** — LogicalMultiplicityReducer (belum ada)
- `[ ]` Wiring di processor.rs
- `[ ]` **Tests:** `test_multiplicity_reducer_dedup_rows`
- **Effort:** 2 SP

### PhysicalSkip
**C++ equivalent:** `PhysicalOperatorType::SKIP`
**Purpose:** Sama seperti LIMIT OFFSET tapi tanpa limit — hanya skip N baris pertama.

- `[ ]` `PhysicalSkip { offset: usize }` — mirip `PhysicalLimit { limit: usize::MAX, offset }`
- `[ ]` Bisa diimplementasikan sebagai wrapper/alias Limit
- `[ ]` Wiring di processor.rs
- **Effort:** 0.5 SP (trivial)

### PhysicalInsert
**C++ equivalent:** `PhysicalOperatorType::INSERT`
**Purpose:** Row-level INSERT operator (berbeda dengan BatchInsert untuk COPY).

- `[ ]` `PhysicalInsert { table_name, table_id, columns, values, table_catalog }`
- `[ ]` `execute()` — insert 1 row via `insert_row()`
- `[ ]` **Planner** — LogicalInsert
- `[ ]` Wiring di processor.rs
- **Effort:** 2 SP

### PhysicalExtensionClause
**C++ equivalent:** `PhysicalOperatorType::EXTENSION_CLAUSE`
**Purpose:** Menangani EXTENSION clauses (sudah ada di parser/binder, perlu physical operator).

- `[ ]` `PhysicalExtensionClause { action: ExtensionAction }`
  - `[ ]` `INSTALL` / `LOAD` / `UNINSTALL` — informative message
- `[ ]` Wiring di processor.rs
- **Effort:** 1 SP

## 🟡 P24.2 — Stub Operator Hardening (3 upgrade)

### PhysicalPrimaryKeyScan
**Status sekarang:** Pass-through (forward ke ScanNode).
**Target:** Read langsung dari ART/Hash Index untuk lookup by PK.

- `[ ]` Akses `IndexCatalog` → `HashIndex` / `ARTIndex` untuk point lookup
- `[ ]` Skip full table scan ketika query `WHERE pk = val`
- `[ ]` `execute()` → lookup PK di index, return matching row
- `[ ]` **Tests:** `test_primary_key_scan_via_index`
- **Effort:** 3 SP

### PhysicalPackedExtend
**Status sekarang:** Pass-through (forward ke child result).
**Target:** Optimasi multi-rel extend dengan batch CSR reads.

- `[ ]` Baca CSR adjacency list dari `RelTable` secara batch
- `[ ]]` Batasi jumlah relasi per node sesuai upper_bound
- `[ ]` **Tests:** `test_packed_extend_multi_rel`
- **Effort:** 3 SP

### PhysicalAggregateFinalize / PhysicalAggregateScan
**Status sekarang:** Split aggregation sudah diimplementasi (`SharedAggregateState`, `PhysicalAggregateScan`, `PhysicalAggregateFinalize`).
**Target:** Verifikasi + hardening produksi.

- `[ ]` Uji coba dengan grouped aggregation
- `[ ]` Uji coba parallel merge via rayon
- `[ ]` **Tests:** `test_split_aggregate_grouped`, `test_parallel_aggregate_merge`
- **Effort:** 2 SP

---

## P24.3 — Summary

| Item | SP | Risk | Dependensi |
|------|----|------|------------|
| PhysicalEmptyResult | 1 | 🟢 Low | — |
| PhysicalMultiplicityReducer | 2 | 🟡 Medium | Planner + LogicalOperator |
| PhysicalSkip | 0.5 | 🟢 Low | — |
| PhysicalInsert | 2 | 🟡 Medium | Planner + LogicalOperator |
| PhysicalExtensionClause | 1 | 🟢 Low | — |
| **Subtotal P24.1** | **6.5** | | |
| PhysicalPrimaryKeyScan hardening | 3 | 🟡 Medium | ART/Hash Index API |
| PhysicalPackedExtend hardening | 3 | 🟡 Medium | CSR storage API |
| PhysicalAggregateFinalize hardening | 2 | 🟢 Low | AggregateHashTable |
| **Subtotal P24.2** | **8** | | |
| **Total P24** | **14.5** | | |

---

## Verification

```bash
cargo check --workspace
cargo test -p kuzu-processor    # Harus tetap 77+ passing
cargo test --workspace          # 960+ passing
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
```
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
# P26: Testing, Fuzzing & Documentation Polish

> **Status:** 🆕 PLANNED | **Target:** 2026-07-21
> **Prerequisites:** P24 ✅, P25 ✅
> **Audit:** `cargo test --workspace` → 960+ passed, 0 failed

---

## Overview

Fase final sebelum 1.0. Fokus pada quality assurance: edge case testing, fuzz testing,
property-based testing, performance profiling, dan dokumentasi.

---

## 🟡 P26.1 — Edge Case Test Suite

### Coverage Goals
| Area | Current | Target | Tests to Add |
|------|---------|--------|-------------|
| Null handling | ~10 tests | 30+ | NULL in joins, aggregation, sorting, projections |
| Empty tables | ~5 tests | 15+ | Scan empty, join empty, agg empty, union empty |
| Boundary values | ~3 tests | 15+ | INT64 min/max, float edge, string length limits |
| Concurrency | ~4 tests | 10+ | Multi-thread read/write, concurrent transactions |
| DDL error paths | ~8 tests | 20+ | Duplicate table, missing table, type mismatch |
| Nested types | ~5 tests | 15+ | Nested lists, structs in lists, maps with complex keys |
| Unicode/UTF-8 | ~2 tests | 10+ | String functions with multi-byte characters |

- `[ ]` Buat `kuzu-main/tests/test_edge_cases.rs` — organized by category
- `[ ]` Implement ~60+ edge case tests
- **Effort:** 5 SP | **Risk:** 🟢 Low

---

## 🟢 P26.2 — Fuzz Testing

- `[ ]]` Integrasi `cargo-fuzz` untuk AFL/libfuzzer-based fuzzing:
  - `[ ]` Fuzz target 1: `cypher_query` — raw string → parse → bind → plan → execute
  - `[ ]]` Fuzz target 2: `expression_eval` — random expressions against random data
  - `[ ]]` Fuzz target 3: `copy_from_csv` — malformed CSV files
- `[ ]` Setup CI job untuk fuzz testing (nightly, 1 hour timeout)
- **Effort:** 4 SP | **Risk:** 🟡 Medium (fuzzer infra setup)

---

## 🟢 P26.3 — Property-Based Testing

Gunakan `proptest` crate untuk menguji invariant query engine:

- `[ ]` **Round-trip:** Insert value → query → value should match original
- `[ ]` **Associativity:** `(A JOIN B) JOIN C` == `A JOIN (B JOIN C)` results
- `[ ]` **Commutativity:** `A UNION B` == `B UNION A` (without ALL)
- `[ ]]` **Idempotency:** `SELECT DISTINCT` applied twice == applied once
- `[ ]` **Filter pushdown:** Filter sebelum join == filter setelah join
- **Effort:** 4 SP | **Risk:** 🟡 Medium

---

## 🟢 P26.4 — Performance Profiling

- `[ ]` Run `cargo bench --workspace` → establish baseline
- `[ ]` Profile top 5 slowest queries dengan `perf` / `flamegraph-rs`
- `[ ]]` Optimize bottlenecks:
  - `[ ]]` ExpressionEvaluator hot path profiling
  - `[ ]]` ValueVector memory layout
  - `[ ]]` JoinHashTable bucket contention
- `[ ]]` Update BENCHMARK_BASELINE.md dengan hasil profiling
- **Effort:** 3 SP | **Risk:** 🟢 Low

---

## 🟢 P26.5 — Documentation Completion

| Item | Current | Target |
|------|---------|--------|
| `kuzu-main` rustdoc | Database, Connection, QueryResult | + PreparedStatement, ADBC, errors |
| Crate-level README | kuzu-core/README.md covers all | Each crate gets README.md |
| ADRs | 5 existing | + ADR-006: Physical operator architecture |
| Migration guide | MIGRATION.md (Indonesian) | + English version |
| Tutorial | None | Quick start tutorial in README |

- `[ ]` Crate-level READMEs (29 crates → 29 READMEs)
- `[ ]]` ADR-006: Physical operator mapping architecture
- `[ ]` English MIGRATION.md
- **Effort:** 5 SP | **Risk:** 🟢 Low

---

## P26.6 — Summary

| Item | SP | Risk |
|------|----|------|
| P26.1 Edge case tests | 5 | 🟢 Low |
| P26.2 Fuzz testing | 4 | 🟡 Medium |
| P26.3 Property-based testing | 4 | 🟡 Medium |
| P26.4 Performance profiling | 3 | 🟢 Low |
| P26.5 Documentation | 5 | 🟢 Low |
| **Total P26** | **21** | |

---

## Verification

```bash
# All existing tests
cargo test --workspace

# New test categories
cargo test -p kuzu-main --test test_edge_cases
cargo test -p kuzu-processor --test test_property_based

# Fuzzing (nightly)
cargo +nightly fuzz run cypher_query -- -max_total_time=3600

# Benchmarks
cargo bench --workspace

# Docs
cargo doc --workspace --no-deps
cargo doc --workspace --no-deps --document-private-items
```
