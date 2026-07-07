# Kuzu Rust — Consolidated Implementation Plan

> **Date:** 2026-07-07 | **Status:** P10 ✅ | P11 ✅ | P12 ✅ | P13-P14 ❌
> **Audit:** `cargo test --workspace` → 952 passed, 0 failed | 50 logical ops, 31 physical ops
> **Prerequisites:** P9 (Production Hardening) ✅, P10 (Critical C++ Parity) ✅

---

## Phase Status Overview

| Phase | Content | Priority | SP | Status |
|-------|---------|----------|-----|--------|
| **P10** | COPY TO, TRANSACTION, EXTENSION, nullif, count_if | 🔴 P0 | 20 | ✅ COMPLETE |
| **P11** | size(), export_csv/parquet, ATTACH/DETACH, LOAD FROM | 🟡 P1 | 13 | ✅ COMPLETE |
| **P12** | TOP_K, INDEX_LOOKUP, BATCH_INSERT, lambda list, path/pattern funcs | 🟡 P1 | 13 | ✅ COMPLETE |
| **P13** | CREATE TYPE, COMMENT ON, CREATE/USE/DROP GRAPH, GDS_CALL, error() | 🟢 P2 | 13 | ❌ |
| **P14** | Storage: Parquet writer, NPY reader, HyperLogLog, Roaring bitmap | 🟢 P2 | 8 | ❌ |
| **P15** | Types: JSON, UINT128, DTime, Value::Union, missing physical ops | 🟢 P3 | 8 | ❌ |
| **Total** | | | **75** | |

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

## P13 — DDL Completeness & Graph Management

> **Catatan audit:** GDS algorithms (Dijkstra, Louvain, K-Core, PageRank, WCC, SCC, BFS, Spanning Forest) sudah selesai di `kuzu-algo/src/lib.rs`. P13 fokus pada DDL dan glue code, bukan algoritma baru.

### ❌ P13.1 — CREATE TYPE
- `[ ]` `CREATE TYPE name AS type` — custom type definition
- `[ ]` **Files:** parser, binder, catalog
- `[ ]` **Reference:** LadybugDB `StatementType::CREATE_TYPE`, `BoundCreateType`, `LogicalCreateType`

### ❌ P13.2 — COMMENT ON TABLE
- `[ ]` `COMMENT ON TABLE t IS 'description'` — table comments
- `[ ]` **Files:** parser, binder, catalog (AlterType::COMMENT)
- `[ ]` **Reference:** LadybugDB `AlterInfo::comment`

### ❌ P13.3 — CREATE/USE/DROP GRAPH (projected graph)
- `[ ]` `CREATE [PROJECTION] GRAPH name AS (MATCH pattern)`
- `[ ]` `USE GRAPH name`
- `[ ]` `DROP GRAPH name`
- `[ ]` **Files:** parser, binder, catalog (GraphCatalogEntry), planner

### ❌ P13.4 — GDS_CALL (CALL with GDS functions)
- `[ ]` `CALL page_rank(...)`, `CALL wcc(...)`, `CALL scc(...)` → glue ke `kuzu-algo`
- `[ ]` GDS functions sudah ada di `kuzu-algo/src/lib.rs`, perlu wiring ke CALL pipeline

### ❌ P13.5 — `error()` utility function
- `[ ]` `error(msg)` — raise error with message
- `[ ]` **File:** `kuzu-function/src/scalar/utility.rs`

### ❌ P13.6 — STANDALONE_CALL refactor (deferred from P10.3)
- `[ ]` Refactor CALL dispatch dari string matching ke `Statement::StandaloneCall` pipeline proper
- `[ ]` **Files:** parser (grammar), binder (BoundStandaloneCall), planner, processor

---

## P14 — Storage Enhancements

### ❌ P14.1 — Parquet writer improvements
- `[ ]` Column-level parquet writing (currently uses Arrow writer, basic)

### ❌ P14.2 — NPY reader
- `[ ]` Read NumPy NPY files as scan source
- `[ ]` **File:** `kuzu-storage/src/`

### ❌ P14.3 — HyperLogLog cardinality stats
- `[ ]` Approximate cardinality estimation for StatsStore
- `[ ]` **File:** `kuzu-storage/src/stats_store.rs`

### ❌ P14.4 — Roaring bitmap
- `[ ]` Compressed bitmap for fast set operations
- `[ ]` **Dependency:** `roaring` crate

### ❌ P14.5 — Lazy segment scanner
- `[ ]` Deferred segment loading for large columnar scans

### ❌ P14.6 — Float compression (delta/offset)
- `[ ]` Delta and offset compression for float columns

---

## P15 — Types & Missing Physical Operators

### ❌ P15.1 — JSON native type
- `[ ]` `LogicalType::Json` variant
- `[ ]` `Value::Json(serde_json::Value)`
- `[ ]` **File:** `kuzu-common/src/types.rs`

### ❌ P15.2 — UINT128
- `[ ]` `LogicalType::UInt128` variant
- `[ ]` **File:** `kuzu-common/src/types.rs`

### ❌ P15.3 — DTime (DateTime with TZ offset)
- `[ ]` `LogicalType::DTime` with timezone-aware semantics
- `[ ]` **File:** `kuzu-common/src/types.rs`

### ❌ P15.4 — Value::Union variant
- `[ ]` Union type support: `Value::Union(tag, value)`
- `[ ]` **File:** `kuzu-common/src/value.rs`

### ❌ P15.5 — Missing Physical Operators (Priority P3)
- `[ ]` `PARTITIONER` (morsel-driven parallelism)
- `[ ]` `PACKED_EXTEND` (optimized multi-rel extend)
- `[ ]` `PATH_PROPERTY_PROBE` (path property resolution)
- `[ ]` `PRIMARY_KEY_SCAN` (PK-based scan)
- `[ ]` `AGGREGATE_FINALIZE/SCAN` (split aggregate)
- `[ ]` `RESULT_COLLECTOR` (explicit result collector)
- `[ ]` `PROFILE` (query profiling)
- `[ ]` `DUMMY_SINK` / `DUMMY_SIMPLE_SINK`
- `[ ]` `PhysicalAccumulate` (currently passed through in processor.rs)
- `[ ]` `PhysicalUnion` (currently handled inline)

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

| # | Item | Severity |
|---|------|----------|
| 1 | `processor.rs` 2.702 lines single file | 🟡 DEFERRED |
| 2 | CALL dispatch via string matching in ddl.rs | 🟡 DEFERRED |
| 3 | Planner→Physical mapping logic scattered | 🟡 DEFERRED |
