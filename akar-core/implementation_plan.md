# Akar — Forward Implementation Plan

> **Revision:** 2026-08-02 (Sprint 15 COMPLETE — **P46 WCOJ + P47 Server Mode DONE**; **P45.2 CANCELLED** — no crates.io publishing until production-ready; **Sprint 16 PLANNED — P48 correctness fixes**, see §Sprint 16)
> **Author:** Anjang Kusuma Netra | **License:** GPLv3
> **Baseline:** `cargo test --workspace` → **1,310 passed, 1 failed (pre-existing `test_migration_ingestion`), 5 ignored (doc-tests only)**, 32 crates, ~86K LOC (git-tracked).
> **Performance verified (hot path):** Rust 397 µs for `MATCH ... WHERE age > 30 RETURN COUNT(p)` on 10k rows. See [`BENCHMARK_COMPARISON.md`](BENCHMARK_COMPARISON.md).
> **For completed phases (P1-P44) and LadybugDB functional parity:** see [`SPEC.md`](../SPEC.md) and [`docs/archive/implemented-context.md`](docs/archive/implemented-context.md)

---

## 🎯 Roadmap Overview

| Phase | Content | Priority | SP | Status |
|-------|---------|----------|:---:|--------|
| **P0-P25** | Foundation (parser, planner, processor, storage, GDS, extensions) | ✅ DONE | ~115 | ✅ Complete |
| **P26** | Testing, fuzzing & profiling | ✅ DONE | 17 | ✅ Complete |
| **P27** | Performance — profiling-driven optimization | ✅ DONE | 14 | ✅ Complete (C++ parity) |
| **P28** | Drop-in replacement — migration tool, CLI | ✅ DONE | 12 | ✅ Complete |
| **P29** | Functions & completeness | ✅ DONE | 6 | ✅ Complete |
| **P30** | Stabilisasi & Benchmark Komprehensif | ✅ DONE | 18 | ✅ Complete |
| **P31** | Final Parity Sprint | ✅ DONE | 4 | ✅ Complete |
| **P32** | Polish & DX | ✅ DONE | 2 | ✅ Complete |
| **P33** | Deferred Items | ✅ DONE | 4 | ✅ Complete |
| **P34** | Extension Depth — Native Readers | ✅ DONE | 13 | ✅ Complete |
| **P35** | Remaining Minor Gaps | ✅ DONE | 1 | ✅ Complete |
| **P36** | Critical Pipeline Gaps | ✅ DONE | 29 | ✅ Complete |
| **P37** | Storage & Performance | ✅ DONE | 18 | ✅ Complete |
| **P38** | DDL Completeness & Documentation | ✅ DONE | 11 | ✅ Complete |
| **P39** | Arrow Aggregate Fast Path | ✅ DONE | 2 | ✅ Complete |
| **P40** | Vectorized GROUP BY | ✅ DONE | 2 | ✅ Complete |
| **AUDIT** | **Codebase Audit Fixes (30/31 issues — 1 N/A)** | ✅ **DONE** | **—** | ✅ **30 issues resolved, 1 N/A (RwLock)** |
| **P41** | **Stress Testing — Crash Recovery** | ✅ **DONE** | **12** | ✅ Complete |
| **P42** | **Full Release Benchmarks** | ✅ **DONE** | **8** | ✅ Complete |
| **P43** | **Bug Fixes & Known Issues** | ✅ **DONE** | **3** | ✅ Complete (P43.3 CANCELLED) |
| **P44** | **Performance Optimization** | ✅ **DONE** | **8** | ✅ Complete |
| **P45** | **Production Readiness** | ✅ **DONE** | **8** | ✅ Complete (P45.2 CANCELLED) |
| **P46** | **Worst-Case Optimal Joins (WCOJ)** | ✅ **DONE** | **4** | ✅ Sprint 15 |
| **P47** | **Multi-Process Access (Embedded Server Mode)** | ✅ **DONE** | **4** | ✅ Sprint 15 |

> [!IMPORTANT]
> **P1-P44 + AUDIT: ALL COMPLETE** — 1,310 tests passing, 3-way C++ parity verified, 100K/1M scalability measured, WAL append-only redesign (52× speedup), crash recovery stress-tested, release profiles optimized, radixsort OOB fixed, 5 perf optimizations landed.
> **P43.3: CANCELLED** — C++ per-operator benchmark source was removed from the repo by review decision (2026-07-31); not needed — SQL-level E2E 3-way parity already verified (~1×).
> **P45: COMPLETE (Sprint 14)** — P45.1 catalog serialization DONE (DDL + cross-process recovery); P45.4 data durability DONE (durable column mirrors, crash recovery, read-only enforcement, cross-process locking — 8 new integration tests); P45.3 operator parity DONE (100% type parity, 58 C++ → 49 Rust fused, see archive §1); **P45.2 crates.io publishing CANCELLED** — tidak publish ke crates.io sebelum benar-benar siap production.
> **P46: COMPLETE (Sprint 15)** — planner-side WCOJ enumeration DONE: `build_wcoj_intersect` emits `LogicalIntersect` for star/fan-out patterns (shared probe node, per-edge build sides) and triangle patterns (star Intersect + closure Extend/Filter); fallback to HashJoin chain otherwise. Binder now allows reusing the same node variable in MATCH when it refers to the same node table. `PhysicalIntersect::execute_sides` emits the full cross-product of matching build rows with proper `field_names`. 5 new integration tests (`akar-main/tests/test_wcoj.rs`) + 4 planner unit tests + 2 processor unit tests; all pass.
> **P47: COMPLETE (Sprint 15)** — embedded server mode DONE: crate `akar-server` (TCP + length-prefixed JSON framing, `Server::bind`/`start`/`shutdown`, non-blocking accept loop), session bridging via `TransactionManager` (satu `Connection` per client), client helper `Database::connect_tcp` di `akar-main/src/remote.rs` (client tidak pernah membuka file lock — server yang memegangnya), exclusive-lock integration. 12 integration tests (`akar-server/tests/server_tests.rs`: concurrent write+read, crash client, DDL visibility, read-only enforcement, embedded unchanged) + 5 frame/response unit tests (`remote.rs`); all pass. True shared-storage multi-process writers di-design-out (butuh distributed buffer-pool protocol, tak cocok untuk embedded).

---

## 🔜 SPRINT 16: RECOMMENDED — CORRECTNESS & BENCHMARK UNBLOCK (P48)

> **Rekomendasi 2026-08-02** (hasil investigasi P46.5): 3 bug nyata ditemukan saat mencoba menjalankan benchmark. Prioritas: **correctness dulu**, pushdown adalah prasyarat untuk re-open P46.5. Semua sudah punya repro.
>
> **Ringkasan eksekutif:** Sprint 15 menutup fase fitur utama (P0–P47). Sebelum sprint baru yang besar, tutup 3 lubang correctness + 1 gap perf yang mem-blocking benchmark + 1 test failure pre-existing.

| Task | Description | Files | Status |
|------|-------------|-------|--------|
| P48.1 | **Fix same-table multi-hop join cross-product** — `MATCH (a {id:0})-[:r1]->(b)-[:r3]->(c)` mengembalikan **110 rows, harusnya 10**: node `b` dari hop pertama di-join silang ke seluruh node di hop kedua (cross product), bukan hanya pasangan edge yang valid. **Pre-existing** (terbukti di HEAD d0450ba, bukan regresi P46); star/cycle justru sudah benar setelah P46. Repro probe tersedia. | `akar-planner/src/join_order.rs`, `akar-processor/src/physical/join_ops.rs` | 🔜 PLANNED |
| P48.2 | **Fix rel-table `COPY`** — `COPY r1 FROM …` gagal `"Column count mismatch: expected 0 columns, got 2"` karena path COPY melihat rel table punya 0 catalog columns. Memutus ability untuk load edge data massal (juga blocker setup benchmark). | `akar-catalog/src/lib.rs`, `akar-main/src/database.rs` | 🔜 PLANNED |
| P48.3 | **Predicate pushdown** — dorong filter WHERE komparasi (mis. `b.id > 0 AND b.id <= 100`) ke scan node/rel agar tidak membangun cross product penuh. Tanpa ini: 1 query bulk CREATE di 10k node = 10k×10k → **794 s**. Setelah ini, P46.5 bisa di-reopen. | `akar-optimizer/src/passes/` (FilterPushDown), `akar-processor/src/physical/` (scan filters) | 🔜 PLANNED |
| P48.4 | **Re-open P46.5 benchmark (WCOJ vs HashJoin)** — setelah P48.3, pakai desain kecil yang sudah divalidasi: fan DB (Person 151 / Tag 101, bulk WHERE-comparison CREATE, setup ≈ 4 s; star 10k rows + `Intersect`, chain via rel `r3t` 10k rows tanpa `Intersect`) dan triangle DB (N=41, 6 bulk single-edge CREATE `WHERE c.id > b.id AND b.id > a.id`, setup ≈ 8 s; expected `C(41,3)=10,660`). Assert lama salah (mengharapkan 10k rows dari setup cross product 10k×10k) — sudah dikoreksi ke desain kecil. Jalankan via `--test` mode (criterion hang di mesin ini, Decision #62). | `akar-main/benches/ladybug_suite.rs` | 🔜 PLANNED |
| P48.5 | **Tutup `test_migration_ingestion`** — pre-existing failure di `akar-migrate` (`"Table 'User' already exists"`); selidiki apakah proses migrasi tidak idempotent (double-run terhadap DB yang sudah dimigrasi). | `akar-migrate/` | 🔜 PLANNED |
| P48.6 | **README server-mode note** — acceptance criteria P47 yang belum tuntas: dokumentasikan "multi-writer" = multi-thread in-process + multi-process via optional `akar-server`. | `README.md` | 🔜 PLANNED |

**Urutan kerja:** P48.1 → P48.2 → P48.3 → P48.4 → P48.5 → P48.6.
**Gate:** `cargo test --workspace` hijau (kecuali pre-existing yang sedang ditutup).

---

## 📅 Execution Strategy

| Sprint | Focus | SP | Key Deliverables |
|--------|-------|:---:|-----------------|
| Sprint 1-12 | P0-P42 + AUDIT | ~298 | ✅ ALL COMPLETE — see [`SPEC.md`](../SPEC.md) |
| **Sprint 13** | **P43 Bug Fixes + P44 Performance** | **11** | ✅ COMPLETE (P43.3 cancelled — C++ source removed by design): radixsort fix, OCC row-level inserts, hash join optimization, Arrow native arrays, sort optimization, GROUP BY hasher, plan caching |
| **Sprint 14** | **P45 Production Readiness** | **5** | Catalog serialization, data durability, operator parity analysis (crates.io publishing CANCELLED — belum siap production) |
| **Sprint 15** | **P46 WCOJ + P47 Multi-Process** | **8** | ✅ COMPLETE — P46 planner-side WCOJ DONE (Intersect emission); P47 embedded server mode DONE (akar-server + connect_tcp, 12 integration + 5 unit tests) |
| **Sprint 16** | **P48 Correctness & Benchmark Unblock** | **~8** | 🔜 PLANNED — fix same-table multi-hop join, rel-COPY, predicate pushdown; re-open P46.5; close `test_migration_ingestion`; README note |

---

## Detail yang Sudah Diimplementasikan

Detail Sprint 12-15 (P41-P47), dependency graph, audit fixes summary (30/31, 1 N/A), dan design decisions log (#1-#68) dipindahkan ke [`docs/archive/implemented-context.md`](docs/archive/implemented-context.md).
