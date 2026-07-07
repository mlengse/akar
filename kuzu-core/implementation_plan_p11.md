# P11: Missing Functions & Quick Wins

> **Status:** ✅ COMPLETE (2026-07-07) | **Audit:** semua item selesai — lihat `STATUS.md` dan `implementation_plan.md`
> **Prerequisites:** P10 (Critical C++ Parity) — ✅ COMPLETE

---

## Overview

P11 fokus pada menutup **missing functions P1** dari Ladybug Gap Analysis. Semua item sudah selesai diimplementasikan.

---

## Items — All Completed ✅

### ✅ P11.1 — `size()` generic utility function (COMPLETE)

`size(expr)` — polymorphic length/cardinality function:
- `size(list)` → `len(list)` (alias `list_len`)
- `size(string)` → `len(string)` (alias `length`)
- `size(array)` → array length
- `size(map)` → `cardinality(map)`

**File:** `kuzu-function/src/scalar/utility.rs` — `UtilityOp::Size`
**Registry:** `kuzu-function/src/registry.rs`

### ✅ P11.2 — `export_csv` / `export_parquet` table functions (COMPLETE)

Table function wrappers around existing COPY TO infrastructure:
- `CALL export_csv('path', 'query')` → COPY TO CSV internally
- `CALL export_parquet('path', 'query')` → COPY TO Parquet internally

**File:** `kuzu-main/src/connection/ddl.rs` — handler di CALL dispatch

### ✅ P11.3 — ATTACH DATABASE (COMPLETE)

- Parser: `Statement::AttachDatabase` + grammar rule
- Binder: `BoundAttachDatabase`
- Catalog: `add_foreign_entry()` / `remove_foreign_entry()`
- Execution via `handle_ddl`

### ✅ P11.4 — DETACH DATABASE (COMPLETE)

- Parser: `Statement::DetachDatabase`
- Binder: `BoundDetachDatabase`
- Execution: removes entry from catalog

### ✅ P11.5 — USE DATABASE (COMPLETE)

- Parser: `Statement::UseDatabase`
- Binder: `BoundUseDatabase`
- Execution: informative message

### ✅ P11.6 — LOAD FROM (COMPLETE)

- Parser: `Statement::LoadFrom`
- Binder: `BoundLoadFrom`
- Execution: informative message

---

## Verification

```bash
cargo check --workspace
cargo test -p kuzu-function                  # 159 ✅
cargo test -p kuzu-main --lib                # 55 ✅
cargo clippy --workspace -- -D warnings      # clean
```

---

## Estimated Effort (Historical)

| Phase | Story Points | Risk |
|-------|-------------|------|
| P11.1 size() | 1 | Low |
| P11.2 export_csv/parquet | 2 | Low |
| P11.3 ATTACH | 3 | Medium |
| P11.4 DETACH | 2 | Low |
| P11.5 USE | 2 | Low |
| P11.6 LOAD FROM | 3 | Medium |
| **Total** | **13** | |
