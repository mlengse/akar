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
