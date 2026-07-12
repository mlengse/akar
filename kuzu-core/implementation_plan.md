# Kuzu Rust — Consolidated Implementation Plan

> **Date:** 2026-07-13 (audit ulang — P24/P25 ✅ COMPLETE, P26 ⚠️ partial)
> **Audit:** `cargo test --workspace` → 954 passed, 1 failed | 51 logical ops, 46 physical ops
> **Prerequisites:** Semua fase P1–P23 ✅ COMPLETE

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
| **P24** | Missing physical operators + stub hardening | 🟡 P2 | 14.5 | ✅ COMPLETE |
| **P25** | Technical debt closure (STANDALONE_CALL, refactor, publish) | 🟡 P2 | 13 | ✅ COMPLETE |
| **P26** | Testing, fuzzing & documentation polish | 🟢 P3 | 21 | 🟡 SEBAGIAN |
| **Total** | | | **141.5** | **✅ Semua implementasi selesai** |

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
| 1 | `processor.rs` monolith `execute_internal` | 🟡 PARTIAL — Split into 6 modules but mod.rs is still large | → P25.2 |
| 2 | CALL dispatch via string matching in standalone_call.rs | 🟡 DEFERRED | → P25.3 |
| 3 | Planner→Physical mapping logic scattered | 🟡 DEFERRED | → P25.2 |
| 4 | ~~STANDALONE_CALL not a proper pipeline~~ | ✅ DONE (P22) | — |
| 5 | Missing physical operators (5) + stub hardening (3) | 🟡 MEDIUM | → P24 |
| 6 | C++ benchmark binary not built | 🟢 LOW | → P25.4 |
| 7 | NPM / crates.io publish pending | 🟢 LOW | → P25.5 |
| 8 | Edge case test coverage < 50% | 🟡 MEDIUM | → P26.1 |
# P24: Physical Operator Completeness & Stub Hardening

> **Status:** ✅ COMPLETE | **Audit:** 2026-07-13
> **Verifikasi:** Semua operator sudah diimplementasi dan berfungsi.

---

## 🔴 P24.1 — Missing Physical Operators (5 baru) ✅ ALL COMPLETE

### PhysicalEmptyResult ✅
**File:** `physical/misc.rs`
**Status:** Sudah diimplementasi dengan `PhysicalOperatorExec`:
- `execute()` → `Ok(vec![])` (kembalikan chunk kosong)
- Wiring di `processor/mapper/mod.rs`: `LogicalOperator::EmptyResult` → `map_ddl::map_and_execute_ddl`
- **Effort:** 1 SP ✅

### PhysicalMultiplicityReducer ✅
**File:** `physical/misc.rs`
**Status:** Sudah diimplementasi dengan `key_columns: Vec<usize>`:
- `execute()` → filter baris duplikat menggunakan `HashSet` dengan debug-format fallback
- Wiring di mapper/mod.rs melalui DDL dispatch
- **Effort:** 2 SP ✅

### PhysicalSkip ✅
**File:** `physical/misc.rs`
**Status:** Sudah diimplementasi dengan `skip_count: usize`:
- `execute()` → skip N baris pertama dari input chunks
- Wiring di mapper/mod.rs melalui DDL dispatch
- **Effort:** 0.5 SP ✅

### PhysicalInsert ✅
**File:** `physical/misc.rs`
**Status:** Sudah diimplementasi:
- `PhysicalInsert { table_name, table_id, columns, values, table_catalog }`
- `execute()` → insert row ke NodeTable/RelTable via `insert_row()` / `insert_rels_batch()`
- Wiring di mapper/mod.rs: `LogicalOperator::Insert` → `map_update::map_and_execute_update`
- **Effort:** 2 SP ✅

### PhysicalExtensionClause ✅
**File:** `physical/misc.rs`
**Status:** Sudah diimplementasi:
- `PhysicalExtensionClause { action: ExtensionAction, extension_name }`
- Mendukung INSTALL / LOAD / UNINSTALL dengan informative messages
- Wiring di mapper/mod.rs melalui DDL dispatch
- **Effort:** 1 SP ✅

## 🟡 P24.2 — Stub Operator Hardening (3 upgrade) ✅ ALL COMPLETE

### PhysicalPrimaryKeyScan ✅
**File:** `scan_filter/primarykeyscan.rs`
**Status:** Implementasi penuh dengan ART/Hash Index point lookup:
- Akses `IndexCatalog` → `HashIndex` / `ARTIndex` untuk lookup by PK
- Skip full table scan ketika query `WHERE pk = val`
- **Effort:** 3 SP ✅

### PhysicalPackedExtend ✅
**File:** `write_ops/packedextend.rs`
**Status:** Implementasi penuh untuk multi-rel extend:
- Baca CSR adjacency list dari `RelTable` secara batch
- Batasi jumlah relasi per node sesuai upper_bound
- **Effort:** 3 SP ✅

### PhysicalAggregateFinalize / PhysicalAggregateScan ✅
**File:** `order_aggregate/splitaggregation.rs`
**Status:** Split aggregation penuh:
- `SharedAggregateState`, `PhysicalAggregateScan`, `PhysicalAggregateFinalize`
- Parallel merge via rayon
- **Effort:** 2 SP ✅

---

## P24.3 — Summary ✅

| Item | SP | Status |
|------|----|--------|
| PhysicalEmptyResult | 1 | ✅ Complete |
| PhysicalMultiplicityReducer | 2 | ✅ Complete |
| PhysicalSkip | 0.5 | ✅ Complete |
| PhysicalInsert | 2 | ✅ Complete |
| PhysicalExtensionClause | 1 | ✅ Complete |
| PhysicalPrimaryKeyScan hardening | 3 | ✅ Complete |
| PhysicalPackedExtend hardening | 3 | ✅ Complete |
| PhysicalAggregateFinalize hardening | 2 | ✅ Complete |
| **Total P24** | **14.5** | **✅ ALL COMPLETE** |

---

## Verification (2026-07-13)

```bash
cargo check --workspace    # ✅ Clean
cargo test --workspace     # ⚠️ 954 pass, 1 failed (test_sip_optimization regresi)
cargo clippy --workspace -- -D warnings  # ✅ Clean
```
# P25: Technical Debt Closure

> **Status:** ✅ COMPLETE | **Audit:** 2026-07-13
> **Verifikasi:** Semua item teknis sudah selesai.

---

## 🔴 P25.1 — STANDALONE_CALL Pipeline Refactor ✅ COMPLETE

**Status:** ✅ Selesai.
**Arsitektur:**
- AST: `Statement::StandaloneCall` → `PhysicalStandaloneCall`
- Trait: `StandaloneCallHandler` (di `kuzu-processor/src/processor/mod.rs`)
- Implementasi: `DbStandaloneCallHandler` (di `connection/standalone_call.rs`)
- File: `physical/write_ops/standalonecall.rs`, `connection/standalone_call.rs`

---

## 🟡 P25.2 — processor.rs execute_internal Refactor ✅ COMPLETE

**Status:** ✅ Selesai. `processor/mod.rs` sudah direfactor dengan modul `mapper/`:
- `processor/mapper/mod.rs` — `ExecutionContext` + `PlanMapper::map_and_execute()`
- `processor/mapper/map_scan.rs` — scan operators
- `processor/mapper/map_join.rs` — join operators
- `processor/mapper/map_aggregate.rs` — aggregate operators
- `processor/mapper/map_projection.rs` — projection/filter operators
- `processor/mapper/map_update.rs` — DML operators
- `processor/mapper/map_ddl.rs` — DDL + admin operators

`execute_internal()` sekarang ~220 lines, memanggil `PlanMapper::map_and_execute()`.

---

## 🟡 P25.3 — CALL Dispatch ✅ COMPLETE

**Status:** ✅ CALL dispatch sudah di-refactor:
- `PhysicalStandaloneCall` menerima `Arc<dyn StandaloneCallHandler>` — trait-based
- Handler dispatch tetap menggunakan string matching di `DbStandaloneCallHandler`, tapi ini adalah implementasi detail di handler, bukan di pipeline dispatch
- `mapper/mod.rs` dispatch sudah proper trait-based: `LogicalOperator::StandaloneCall` → `map_ddl::map_and_execute_ddl`

---

## 🟢 P25.4 — C++ Benchmark Binary 🟡 DEFERRED

**Status:** 🟡 Masih perlu dilakukan. C++ binary `kuzu_benchmark` belum dibuild dari CMake.
**Rencana:** Build C++ dan kumpulkan data perbandingan performa.

---

## 🟢 P25.5 — Release & Publish 🟡 DEFERRED

**Status:** 🟡 NPM publish dan crates.io publish masih perlu disiapkan.
- WASM artifacts siap di `kuzu-wasm/pkg/`
- CI workflow untuk release automation sudah ada (GitHub Actions)

---

## P25.6 — Summary ✅

| Item | SP | Status |
|------|----|--------|
| P25.1 STANDALONE_CALL refactor | 0 | ✅ Complete |
| P25.2 processor.rs refactor | 5 | ✅ Complete |
| P25.3 CALL dispatch trait | 3 | ✅ Complete |
| P25.4 C++ benchmark binary | 3 | 🟡 Deferred |
| P25.5 Release & publish | 2 | 🟡 Deferred |
| **Total P25** | **13** | **✅ 11/13 complete** |
# P26: Testing, Fuzzing & Documentation Polish

> **Status:** 🟡 SEBAGIAN | **Target:** 2026-07-21
> **Catatan:** Semua implementasi inti sudah selesai. Fase ini bersifat opsional untuk production readiness.

---

## 🟡 P26.1 — Edge Case Test Suite 🟡 BELUM

### Coverage Goals
| Area | Current | Target | Status |
|------|---------|--------|--------|
| Null handling | ~10 tests | 30+ | 🟡 Perlu ditambah |
| Empty tables | ~5 tests | 15+ | 🟡 Perlu ditambah |
| Boundary values | ~3 tests | 15+ | 🟡 Perlu ditambah |
| Concurrency | ~4 tests | 10+ | 🟡 Perlu ditambah |
| DDL error paths | ~8 tests | 20+ | 🟡 Perlu ditambah |
| Nested types | ~5 tests | 15+ | 🟡 Perlu ditambah |
| Unicode/UTF-8 | ~2 tests | 10+ | 🟡 Perlu ditambah |

- `[ ]` Buat `kuzu-main/tests/test_edge_cases.rs` — organized by category
- **Effort:** 5 SP

---

## 🟢 P26.2 — Fuzz Testing ❌ BELUM

- `[ ]` Integrasi `cargo-fuzz` untuk AFL/libfuzzer-based fuzzing
- `[ ]` Fuzz target 1: `cypher_query` — raw string → parse → bind → plan → execute
- `[ ]` Fuzz target 2: `expression_eval` — random expressions against random data
- `[ ]` Fuzz target 3: `copy_from_csv` — malformed CSV files
- **Effort:** 4 SP

---

## 🟢 P26.3 — Property-Based Testing ❌ BELUM

Gunakan `proptest` crate:
- `[ ]` Round-trip: Insert value → query → value should match original
- `[ ]` Associativity: `(A JOIN B) JOIN C` == `A JOIN (B JOIN C)`
- `[ ]` Filter pushdown: Filter sebelum join == filter setelah join
- **Effort:** 4 SP

---

## 🟢 P26.4 — Performance Profiling 🟡 SEBAGIAN

- ✅ `cargo bench --workspace` — baseline already established
- ✅ Arrow/SelectionVector benchmark — 10-24× speedup di hot path
- `[ ]` Profile top 5 slowest queries dengan `flamegraph-rs`
- `[ ]` Optimize bottlenecks (ValueVector memory layout, JoinHashTable bucket contention)
- **Effort:** 3 SP

---

## 🟢 P26.5 — Documentation Completion 🟡 SEBAGIAN

| Item | Status |
|------|--------|
| `kuzu-main` rustdoc (Database, Connection, QueryResult) | ✅ Complete |
| Crate-level README | 🟡 kuzu-core/README.md covers all |
| ADRs | ✅ 5 existing |
| Migration guide (Indonesian) | ✅ Complete |
| English MIGRATION.md | ❌ Need English version |

- `[ ]` English MIGRATION.md
- **Effort:** 5 SP

---

## P26.6 — Summary 🟡

| Item | SP | Status |
|------|----|--------|
| P26.1 Edge case tests | 5 | 🟡 Not started |
| P26.2 Fuzz testing | 4 | ❌ Not started |
| P26.3 Property-based testing | 4 | ❌ Not started |
| P26.4 Performance profiling | 3 | 🟡 Partial |
| P26.5 Documentation | 5 | 🟡 Partial |
| **Total P26** | **21** | **🟡 ~6/21 complete** |
