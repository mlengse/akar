# Akar — Forward Implementation Plan

> **Revision:** 2026-08-07 (Sprint 17 **COMPLETE** — P49.1..P49.3 all DONE; gate hijau **1,354 passed / 0 failed**; bench BUG-A workaround dibersihkan, `pk_col` default footgun dihapus, benchmark di-re-run & docs disinkronkan; **P45.2 CANCELLED** — no crates.io publishing until production-ready)
> **Author:** Anjang Kusuma Netra | **License:** GPLv3
> **Baseline (sekarang):** `cargo test --workspace` → **1,354 passed, 0 failed, 5 ignored (doc-tests only)**, 32 crates, ~86K LOC (git-tracked). (Baseline historis di header lama = 1,310 + 1 fail `test_migration_ingestion` — sudah ditutup P48.5.)
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
| **P48** | **Correctness & Benchmark Unblock** | ✅ **DONE** | **~10** | ✅ **Sprint 16 — COMPLETE** (semua P48.1–P48.18 DONE, gate 1,354 hijau) |
| **P49** | **Post-Benchmark Hardening & Cleanup** | ✅ **DONE** | **~3** | ✅ **Sprint 17 — COMPLETE** (P49.1–P49.3 DONE — bench syntax bersih, `pk_col` guard, benchmark re-run) |

> [!IMPORTANT]
> **P1-P49 + AUDIT: ALL COMPLETE** — 1,354 tests passing, 3-way C++ parity verified, 100K/1M scalability measured, WAL append-only redesign (52× speedup), crash recovery stress-tested, release profiles optimized, radixsort OOB fixed, 5 perf optimizations landed, WCOJ planner-side + embedded server mode (Sprint 15), 18 item correctness/benchmark unblock (Sprint 16), dan 3 item post-benchmark hardening (Sprint 17: bench BUG-A workaround → node-predicate syntax, `pk_col` default footgun dihapus, benchmark re-run & docs sync).
> **P43.3: CANCELLED** — C++ per-operator benchmark source was removed from the repo by review decision (2026-07-31); not needed — SQL-level E2E 3-way parity already verified (~1×).
> **P45.2: CANCELLED** — tidak publish ke crates.io sebelum benar-benar siap production.
> **P48.18 (BUG-B):** equality 2-kolom (`a.id = b.id`) silent all-pass — root cause `JoinOptimization` fallback menghapus join-condition filter saat tak ada join nyata; FIXED dengan guard (plan tanpa join mempertahankan filter).
> **P48.17 (BUG-A):** node-predicate `(a:Person {id:X})` diabaikan saat dikombinasikan dengan WHERE eksplisit — planner kini AND-combine BoundWhere implisit + eksplisit; engine fix selesai.
> **P49.1:** workaround BUG-A di bench (variable-comparison `WHERE a.id >= {a} AND a.id <= {a}`) diganti ke syntax node-predicate `MATCH (a:Person {id: {a}}) ...` — hasil identik (star 10,000 / chain 10,000 / triangle 10,660 rows), validasi end-to-end P48.17 di jalur bench PASS.
> **P49.2:** default `pk_col = 0` dihapus (footgun laten) — `catalog.rs`/`table.rs` kini memakai sentinel `usize::MAX` (no-PK eksplisit); SQL tak berubah (binder tetap memaksa PK).
> **P49.3:** benchmark di-re-run — gate 1,354 hijau; setup fan DB 2.07 s / triangle 540 ms (debug, vs 1.45 s baseline — variasi timing, tak ada delta correctness).

---

## ✅ SPRINT 17 (COMPLETE): POST-BENCHMARK HARDENING & CLEANUP (P49)

> **Rekomendasi 2026-08-07:** Sprint 16 menutup semua lubang correctness yang mem-blocking benchmark. Sprint 17 = **bersihkan workaround BUG-A di bench** (sekaligus validasi end-to-end P48.17 di jalur bench) + **hapus footgun `pk_col` default** + **re-run benchmark & sinkronisasi angka docs**. Semua item punya bukti konkret; tak ada spekulasi.
> **Koreksi audit 2026-08-07:** item audit `LogicalMultiplicityReducer` dead code ternyata **SALAH** — tipe ini di-instantiate di `map_ddl.rs:404-405` (DISTINCT dedup); **bukan task**.

| Task | Description | Files | Status |
|------|-------------|-------|--------|
| P49.1 | **Bersihkan workaround BUG-A di bench → node-predicate syntax** — workaround variable-comparison `WHERE a.id >= {a} AND a.id <= {a}` untuk pin center (P48.4) diganti ke syntax node-predicate yang benar `MATCH (a:Person {id: {a}}), (b:Person) WHERE b.id > a.id AND b.id <= a.id + 10 CREATE (a)-[:r1]->(b)`. Validasi end-to-end P48.17 di jalur bench (bukan hanya unit test) PASS: star 10,000 / chain 10,000 / triangle 10,660 rows identik. Pola **range** (`a.id >= 0 AND a.id <= 99`) tetap WHERE — node-predicate hanya mendukung equality. | `akar-main/tests/test_wcoj_bench.rs`, `akar-main/benches/ladybug_suite.rs` | ✅ DONE |
| P49.2 | **Hilangkan default `pk_col = 0`** — `catalog.rs`/`table.rs`: default 0 adalah footgun laten (SQL path tak terjangkau — binder memaksa PK ada, `binder/mod.rs:861`), kini diganti sentinel eksplisit `usize::MAX` (no-PK) dengan guard terdokumentasi; `copyfrom.rs` dibuat defensif (`get`/`and_then`). Perilaku SQL tak berubah. | `akar-core/akar-catalog/src/lib.rs`, `akar-storage/src/table.rs`, `akar-processor/src/physical/write_ops/copyfrom.rs` | ✅ DONE |
| P49.3 | **Re-run benchmark suite & sinkronisasi docs** — `test_wcoj_bench.rs` + `cargo test --workspace` dijalankan ulang: gate **1,354 passed / 0 failed / 5 ignored** (tiada penurunan); star 10,000 / chain 10,000 / triangle 10,660 rows identik; setup fan DB 2.07 s / triangle 540 ms (debug, baseline 1.45 s — variasi timing, tak ada delta correctness). Angka benchmark WCOJ hanya ada di `implementation_plan.md` (bukan `BENCHMARK_COMPARISON.md`/`SPEC.md` — keduanya tidak memuat setup WCOJ), jadi tak ada dokumen lain yang perlu diubah. | `akar-main/tests/test_wcoj_bench.rs`, `akar-main/benches/ladybug_suite.rs`, docs | ✅ DONE |

**Urutan kerja:** P49.1 → P49.2 → P49.3 (bench cleanup dulu — sekali jalan dengan gate).
**Gate:** `cargo test --workspace` hijau = **1,354 passed / 0 failed / 5 ignored** (tiada penurunan dari P48.17; P49.1/2 tidak menambah test, P49.3 hanya sinkronisasi docs).

---

## ✅ SPRINT 16 (COMPLETE): CORRECTNESS & BENCHMARK UNBLOCK (P48)

> **Ringkasan hasil (detail lengkap di commit history & `git log`):**

| Task | Ringkasan | Status |
|------|-----------|--------|
| P48.1 | Fix multi-hop join cross-product (comma-pattern share-var di-scan ulang → `CrossProduct`); fix `available_vars` tracking di `planner.rs` → single-scan plan. | ✅ DONE |
| P48.2 | Fix rel-table `COPY` (CSV/Parquet panic) — `bind_copy_from` + `PhysicalCopyFrom` sintesis kolom src/dst + resolve PK→offset. | ✅ DONE |
| P48.3 | Predicate pushdown multi-scan — `build_join_tree` pecah AND-conjuncts, dorong single-var conjunct ke `scan.predicate` (714 ms → 56.8 ms, ~12.6×). | ✅ DONE |
| P48.4 | Re-open P46.5 benchmark — setup 1.45 s (dari 223 s), star 10,000 / chain 10,000 / triangle 10,660 rows, EXPLAIN `Intersect` PASS. | ✅ DONE |
| P48.5 | Tutup `test_migration_ingestion` — `akar-migrate` cek eksistensi table sebelum CREATE/COPY (idempotent). | ✅ DONE |
| P48.6 | README server-mode note — section "Concurrency Model". | ✅ DONE |
| P48.7 | Fix Date/Timestamp scan → NULL — `evaluate_constant_expr` di kedua write path + String→Date/Timestamp cast + scan-side arms. | ✅ DONE |
| P48.8 | Fix FTS+WHERE index-OOB panic — `filter_rows_by_mask` (indeks absolute row). | ✅ DONE |
| P48.9 | WHERE-predicate eval error → kini di-propagate (bukan silent all-pass). | ✅ DONE |
| P48.10 | `read_art_varint` OOB → `Result` + bounds/shift guards. | ✅ DONE |
| P48.11 | String truncation 255 → error eksplisit; physicalexplain Arrow rewrite (plan penuh). | ✅ DONE |
| P48.12 | UInt64 narrowing diselaraskan → UInt64 utuh di scan/hash/arithmetic + koersi insert. | ✅ DONE |
| P48.13 | ART PK-index gagal → fail-fast di kedua path CREATE NODE TABLE. | ✅ DONE |
| P48.14 | `COUNT(variable)` = 0 → hitung semua baris (≡ COUNT(*) utk node binding) di 3 path. | ✅ DONE |
| P48.15 | NaN ordering — `double_cmp` (NaN > finite, NaN = NaN) di value_cmp/compare_values/sort/Percentile + `values_equal` NaN-aware. | ✅ DONE |
| P48.16 | SIP semi-mask mati → **DIHAPUS** (4 bukti no-op; `NodeSemiMask` kernel dipertahankan). | ✅ DONE |
| P48.17 | (BUG-A) node-predicate + WHERE eksplisit kini AND-combine di `planner.rs` — tidak lagi saling timpa; 3 test regresi permanen. | ✅ DONE |
| P48.18 | (BUG-B) equality 2-kolom silent all-pass → guard fallback `JoinOptimization` (plan tanpa join mempertahankan filter); triangle N=41 → 10,660. | ✅ DONE |

**Gate Sprint 16:** `cargo test --workspace` = **1,354 passed / 0 failed / 5 ignored** (97 suite) — tak ada lagi test failure pre-existing (dulu skip `test_count_variable`).
**Temuan scan tambahan (audit 2026-08-02, VERIFIED):** dua diangkat jadi task (P48.15, P48.16); `LogicalMultiplicityReducer` **diverifikasi 2026-08-07 bukan dead code** (di-instantiate di `map_ddl.rs:404-405` utk DISTINCT) → bukan task; `pk_col` default 0 = low-priority cleanup → dipindah ke **P49.2**.

---

## 📅 Execution Strategy

| Sprint | Focus | SP | Key Deliverables |
|--------|-------|:---:|-----------------|
| Sprint 1-12 | P0-P42 + AUDIT | ~298 | ✅ ALL COMPLETE — see [`SPEC.md`](../SPEC.md) |
| **Sprint 13** | **P43 Bug Fixes + P44 Performance** | **11** | ✅ COMPLETE (P43.3 cancelled): radixsort fix, OCC row-level inserts, hash join optimization, Arrow native arrays, sort optimization, GROUP BY hasher, plan caching |
| **Sprint 14** | **P45 Production Readiness** | **5** | ✅ COMPLETE — catalog serialization, data durability, operator parity (crates.io publishing CANCELLED) |
| **Sprint 15** | **P46 WCOJ + P47 Multi-Process** | **8** | ✅ COMPLETE — planner-side WCOJ (Intersect), embedded server mode (akar-server + connect_tcp, 12+5 tests) |
| **Sprint 16** | **P48 Correctness & Benchmark Unblock** | **~10** | ✅ **COMPLETE 2026-08-07** — 18 item correctness/perf unblock (lihat tabel Sprint 16), gate 1,354 hijau tanpa skip |
| **Sprint 17** | **P49 Post-Benchmark Hardening & Cleanup** | **~3** | ✅ **COMPLETE 2026-08-07** — bench BUG-A workaround → node-predicate syntax, `pk_col` default guard dihapus, benchmark re-run (gate 1,354 hijau) + docs sync |

---

## Detail yang Sudah Diimplementasikan

Detail Sprint 12-15 (P41-P47), dependency graph, audit fixes summary (30/31, 1 N/A), dan design decisions log (#1-#68) dipindahkan ke [`docs/archive/implemented-context.md`](docs/archive/implemented-context.md). Detail Sprint 16 (P48) tersimpan di commit history (satu commit per P48: `2fe874c`=P48.14, `dc023bc`=P48.15, `5bb6e74`=P48.16, `b8b51db`=P48.17, `fbd1681`=cleanup naming). Detail Sprint 17 (P49) tersimpan di commit P49 (bench cleanup, `pk_col` guard, docs sync).
