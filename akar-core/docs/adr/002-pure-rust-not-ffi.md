# ADR 002: Pure Rust, bukan FFI/cxx

> **Status:** Accepted | **Date:** 2026-07-07 | **Last Updated:** 2026-07-19

## Context

Kùzu C++ memiliki ~200K LOC. Porting bisa dilakukan via:
1. FFI wrapper (memanggil C++ dari Rust)
2. Auto-generated binding (cxx, bindgen)
3. Pure Rust rewrite

## Decision

**Pure Rust rewrite** — semua kode C++ diporting ulang ke Rust idiomatik, tanpa FFI.

## Rationale

| Faktor | FFI/cxx | Pure Rust |
|--------|---------|-----------|
| **Safety** | unsafe blocks di boundary | 100% safe Rust |
| **Performance** | Overhead serialisasi antar bahasa | Zero-cost abstractions |
| **Maintainability** | Dua codebase (C++ + Rust wrapper) | Satu codebase |
| **WASM support** | Tidak mungkin (C++ di browser) | ✅ `wasm32-unknown-unknown` |
| **Compile time** | C++ compile + Rust compile | Rust only |

## Consequences

- ~86.000 LOC Rust ditulis dari nol (setara ~200K LOC C++)
- 32 crate, 1,346 test (1,345 passing + 1 failing `test_count_variable` = P48.14, per 2026-08-06), 25 optimizer passes
- Tidak ada ketergantungan pada C++ compiler
- WASM target didukung penuh (kecuali crate native: duckdb, postgres, dll)
