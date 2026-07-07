# Kuzu Rust — Consolidated Implementation Plan

> **Date:** 2026-07-07 | **Status:** P10 ✅ | P11 ✅ | P12 🔄 | P13-P15 ❌
> **Prerequisites:** P9 (Production Hardening) ✅, P10 (Critical C++ Parity) ✅

---

## Phase Status Overview

| Phase | Content | Priority | SP | Status |
|-------|---------|----------|-----|--------|
| **P10** | COPY TO, TRANSACTION, EXTENSION, nullif, count_if | 🔴 P0 | 20 | ✅ COMPLETE |
| **P11** | size(), export_csv/parquet, ATTACH/DETACH, LOAD FROM | 🟡 P1 | 13 | ✅ COMPLETE |
| **P12** | TOP_K, INDEX_LOOKUP, lambda list, path/pattern funcs | 🟡 P1 | 13 | ❌ |
| **P13** | GDS expansion, CREATE GRAPH, GDS_CALL | 🟢 P2 | 13 | ❌ |
| **P14** | Storage features, error() func, misc | 🟢 P2 | 8 | ❌ |
| **P15** | Types: JSON, UINT128, DTime | 🟢 P3 | 5 | ❌ |
| **Total** | | | **72** | |

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

## P12 — Physical Operators & Lambda Functions

### ❌ P12.1 — TOP_K physical operator

- `[ ]` **Planner** — `LogicalOperator::TopK(LogicalTopK)`
- `[ ]` **Optimizer** — Update `TopKOptimization` pass to fuse ORDER BY + LIMIT
- `[ ]` **Processor** — `PhysicalTopK` with `BinaryHeap` (size k)
- `[ ]` **Verify** — `cargo test -p kuzu-processor` (77 passing)

### ❌ P12.2 — INDEX_LOOKUP

- `[ ]` **Planner** — `LogicalOperator::IndexLookup`
- `[ ]` **Processor** — `PhysicalIndexLookup` using ART index point lookup
- `[ ]` **Verify** — point queries via primary key

### ❌ P12.3 — BATCH_INSERT

- `[ ]` **Planner** — `LogicalOperator::BatchInsert`
- `[ ]` **Processor** — `PhysicalBatchInsert` using existing `insert_rows_batch()`
- `[ ]` **Verify** — large COPY FROM performance

### ❌ P12.4 — `list_transform` / `list_reduce` / `list_filter`

Lambda-based list operations:
- `[ ]` `list_transform(list, x -> expr)` — apply expression to each element
- `[ ]` `list_reduce(list, (acc, x) -> expr, initial)` — fold
- `[ ]` `list_filter(list, x -> condition)` — filter by predicate
- `[ ]` **Files:** `kuzu-function/src/scalar/list.rs`, `registry.rs`

### ❌ P12.5 — Path functions: `properties`, `is_trail`, `is_acyclic`

- `[ ]` `properties(path)` — extract all properties from path
- `[ ]` `is_trail(path)`, `is_acyclic(path)` — path validation
- `[ ]` Already have `PathOp` infrastructure in `scalar/path.rs`

### ❌ P12.6 — Pattern functions: `cost`, `id`, `label`, `rowid`

- `[ ]` `cost(pattern)`, `id(pattern)`, `label(pattern)`, `rowid(pattern)`
- `[ ]` **Files:** `kuzu-function/src/scalar/schema.rs`

---

## P13 — GDS Expansion & Graph Management

### ❌ P13.1 — Dijkstra (weighted shortest path)

- `[ ]` Already have weighted `RecursiveExtend` — expose as standalone GDS function
- `[ ]` `CALL dijkstra(source, target, weight_prop)`

### ❌ P13.2 — Louvain Community Detection

- `[ ]` Implement Louvain algorithm in `kuzu-algo`

### ❌ P13.3 — K-Core Decomposition

- `[ ]` Implement K-Core in `kuzu-algo`

### ❌ P13.4 — CREATE/USE/DROP GRAPH

- `[ ]` `CREATE PROJECTION GRAPH name AS (MATCH pattern)`
- `[ ]` `USE GRAPH name`
- `[ ]` `DROP GRAPH name`

### ❌ P13.5 — GDS_CALL

- `[ ]` `CALL page_rank(...)`, `CALL wcc(...)`, `CALL scc(...)`

---

## P14 — Storage & Utilities

### ❌ P14.1 — Parquet writer improvements
### ❌ P14.2 — NPY reader
### ❌ P14.3 — HyperLogLog cardinality stats
### ❌ P14.4 — Roaring bitmap
### ❌ P14.5 — `error()` utility function

---

## P15 — Types

### ❌ P15.1 — JSON native type
### ❌ P15.2 — UINT128
### ❌ P15.3 — DTime (DateTime with TZ offset)

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
