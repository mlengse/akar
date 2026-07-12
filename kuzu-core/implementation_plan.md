# Kuzu Rust — Forward Implementation Plan

> **Note:** For the current status of the project and all completed phases (P1-P25), please refer to [`STATUS.md`](./STATUS.md).
> This document strictly focuses on the strategic roadmap and pending implementation tasks, completely separated from the status report to avoid duplication.

---

## 🎯 Roadmap Overview

| Phase | Content | Priority | SP | Target |
|-------|---------|----------|:---:|--------|
| **P26** | Testing, fuzzing & documentation polish | 🟢 P3 | 21 | Sprint 1 |
| **P27** | Performance — zero-copy Arrow, JoinHashTable | 🔴 P0 | 14 | Sprint 1-2 |
| **P28** | Drop-in replacement — C++ storage, ABI, CLI | 🔴 P0 | 23 | Sprint 2-3 |
| **P29** | Functions, fuzz, proptest, edge cases | 🟡 P1 | 18 | Sprint 3 |
| **Total** | | | **76** | **~6 weeks** |

---

## 🟡 P26: Testing, Fuzzing & Documentation Polish
*Target: 2026-07-21*

### P26.1 — Edge Case Test Suite (5 SP)
- `[ ]` Create `kuzu-main/tests/test_edge_cases.rs` organized by category:
  - Null handling (Target: 30+ tests)
  - Empty tables (Target: 15+ tests)
  - Boundary values (Target: 15+ tests)
  - Concurrency (Target: 10+ tests)
  - DDL error paths (Target: 20+ tests)
  - Nested types (Target: 15+ tests)
  - Unicode/UTF-8 (Target: 10+ tests)

### P26.2 — Fuzz Testing (4 SP)
- `[ ]` Integrate `cargo-fuzz` for AFL/libfuzzer-based fuzzing
- `[ ]` Target 1: `cypher_query` (raw string → parse → bind → plan → execute)
- `[ ]` Target 2: `expression_eval` (random expressions against random data)
- `[ ]` Target 3: `copy_from_csv` (malformed CSV files)

### P26.3 — Property-Based Testing (4 SP)
- `[ ]` Integrate `proptest` crate:
  - Round-trip: Insert value → query → value should match original
  - Associativity: `(A JOIN B) JOIN C` == `A JOIN (B JOIN C)`
  - Filter pushdown: Filter before join == filter after join

### P26.4 — Performance Profiling (3 SP)
- `[ ]` Profile top 5 slowest queries with `flamegraph-rs`
- `[ ]` Optimize bottlenecks (ValueVector memory layout, JoinHashTable bucket contention)

### P26.5 — Documentation & Deployment (8 SP)
- `[ ]` English `MIGRATION.md`
- `[ ]` Build C++ benchmark binary (`kuzu_benchmark`) from CMake (deferred from P25.4)
- `[ ]` NPM / crates.io publish release (deferred from P25.5)

---

## 🔴 P27: Performance — Zero-Copy & Optimization
*Target: Close the 3.7× performance gap to <1.5× within 3 parallel sprints.*

### P27.1 — Zero-Copy Arrow Storage Layer (8 SP)
- `[ ]` Storage output `ArrayRef` directly (skip `ValueVector`)
- `[ ]` Eliminate `from_legacy` variable lookup
- `[ ]` Pipeline fused operations (Filter + Projection in 1 pass)

### P27.2 — JoinHashTable Optimization (3 SP)
- `[ ]` Use `hashbrown::raw::RawTable` API (direct bucket access)
- `[ ]` Parallel build `par_extend` (chunked keys parallel insertion)
- `[ ]` SIMD hash for multi-column (SWAR hash for multi-key join)

### P27.3 — Quick Wins (3 SP)
- `[ ]` Use `SmallVec<[u32; 8]>` for `SelectionVector` (stack allocation)
- `[ ]` `Arc<[Value]>` constant pools (skip ref-counting overhead)
- `[ ]` Add `#[inline(always)]` to hot paths (`evaluate_binary`, `evaluate_aggregate`)

---

## 🔴 P28: Drop-in Replacement 1:1 — C++ Vela + LadybugDB
*Target: Read C++ DBs, load C++ extensions, provide identical CLI.*

### P28.1 — C++ Storage Reader (Read-Only) (10 SP)
- `[ ]` C++ page layout reader (page size, header format)
- `[ ]` C++ catalog deserialization (`catalog.h` format → Rust struct)
- `[ ]` C++ WAL reader (format parsing for crash recovery)
- `[ ]` C++ index reader (ART/HashIndex format compatibility)

### P28.2 — Extension ABI Compatibility (8 SP)
- `[ ]` C API boundary (`extern "C"` wrapper for extension entry points)
- `[ ]` Extension loader (load `.so`/`.dll` with symbol resolution)
- `[ ]` Fallback: Port remaining extensions natively (if ABI is too unstable)

### P28.3 — CLI Feature Parity (5 SP)
- `[ ]` Interactive history (`rustyline`/`reedline` integration)
- `[ ]` Multi-line query parsing
- `[ ]` `.import` / `.export` shell built-in commands
- `[ ]` Tab completion for table and function names
- `[ ]` Exact output formatting parity (Aligned table, CSV, JSON, box)

---

## 🟡 P29: Feature & Function Completeness
*Target: 100% API compatibility with Ladybug.*

### P29.1 — 18 Missing Unique Functions (6 SP)
- `[ ]` **Math**: `atan2`, `degrees`, `radians`, `sinh`, `cosh`, `tanh`, `asin`, `acos`, `atan`, `log2`, `gcd`, `lcm`, `factorial`, `sign`
- `[ ]` **String**: `levenshtein`, `soundex`, `encode/decode_base64`, `sha256`
- `[ ]` **List**: `list_contains_all`, `list_has_any`, `list_has_all`, `list_sort`
- `[ ]` **Map**: `map_from_entries`, `map_values`, `map_keys`
- `[ ]` **Blob**: `blob_from_bytes`, `to_base64`, `from_base64`
- `[ ]` **Net**: `pg_isready`

### P29.2 — Verification & Validation (Same as P26)
*(Execution testing handled in P26.2, P26.3, P26.4)*

---

## 📅 Execution Strategy

| Focus Area | Effort | Expected Outcome |
|------------|:------:|------------------|
| **Sprint 1** | 22 SP | Zero-copy Arrow (2-3x speedup), Edge cases |
| **Sprint 2** | 24 SP | C++ Storage Reader, Missing Functions |
| **Sprint 3** | 25 SP | ABI Compat, CLI Parity, Fuzz & Prop tests |
