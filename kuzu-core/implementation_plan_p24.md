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
