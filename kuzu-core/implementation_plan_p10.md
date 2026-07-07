# P10: Critical C++ Parity — COPY TO, TRANSACTION, STANDALONE CALL, Extension Mgmt + Refactor

> **Status:** ✅ COMPLETE (P10.1–P10.6 done; P10.3 deferred) | **Completed:** 2026-07-07
> **Prerequisites:** P9 (Production Hardening) — ✅ COMPLETE
> **Audit:** `cargo test --workspace` → 960 passed, 0 failed

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

### 🔴 P10.2 — TRANSACTION Statement (parsed DDL) ✅ COMPLETE (2026-07-07)

- `[x]` **Parser** — Tambah `Statement::Transaction` + rule `transaction_statement`
  - `[x]` `BEGIN [TRANSACTION|WORK]`
  - `[x]` `COMMIT [TRANSACTION|WORK]`
  - `[x]` `ROLLBACK [TRANSACTION|WORK]`
  - `[x]` `CHECKPOINT`
- `[x]` **Binder** — Tambah `BoundTransaction` dengan `TransactionAction` enum
  - `[x]` `BEGIN`, `COMMIT`, `ROLLBACK`, `CHECKPOINT`
- `[x]` **Execution** — Handle langsung di `handle_ddl` (tidak perlu planner/processor untuk side-effect ops)
  - `[x]` `BEGIN` → panggil `Connection::begin_write_txn()`
  - `[x]` `COMMIT` → panggil `Connection::commit_write_txn()`
  - `[x]` `ROLLBACK` → panggil `Connection::rollback_write_txn()`
  - `[x]` `CHECKPOINT` → panggil `do_sync_checkpoint()`
- `[x]` **Refactor** — Pindahkan handling TRANSACTION dari `Connection::query()` string matching ke pipeline proper
- `[x]` **Verifikasi** — `cargo check`, `cargo clippy`, `cargo test` all pass

### 🔴 P10.3 — STANDALONE_CALL (Top-Level CALL) 🟡 DEFERRED

> **Rationale:** CALL already works through the parser → binder → ddl.rs pipeline. The
> current string-based dispatch in `handle_ddl` is stable and well-tested. A full
> refactor to `Statement::StandaloneCall` + `PhysicalStandaloneCall` would duplicate the
> existing `Call` pipeline for architectural purity without adding new functionality.
> Deferred to P12 (when physical operator table-function dispatch will be redesigned).

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

### 🔴 P10.4 — INSTALL / LOAD / UNINSTALL EXTENSION ✅ COMPLETE (2026-07-07)

- `[x]` **Parser** — Tambah `Statement::Extension` + rule `extension_statement`
  - `[x]` `INSTALL [EXTENSION] name`
  - `[x]` `LOAD [EXTENSION] name`
  - `[x]` `UNINSTALL [EXTENSION] name`
- `[x]` **Binder** — Tambah `BoundExtension` dengan `ExtensionAction` enum
- `[x]` **Execution** — Handle di `handle_ddl` dengan pesan informatif
  - `[x]` `LOAD` / `INSTALL` → arahkan ke compile-time features
  - `[x]` `UNINSTALL` → arahkan ke rebuild tanpa feature flag
  - `[x]` Runtime dynamic loading deferred (extensions are compile-time in Kuzu Rust)
- `[x]` **Verifikasi** — `cargo check`, `cargo clippy`, `cargo test` all pass

---

## Additional Functions

### 🟡 P10.5 — Missing Scalar/Aggregate Functions ✅ COMPLETE (2026-07-07)

- `[x]` **`nullif(expr, value)`** — return NULL if expr == value, else expr
  - File: `kuzu-function/src/scalar/utility.rs` + `registry.rs`, `UtilityOp::NullIf`
- `[x]` **`count_if(condition)`** — aggregate: COUNT rows where condition is TRUE
  - File: `kuzu-function/src/aggregate/mod.rs` + `registry.rs`, `AggregateFunction::CountIf` + `AggValueState::CountIf`
- `[ ]` **`export_csv(path, query)`** — table function
  - Deferred (COPY TO already covers export use case)
- `[ ]` **`export_parquet(path, query)`** — table function
  - Deferred (COPY TO already covers export use case)
- `[x]` **Verifikasi** — `cargo test -p kuzu-function` (159 passing), `cargo clippy` clean

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

## 🟡 P10.6 — Refactor: Split physical_operator.rs ✅ DONE via P-MOD2A

> **Prioritas:** Sebelum P11/P12 | **Risk:** Low (compiler-guided, no behavior change)
> **Bagian dari:** `implementation_plan_modularization.md` Phase 2A

### Result
`kuzu-processor/src/physical_operator.rs` sekarang **4-line re-export stub** (`pub use crate::physical::*`).
Semua operator dipindahkan ke `kuzu-processor/src/physical/{types,common,scan_filter,order_aggregate,join_ops,write_ops}.rs` (6 files).

---

## Success Criteria

| # | Criteria | Status |
|---|----------|--------|
| 1 | `COPY (MATCH (n) RETURN n) TO 'out.csv' (FORMAT 'CSV', HEADER true)` works | ✅ |
| 2 | `BEGIN; CREATE ...; COMMIT;` / `ROLLBACK;` works via pipeline | ✅ |
| 3 | `CALL show_tables()` returns results via parsed statement | ✅ (existing) |
| 4 | `LOAD EXTENSION json` returns informative message | ✅ |
| 5 | `nullif(expr, val)`, `count_if(cond)` return correct results | ✅ |
| 6 | All 960+ existing tests still pass | ✅ |
| 7 | Clippy `-D warnings` clean | ✅ |
| 8 | `physical_operator.rs` → `physical/{6 files}` re-export stub | ✅ (P-MOD2A) | |
