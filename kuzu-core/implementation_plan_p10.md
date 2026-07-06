# P10: Critical C++ Parity — COPY TO, TRANSACTION, STANDALONE CALL, Extension Mgmt + Refactor

> **Status:** In Progress (P10.1 ✅, P10.2–P10.6 pending) | **Target Date:** 2026-07-14
> **Prerequisites:** P9 (Production Hardening) — ✅ COMPLETE

---

## Overview

Berdasarkan Ladybug Gap Analysis (§8 STATUS.md) + Architecture Audit (§9 STATUS.md), fase P10 menutup **4 gap kritis P0** + **3 fungsi missing** + **1 refactor arsitektur** untuk meningkatkan paritas dari ~85% ke ~90%.

**Prioritas baru:** P10.6 (refactor `physical_operator.rs`) harus dilakukan sebelum P11 untuk menghindari akumulasi technical debt. Lihat juga `implementation_plan_modularization.md` untuk rencana pemecahan 7 file besar lainnya.

---

## Prioritas

### 🔴 P10.1 — COPY TO (Export Data) ✅ COMPLETE (2026-07-07)

- `[x]` **Parser** — `Statement::CopyTo` + rule `copy_to` di `cypher.pest`
  - `[x]` Syntax: `COPY (query) TO 'path' (FORMAT 'CSV'|'PARQUET', HEADER true|false)`
- `[x]` **AST** — `CopyTo` struct + `CopyToFormat` enum (`Csv`, `Parquet`)
- `[x]` **Parser** — `parse_copy_to()` function
- `[x]` **Binder** — `BoundCopyTo` + `bind_copy_to()` — binds inner query
- `[x]` **Execution** — `handle_ddl` in connection.rs: executes inner query, writes CSV with proper escaping
  - `[x]` CSV writer with header support and proper value escaping
  - `[x]` Parquet writer — `#[cfg(feature = "parquet-export")]` via `arrow::StringArray` + `parquet::ArrowWriter`
- `[x]` **Tests** — `test_copy_to.rs` (4 tests): basic CSV, no-header, empty result, parquet (with/without feature)
- `[x]` **Clippy clean** — all workspace passes `-D warnings`

### 🔴 P10.2 — TRANSACTION Statement (parsed DDL)

- `[ ]` **Parser** — Tambah `Statement::Transaction` + rule `transaction_statement`
  - `[ ]` `BEGIN [TRANSACTION|WORK]`
  - `[ ]` `COMMIT [TRANSACTION|WORK]`
  - `[ ]` `ROLLBACK [TRANSACTION|WORK]`
  - `[ ]` `CHECKPOINT`
- `[ ]` **Binder** — Tambah `BoundTransaction` dengan `TransactionAction` enum
  - `[ ]` `BEGIN`, `COMMIT`, `ROLLBACK`, `CHECKPOINT`
- `[ ]` **Planner** — Tambah `LogicalOperator::Transaction`
- `[ ]` **Processor** — Tambah `PhysicalTransaction`
  - `[ ]` `BEGIN` → panggil `Connection::begin_write_txn()`
  - `[ ]` `COMMIT` → panggil `Connection::commit_write_txn()`
  - `[ ]` `ROLLBACK` → panggil `Connection::rollback_write_txn()`
  - `[ ]` `CHECKPOINT` → panggil `StorageManager::checkpoint()`
  - `[ ]` Connection reference needed — pass via processor
- `[ ]` **Refactor** — Pindahkan handling TRANSACTION dari `Connection::query()` string matching ke pipeline proper
- `[ ]` **Tests** — `test_transaction.rs`:
  - `[ ]` BEGIN + CREATE + COMMIT + verify data persists
  - `[ ]` BEGIN + CREATE + ROLLBACK + verify data NOT persisted
  - `[ ]` CHECKPOINT
  - `[ ]` Error: COMMIT tanpa BEGIN, nested BEGIN

### 🔴 P10.3 — STANDALONE_CALL (Top-Level CALL)

- `[ ]` **Parser** — Tambah `Statement::StandaloneCall` + rule `standalone_call`
  - `[ ]` `CALL function_name(arg1, arg2, ...)`
  - `[ ]` `CALL function_name(arg1, arg2, ...) RETURN *`
- `[ ]` **Binder** — Tambah `BoundStandaloneCall`
  - `[ ]` Resolve function name di `FunctionRegistry`
  - `[ ]` Validasi argument count dan types
- `[ ]` **Planner** — Tambah `LogicalOperator::StandaloneCall`
- `[ ]` **Processor** — Tambah `PhysicalStandaloneCall`
  - `[ ]` Dispatch ke table function registry
  - `[ ]` Support RETURN clause untuk filtering output columns
- `[ ]` **Tests** — `test_standalone_call.rs`:
  - `[ ]` `CALL show_tables()` via parsed statement (bukan string matching)
  - `[ ]` `CALL table_info('Person')`
  - `[ ]` `CALL show_functions() RETURN name`
  - `[ ]` Error: unknown function, wrong args

### 🔴 P10.4 — INSTALL / LOAD / UNINSTALL EXTENSION

- `[ ]` **Parser** — Tambah `Statement::Extension` + rule `extension_statement`
  - `[ ]` `INSTALL [EXTENSION] name`
  - `[ ]` `LOAD [EXTENSION] name`
  - `[ ]` `UNINSTALL [EXTENSION] name`
- `[ ]` **Binder** — Tambah `BoundExtension` dengan `ExtensionAction` enum
- `[ ]` **Planner** — Tambah `LogicalOperator::Extension`
- `[ ]` **Processor** — Tambah `PhysicalExtension`
  - `[ ]` `LOAD` → panggil `ExtensionRegistry::load(name)`
  - `[ ]` `INSTALL` → download + load (deferred: butuh repo URL config)
  - `[ ]` `UNINSTALL` → panggil `ExtensionRegistry::unload(name)`
  - `[ ]` Perlu `ExtensionRegistry` reference di processor
- `[ ]` **Tests** — `test_extension_mgmt.rs`:
  - `[ ]` LOAD EXTENSION json (verify functions registered)
  - `[ ]` LOAD EXTENSION fts
  - `[ ]` UNINSTALL EXTENSION json (verify functions removed)
  - `[ ]` Error: unknown extension, already loaded

---

## Additional Functions

### 🟡 P10.5 — Missing Scalar/Aggregate Functions

- `[ ]` **`nullif(expr, value)`** — return NULL if expr == value, else expr
  - File: `kuzu-function/src/scalar.rs`, tambah `UtilityOp::NullIf`
- `[ ]` **`count_if(condition)`** — aggregate: COUNT rows where condition is TRUE
  - File: `kuzu-function/src/scalar.rs`, tambah `AggregateOp::CountIf`
- `[ ]` **`export_csv(path, query)`** — table function
  - Delegasikan ke `PhysicalCopyTo` internally
- `[ ]` **`export_parquet(path, query)`** — table function
  - Sama seperti export_csv
- `[ ]` **Tests** di `kuzu-function/src/scalar.rs` dan integration tests

---

## Verification Plan

```bash
# Full build
cargo build --workspace

# P10 tests
cargo test --test test_copy_to -p kuzu-main
cargo test --test test_transaction -p kuzu-main
cargo test --test test_standalone_call -p kuzu-main
cargo test --test test_extension_mgmt -p kuzu-main

# Regression
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

---

## Dependencies

| Crate | New Deps | Purpose |
|-------|----------|---------|
| `kuzu-processor` | `csv` (already workspace dep) | CSV writer for COPY TO |
| `kuzu-processor` | `parquet` (already workspace dep), `arrow` | Parquet writer for COPY TO |
| `kuzu-function` | none | nullif, count_if |

---

## Estimated Effort

| Phase | Story Points | Risk |
|-------|-------------|------|
| P10.1 COPY TO | 5 | Medium (Parquet writer) |
| P10.2 TRANSACTION | 3 | Low |
| P10.3 STANDALONE_CALL | 3 | Low |
| P10.4 EXTENSION Mgmt | 5 | Medium (INSTALL needs repo config) |
| P10.5 Functions | 2 | Low |
| **P10.6 — Refactor physical_operator.rs** | **5** | Low (murni reorganisasi kode) |
| **Total** | **23** | |


---

## 🟡 P10.6 — Refactor: Split physical_operator.rs (Architecture Debt)

> **Prioritas:** Sebelum P11/P12 | **Risk:** Low (compiler-guided, no behavior change)
> **Bagian dari:** `implementation_plan_modularization.md` Phase 2A

### Problem
`kuzu-processor/src/physical_operator.rs` saat ini ~2500+ LOC berisi 28+ tipe physical operator dalam 1 file. Bandingkan dengan C++ Ladybug yang punya 32 file `.cpp` terpisah + 50 file `map_*.cpp`. Ini menyulitkan navigasi, code review, dan penambahan operator baru.

### Target Struktur

```
kuzu-processor/src/
├── operators/
│   ├── mod.rs
│   ├── scan.rs                 # PhysicalScan, PhysicalScanRel
│   ├── filter.rs               # PhysicalFilter
│   ├── projection.rs           # PhysicalProjection
│   ├── hash_join.rs            # PhysicalHashJoin + JoinHashTable
│   ├── cross_product.rs        # PhysicalCrossProduct
│   ├── intersect.rs            # PhysicalIntersect
│   ├── semi_join.rs            # PhysicalSemiJoin
│   ├── anti_join.rs            # PhysicalAntiJoin
│   ├── aggregate.rs            # PhysicalAggregate + AggregateHashTable
│   ├── order_by.rs             # PhysicalOrderBy + BlockMergeSorter
│   ├── limit.rs                # PhysicalLimit
│   ├── union.rs                # PhysicalUnion
│   ├── flatten.rs              # PhysicalFlatten
│   ├── semi_masker.rs          # PhysicalSemiMasker + NodeSemiMask
│   ├── recursive_extend.rs     # PhysicalRecursiveExtend
│   ├── explain.rs              # PhysicalExplain
│   ├── foreach.rs              # PhysicalForeach
│   ├── ddl/
│   │   ├── mod.rs
│   │   ├── copy_from.rs        # PhysicalCopyFrom
│   │   ├── copy_to.rs          # PhysicalCopyTo
│   │   ├── create_table.rs
│   │   ├── drop_table.rs
│   │   ├── alter_table.rs
│   │   ├── create_index.rs
│   │   └── drop_index.rs
│   └── ...
├── expression_evaluator.rs     # Tetap
├── lib.rs
└── processor.rs                # Tetap
```

### Langkah

- `[ ]` **1. Buat direktori** `kuzu-processor/src/operators/` + `operators/ddl/`
- `[ ]` **2. Ekstrak per kelompok operator** — Pindahkan struct + impl ke file masing-masing
- `[ ]` **3. `operators/mod.rs`** — Re-export semua operator dengan `pub use`
- `[ ]` **4. Update `lib.rs`** — `pub mod operators` menggantikan `pub mod physical_operator`
- `[ ]` **5. Update `processor.rs`** — Import dari `operators::*` bukan `physical_operator::*`
- `[ ]` **6. Update downstream crates** — `kuzu-main`, `kuzu-binder` jika ada import langsung
- `[ ]` **7. Verifikasi** — `cargo check`, `cargo test -p kuzu-processor` (77 passing), `cargo test --workspace` (954 passing)

### Verifikasi

```bash
cargo check -p kuzu-processor
cargo test -p kuzu-processor      # Harus tetap 77 passing
cargo test --workspace             # Harus tetap 954 passing
cargo clippy --workspace -- -D warnings
```

---

## Success Criteria

| # | Criteria |
|---|----------|
| 1 | `COPY (MATCH (n) RETURN n) TO 'out.csv' (FORMAT 'CSV', HEADER true)` works |
| 2 | `BEGIN; CREATE ...; COMMIT;` / `ROLLBACK;` works via pipeline |
| 3 | `CALL show_tables()` returns results via parsed statement |
| 4 | `LOAD EXTENSION json` registers json_* functions |
| 5 | `nullif(expr, val)`, `count_if(cond)` return correct results |
| 6 | All 954 existing tests still pass |
| 7 | Clippy `-D warnings` clean |
| 8 | `physical_operator.rs` split into `operators/*.rs` — verified by `cargo test --workspace` | |
