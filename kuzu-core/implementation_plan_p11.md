# P11: Missing Functions & Quick Wins

> **Status:** In Progress | **Target:** 2026-07-08
> **Prerequisites:** P10 (Critical C++ Parity) — ✅ COMPLETE

---

## Overview

P11 fokus pada menutup **missing functions P1** dari Ladybug Gap Analysis (§8.4 STATUS.md) + **TOP_K physical operator**. Fase ini lebih kecil dari P10 (8 SP vs 23 SP) dan fokus pada fungsi-fungsi yang bisa diimplementasikan secara independen.

---

## Prioritas

### 🟡 P11.1 — `size()` generic utility function

`size(expr)` — polymorphic length/cardinality function:
- `size(list)` → `len(list)` (alias `list_len`)
- `size(string)` → `len(string)` (alias `length`)
- `size(array)` → array length
- `size(map)` → `cardinality(map)`

**File:** `kuzu-function/src/scalar/utility.rs` — tambah `UtilityOp::Size`

### 🟡 P11.2 — `export_csv` / `export_parquet` table functions

Table function wrappers around existing COPY TO infrastructure:
- `CALL export_csv('path', 'query')` → COPY TO CSV internally
- `CALL export_parquet('path', 'query')` → COPY TO Parquet internally

**File:** `kuzu-main/src/connection/ddl.rs` — tambah handler di CALL dispatch

### 🟡 P11.3 — TOP_K / TOP_K_SCAN physical operator

Fused top-k optimization: combine ORDER BY + LIMIT into a single operator
that maintains a heap of size k, avoiding full sort.

**Files:** `kuzu-planner/src/logical_operator.rs`, `kuzu-processor/src/physical/write_ops.rs`, `kuzu-optimizer/src/passes/flat/top_k.rs`

### 🟡 P11.4 — `list_transform` / `list_reduce` / `list_filter` (lambda list)

Lambda-based list operations that apply a function to each list element.

**Files:** `kuzu-function/src/scalar/list.rs`, `kuzu-function/src/registry.rs`

---

## Verification Plan

```bash
cargo check --workspace
cargo test -p kuzu-function
cargo test -p kuzu-main --lib
cargo clippy --workspace -- -D warnings
```

---

## Estimated Effort

| Phase | Story Points | Risk |
|-------|-------------|------|
| P11.1 size() | 1 | Low |
| P11.2 export_csv/parquet | 2 | Low |
| P11.3 TOP_K | 3 | Medium |
| P11.4 list_transform/reduce/filter | 3 | Medium |
| **Total** | **9** | |
