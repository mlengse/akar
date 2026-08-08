# Akar — Forward Implementation Plan

> **Revision:** 2026-08-08 (Sprint 18 — P50.1–P50.4 **DONE**: **31/31 published**; **31/31 publishable** (hanya `akar-c` `publish = false`); audit keamanan & lisensi **CLEAN**; gate hijau **1,535 passed / 0 failed**; worktree committed d4b3035/deb9494/004b6c1/9af7995/4b099d1 & pushed; **audit markdown BERES (batch 1 & batch 2)** — semua doc sync ke state 2026-08-08; **sisa = P50.5** (repo GitHub public → tag `v0.1.0` + release notes) + verifikasi akhir; detail di tabel P50)
> **Author:** Anjang Kusuma Netra | **License:** GPLv3
> **Baseline (sekarang):** `cargo test --workspace` → **1,535 passed, 0 failed, 0 ignored**, 32 crates, ~86K LOC (git-tracked), diukur pada worktree saat ini (82 file uncommitted). (Baseline historis di header lama = 1,354 — selisih karena perubahan worktree pasca-P49.3: migrasi test `akar-processor` ke API post-Arrow & sinkronisasi `tests.rs`/`tests_only.rs` (142 test tiap file), plus perbaikan panic `col_indices` kosong di `aggregatehashtable.rs`; baseline lebih lama = 1,310 + 1 fail `test_migration_ingestion` — sudah ditutup P48.5.)
> **Performance verified (hot path):** Rust 397 µs for `MATCH ... WHERE age > 30 RETURN COUNT(p)` on 10k rows. See [`BENCHMARK_COMPARISON.md`](BENCHMARK_COMPARISON.md).
> **For completed phases (P1-P44) and LadybugDB functional parity:** see [`SPEC.md`](../SPEC.md)

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
| **P50** | **Release Readiness & Publishing** | 🔜 **HIGH** | **~5** | 🔜 Sprint 18 — **P50.1–P50.4 DONE (31/31 published)**; sisa **P50.5** (repo public → tag `v0.1.0` + release notes) |

> [!IMPORTANT]
> **P1-P49 + AUDIT: ALL COMPLETE** — 1,535 tests passing, 3-way C++ parity verified, 100K/1M scalability measured, WAL append-only redesign (52× speedup), crash recovery stress-tested, release profiles optimized, radixsort OOB fixed, 5 perf optimizations landed, WCOJ planner-side + embedded server mode (Sprint 15), 18 item correctness/benchmark unblock (Sprint 16), dan 3 item post-benchmark hardening (Sprint 17: bench BUG-A workaround → node-predicate syntax, `pk_col` default footgun dihapus, benchmark re-run & docs sync).
> **P43.3: CANCELLED** — C++ per-operator benchmark source was removed from the repo by review decision (2026-07-31); not needed — SQL-level E2E 3-way parity already verified (~1×).
> **P45.2: REOPENED (2026-08-07)** — publish ke crates.io diaktifkan kembali sebagai tujuan Sprint 18 (P50); dijalankan setelah audit release-readiness (metadata, license, package verify) selesai.
> **P48.18 (BUG-B):** equality 2-kolom (`a.id = b.id`) silent all-pass — root cause `JoinOptimization` fallback menghapus join-condition filter saat tak ada join nyata; FIXED dengan guard (plan tanpa join mempertahankan filter).
> **P48.17 (BUG-A):** node-predicate `(a:Person {id:X})` diabaikan saat dikombinasikan dengan WHERE eksplisit — planner kini AND-combine BoundWhere implisit + eksplisit; engine fix selesai.
> **P49.1:** workaround BUG-A di bench (variable-comparison `WHERE a.id >= {a} AND a.id <= {a}`) diganti ke syntax node-predicate `MATCH (a:Person {id: {a}}) ...` — hasil identik (star 10,000 / chain 10,000 / triangle 10,660 rows), validasi end-to-end P48.17 di jalur bench PASS.
> **P49.2:** default `pk_col = 0` dihapus (footgun laten) — `catalog.rs`/`table.rs` kini memakai sentinel `usize::MAX` (no-PK eksplisit); SQL tak berubah (binder tetap memaksa PK).
> **P49.3:** benchmark di-re-run — gate 1,354 hijau; setup fan DB 2.07 s / triangle 540 ms (debug, vs 1.45 s baseline — variasi timing, tak ada delta correctness).
> **P45.2: REOPENED via P50** — publishing ke crates.io tidak lagi dicancel; menjadi tujuan utama Sprint 18 (P50) setelah codebase stabil & production-ready.

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

## 🔜 SPRINT 18: RECOMMENDED — RELEASE READINESS & PUBLISHING (P50)

> **Rekomendasi 2026-08-07:** Semua phase plan (P1-P49 + AUDIT) sudah COMPLETE dan codebase stabil (gate 1,535 hijau, tak ada test failure pre-existing). **P45.2 (publish ke crates.io) di-REOPEN** sebagai tujuan Sprint 18 — satu-satunya item yang sengaja ditunda menunggu production-readiness. Sprint 18 = audit release-readiness → publish crates → tag v0.1.0 + release notes. Semua item punya bukti konkret; tak ada spekulasi.
> **Catatan scope:** workspace punya **31/31 crate publishable** (hanya `akar-c` yang `publish = false` — FFI cdylib, build lokal saja). Semula 14 crate di-set `publish = false` (kebijakan, bukan blocker teknis): `akar-cli`/`akar-migrate` (binary-only), dan extension/connector `akar-json`, `akar-httpfs`, `akar-azure`, `akar-duckdb`, `akar-delta`, `akar-iceberg`, `akar-sqlite`, `akar-postgres`, `akar-neo4j`, `akar-llm`, `akar-unity-catalog`, plus `akar-graph` (hard-dep `akar-algo`) — **semua di-flip ke `publish = true` 2026-08-07/08** karena dep internal sudah terpublish dan tidak ada blocker teknis (licensing & keamanan sudah diaudit). `fuzz` (internal). Publish **bottom-up** mengikuti dependency graph.

| Task | Description | Files | Status |
|------|-------------|-------|--------|
| P50.1 | **Audit metadata & license per crate** — pastikan tiap `Cargo.toml` punya `description`, `repository` (mlengse/akar), `license = "GPL-3.0-or-later"`, `keywords`, `categories`; LICENSE header di tiap crate dir; tak ada crate yang mem-publish path/rahasia lokal. | semua `akar-core/akar-*/Cargo.toml`, LICENSE | ✅ DONE — 32/32 crate inherit `[workspace.package]` (version 0.1.0, edition 2024, GPL-3.0-or-later, repository mlengse/akar, description, keywords, categories); LICENSE root GPL-3.0 disalin ke 32 crate dir → kini ikut ter-package (terverifikasi di package akar-common); hanya `akar-c` yang `publish = false`; `akar-common` dry-run publish PASS |
| P50.2 | **Version baseline & semver** — seluruh crate versi `0.1.0` yang konsisten (workspace `[workspace.package]`); verifikasi dependensi internal pakai path-versi yang benar (dipublish sebagai versi crates.io, bukan path). | `akar-core/Cargo.toml`, semua crate Cargo.toml | ✅ DONE — semua crate `version = "0.1.0"` via `[workspace.package]`; **120 path-dependensi internal kini punya `version = "0.1.0"`** (sebelumnya path-only = gagal publish); terverifikasi: `cargo package --list` 32/32 crate OK, `cargo publish --dry-run` untuk leaf crate (`akar-common`, `akar-parser`) PASS & error resolve `akar-common` di crate dependen = bukti normalize path→registry version bekerja; `cargo check --workspace` hijau |
| P50.3 | **`cargo publish --dry-run`** — validasi tiap crate bisa dikemas & di-compile dari packaged source (bukan dari workspace checkout); perbaiki packaging error sampai bersih; pastikan `akar-main` tidak mengekspos benchmark/test-only deps ke publik. | semua crate, CI script publish | ✅ DONE — dry-run terverifikasi via publish nyata: tiap publish mengemas + meng-compile packaged source vs deps registry yang sudah ter-publish (`akar-vector` download `akar-extension` 0.1.0; `akar-storage` download transaction/vector; `akar-planner` download binder; `akar-optimizer` download planner/storage; `akar-main`/`akar-server`/`akar-wasm`/`akar-cli`/`akar-migrate` download deps dari registry — verify PASS semua 31/31). `cargo package --list` 32/32 OK, LICENSE ikut ter-package, tak ada file rahasia/path lokal. `akar-main` publish jalan dengan default features (tanpa `--all-features`), optional deps semua ter-publish lebih dulu |
| P50.4 | **Publish ke crates.io (bottom-up)** — publish berurutan leaf→root (`akar-common` dulu, `akar-main` terakhir) dengan verifikasi tiap crate ter-resolve setelah dipublish; jadikan `akar-main` sebagai entry point utama. | crates.io, git tag `v0.1.0` | ✅ **DONE — 31/31 published** (2026-08-08, terverifikasi di crates.io, semua `0.1.0`): `akar-common`, `akar-parser`, `akar-transaction`, `akar-catalog`, `akar-function`, `akar-binder`, `akar-extension`, `akar-vector`, `akar-storage`, `akar-planner`, `akar-fts`, `akar-optimizer`, `akar-processor`, `akar-graph`, `akar-algo`, `akar-json`, `akar-httpfs`, `akar-duckdb`, `akar-sqlite`, `akar-postgres`, `akar-neo4j`, `akar-llm`, `akar-delta`, `akar-iceberg`, `akar-azure`, `akar-unity-catalog`, `akar-main`, `akar-server`, `akar-wasm`, `akar-cli`, `akar-migrate`. Tiap publish meng-compile packaged source vs deps yang sudah ter-publish di registry (bukti normalize path→registry). **Rate-limit crates.io (docs resmi):** new crate = burst 5 lalu **1 crate / 10 menit** (new version = burst 30 lalu 1/min); error 429 memberi waktu retry eksplisit ("try again after ... GMT") — waktu itu **otoritatif**, bukan estimasi. **Pacing via checkpoint (lihat tabel jadwal di bawah):** setiap kali waktu sistem melewati checkpoint, publish 1 crate; sisa waktu di antara dipakai untuk pekerjaan lain (mis. audit markdown). **Konflik `akar-algo` → `akar-graph`:** `akar-algo` hard-dep `akar-graph` yang tadinya `publish = false` → di-flip ke `publish = true` 2026-08-07 (dep graph = common/storage, sudah publish). **Sisa publish (urutan):** ✅ KOSONG — semua 31 crate sudah ter-publish. Rate-limit crates.io (docs resmi): new crate = burst 5 lalu **1 crate / 10 menit**; sesi ini memakai 2 burst (12:28Z & 15:08Z) + window 10-menit untuk `akar-cli` (15:13:41Z) & `akar-migrate` (15:23:49Z). **Audit keamanan CLEAN:** full history `rev-list --all` tanpa secret high-entropy (token AWS/GitHub/private key/PGP); semua referensi kredensial = env-var reads (`OPENAI_API_KEY`, `AZURE_STORAGE_SAS_TOKEN`, `CODECOV_TOKEN` via `secrets.*`); tanpa `.env`/`.pem`/`.key`/`credentials.toml` ter-track; komentar path lokal di `.gitignore:46` dihapus. **Audit lisensi CLEAN:** 32/32 crate GPL-3.0-or-later; 282 dep crates.io semua punya field license; satu-satunya flag GPL-compatible (`option-ext` MPL-2.0 via `dirs`→`akar-cli`; `r-efi` MIT/Apache/LGPL-2.1). **Repo GitHub `mlengse/akar` masih PRIVATE** — set public sebelum tag v0.1.0/Release (docs.rs/repo link 404 untuk publik; publish crates.io tidak membutuhkan repo public). **Aman (no version skew):** diff worktree ke crate yang sudah ter-publish murni rustfmt/formatting — tanpa perubahan semantik, jadi 0.1.0 yang beredar tetap valid |

**Jadwal checkpoint upload (P50.4) — 31/31 ter-upload (log, 1 crate per 10 menit max):**

> Waktu di bawah adalah **target estimasi** (UTC). Setiap kali waktu sistem (UTC) melewati checkpoint, publish **1 crate** berikutnya; pembulatan ke window berikutnya jika build masih berjalan. Waktu retry eksplisit dari pesan 429 **mengalahkan** estimasi ini. Update baris status `✅` setelah tiap crate ter-upload.

| # | Crate | Checkpoint (UTC) | Waktu lokal (UTC+7) | Status |
|---|-------|------------------|---------------------|--------|
| 16 | `akar-json` | 17:35 | 00:35 | ✅ published 17:33:37Z |
| 17 | `akar-httpfs` | 17:45 | 00:45 | ✅ published 17:48:53Z |
| 18 | `akar-duckdb` | 17:55 | 00:55 | ✅ published 17:55Z |
| 19 | `akar-sqlite` | 12:28 | 19:28 | ✅ published 12:28:11Z |
| 20 | `akar-postgres` | 12:28 | 19:28 | ✅ published 12:28:27Z |
| 21 | `akar-neo4j` | 12:28 | 19:28 | ✅ published 12:28:37Z |
| 22 | `akar-llm` | 12:28 | 19:28 | ✅ published 12:28:47Z |
| 23 | `akar-delta` | 12:28 | 19:28 | ✅ published 12:28:58Z |
| 24 | `akar-iceberg` | 12:33 | 19:33 | ✅ published 12:33:51Z |
| 25 | `akar-azure` | 15:08 | 22:08 | ✅ published 15:08:18Z |
| 26 | `akar-unity-catalog` | 15:09 | 22:09 | ✅ published 15:09:25Z |
| 27 | `akar-main` | 15:09 | 22:09 | ✅ published 15:09Z (verify compile vs deps registry) |
| 28 | `akar-server` | 15:10 | 22:10 | ✅ published 15:10:29Z |
| 29 | `akar-wasm` | 15:10 | 22:10 | ✅ published 15:10:56Z |
| 30 | `akar-cli` | 15:13 | 22:13 | ✅ published 15:13:41Z |
| 31 | `akar-migrate` | 15:23 | 22:23 | ✅ published 15:23:49Z |
| P50.5 | **Tag rilis + release notes** — tag `v0.1.0`, tulis release notes (ringkasan P0-P50: 3-way parity, WAL 52×, crash recovery, WCOJ, embedded server, 1,535 tests) di GitHub Releases. | GitHub Releases, git tag | 🔜 PLANNED |

**Urutan kerja:** P50.1 → P50.2 → P50.3 → P50.4 → P50.5 (audit & verifikasi dulu sebelum publish nyata).
**Gate:** `cargo test --workspace` tetap hijau (1,535 passed / 0 failed) setelah setiap publish; `cargo publish --dry-run` bersih untuk semua 31 crate; tag v0.1.0 terverifikasi build dari source yang dipublish.
**Prereq publish berikutnya (2026-08-08):** commit worktree (DONE: d4b3035, deb9494, 004b6c1, 9af7995 — pushed ke origin/main); **audit markdown batch 2 BERES** — 11 doc diperbaiki (API contoh `Arc::new` di `akar-core/README.md`/`SPEC.md`, test-count basi di 7 crate README + tabel SPEC, klaim publish basi di `SPEC.md`/`RELEASE.md`, `implementation_plan.md` 1,354→1,535) — belum commit (bersama update plan ini); lanjut publish `akar-azure` pada window **12:43:24Z**, lalu satu-per-10-menit per tabel checkpoint; repo GitHub di-set public oleh user sebelum tag v0.1.0.

---

## 🗒 NEXT ACTIONS — Sesi Berikutnya (2026-08-08, Sprint 18/P50)

> **Status sesi ini:** publish **P50.4 COMPLETE — 31/31 published** (akhir: `akar-migrate` ✅ 15:23:49Z); audit markdown **batch 2 BERES** — 11 doc diperbaiki (API contoh `Arc::new`, test-count basi, klaim publish basi) + plan sync, committed `4b099d1` & pushed; verifikasi crates.io 31/31 `0.1.0` PASS. Sisa: **P50.5** (repo public → tag + release) + verifikasi akhir.

**Lanjutkan dari sini (urut):**

1. **P50.5 — tag rilis + release notes:** minta user set repo GitHub **public**, lalu `git tag v0.1.0` + `git push origin v0.1.0`; buat GitHub Release dengan notes: **3-way C++ parity (~1×), WAL append-only 52× speedup, crash recovery stress-tested, WCOJ planner-side (Intersect), embedded server mode (multi-process TCP), 1,535 tests passing, 31 crates di crates.io, 0.1.0**; attach CLI binaries (build via `cargo build --release -p akar-cli` + release workflow).
2. **Verifikasi akhir:** `cargo test --workspace` hijau (1,535 passed / 0 failed); `cargo add akar-main` di project kosong berhasil build; README Quick Start contoh (pakai `Arc::new(Database::new(.., SystemConfig::default()))` + `Connection::new(&db)`) ter-compile.
3. **Update plan ke Sprint 18 COMPLETE** setelah P50.5 + verifikasi tuntas; sinkronkan angka `31/31` di RELEASE.md/SPEC jika belum.

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
| **Sprint 14** | **P45 Production Readiness** | **5** | ✅ COMPLETE — catalog serialization, data durability, operator parity (crates.io publishing deferred → dituntaskan Sprint 18/P50) |
| **Sprint 15** | **P46 WCOJ + P47 Multi-Process** | **8** | ✅ COMPLETE — planner-side WCOJ (Intersect), embedded server mode (akar-server + connect_tcp, 12+5 tests) |
| **Sprint 16** | **P48 Correctness & Benchmark Unblock** | **~10** | ✅ **COMPLETE 2026-08-07** — 18 item correctness/perf unblock (lihat tabel Sprint 16), gate 1,354 hijau tanpa skip |
| **Sprint 17** | **P49 Post-Benchmark Hardening & Cleanup** | **~3** | ✅ **COMPLETE 2026-08-07** — bench BUG-A workaround → node-predicate syntax, `pk_col` default guard dihapus, benchmark re-run (gate 1,354 hijau) + docs sync |
| **Sprint 18** | **P50 Release Readiness & Publishing** | **~5** | 🔜 IN PROGRESS — **P50.1–P50.4 COMPLETE: 31/31 published** (bottom-up, rate-limit pacing), 31/31 publishable, audit keamanan & lisensi CLEAN, audit markdown batch 1 (`9af7995`) + batch 2 (`4b099d1`) BERES (API contoh, test-count, klaim publish basi fixed); mojibake `description` fixed di akar-main/server/wasm Cargo.toml; sisa **P50.5** — repo GitHub private → public (user), tag v0.1.0 + release notes + verifikasi akhir |

---

## Detail yang Sudah Diimplementasikan

Detail Sprint 12-15 (P41-P47), dependency graph, audit fixes summary (30/31, 1 N/A), dan design decisions log (#1-#68) dipindahkan ke arsip historis di luar repo (`docs/archive/implemented-context.md`). Detail Sprint 16 (P48) tersimpan di commit history (satu commit per P48: `2fe874c`=P48.14, `dc023bc`=P48.15, `5bb6e74`=P48.16, `b8b51db`=P48.17, `fbd1681`=cleanup naming). Detail Sprint 17 (P49) tersimpan di commit P49 (bench cleanup, `pk_col` guard, docs sync). Detail Sprint 18 (P50) akan tersimpan di commit rilis (audit publish-readiness, publish bottom-up ke crates.io, tag `v0.1.0` + release notes).
