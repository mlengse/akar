# Akar — Forward Implementation Plan

> **Revision:** 2026-08-03 (Sprint 16 IN PROGRESS — **P48.1, P48.2, P48.3, P48.5 DONE**; next: P48.4 benchmark re-open; **P45.2 CANCELLED** — no crates.io publishing until production-ready)
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
> **2026-08-02 (audit):** P48.1, P48.2, P48.5 **terkonfirmasi nyata dgn repro** di `akar-main/tests/repro_multihop.rs` (untracked). Temuan audit memperbaiki deskripsi asli: P48.1 sebenarnya di bentuk **comma-pattern** (single-path chain sudah benar); P48.2 root cause = `bind_copy_from` pakai `entry.columns()` yg utk rel table hanya properti user (src/dst terpisah); P48.5 = `CREATE NODE TABLE` di migrate main.rs non-idempotent. **Scan tambahan (2026-08-02):** ditemukan **P48.7 Date/Timestamp scan → NULL** (terkonfirmasi repro) dan **P48.8 FTS+WHERE index-OOB panic** (terkonfirmasi dari kode), plus 6 kandidat medium/low.
>
> **2026-08-03 (P48.7):** root cause sebenarnya P48.7 **bukan** murni scan-side — CREATE DML write path hanya memproses `Expression::Constant`, sehingga `DATE('2024-01-15')` tersimpan sebagai `Value::Null`. Diperbaiki di kedua write path (inline ddl.rs + pipeline map_ddl.rs) via helper `evaluate_constant_expr`, plus String→Date/Timestamp cast + scan-side arms. **P48.3 dan P48.5 juga FIXED 2026-08-03** (lihat baris masing-masing).
>
> **Ringkasan eksekutif:** Sprint 15 menutup fase fitur utama (P0–P47). Sebelum sprint baru yang besar, tutup 3 lubang correctness + 1 gap perf yang mem-blocking benchmark + 1 test failure pre-existing.

| Task | Description | Files | Status |
|------|-------------|-------|--------|
| P48.1 | **Fix multi-hop join cross-product** — bentuk comma-separated `MATCH (a {id:0})-[:r1]->(b:Person), (b:Person)-[:r3]->(c:Person)` mengembalikan **27 rows, harusnya 3** (cross product 9 b × 3 c; tiap path valid diulang 9×). **Audit 2026-08-02:** bentuk single-path chain `(a)-[:r1]->(b)-[:r3]->(c)` **sudah benar** (3 rows valid) — deskripsi asli ("110 vs 10") keliru menunjuk bentuk chain; bug ada di jalur multi-pattern/share-var. **Pre-existing** (bukan regresi P46). **FIXED 2026-08-02:** root cause = `parse_patterns` (`dml.rs:172-214`) memecah comma-pattern jadi 4 `Pattern` (`{a,r1}`, `{b,None}`, `{b,r3}`, `{c,None}`); pattern referensi `{b,None}` me-reset `skip_next_node` → pattern `{b,r3}` menscan ulang `b` → `CrossProduct(ScanNode, ScanNode)`. Fix di `planner.rs`: tracking `available_vars: HashSet<String>` — variabel node yang sudah terikat (hasil scan/Extend dst) tidak di-scan ulang; dst var didaftarkan setelah setiap Extend. Plan kini single-scan: `ScanNode(Person) → Extend(r1) → Extend(r3)`. Test permanen: `test_comma_pattern_chain_shared_var` + `test_comma_pattern_chain_no_cross_product` (test_wcoj.rs), semua PASS. Repro: `test_chain_comma_patterns`. | `akar-planner/src/planner.rs` | ✅ DONE |
| P48.2 | **Fix rel-table `COPY`** — `COPY r1 FROM …` gagal. **Audit 2026-08-02 (root cause):** `bind_copy_from` (`akar-binder/src/binder/mod.rs:1701`) memakai `entry.columns()` yang utk rel table **hanya properti user** — src/dst disimpan terpisah (`RelTableEntry.src_table_id`/`dst_table_id`) sehingga tidak masuk schema COPY. CSV: `Column count mismatch at line 1: expected 0 columns, got 2` (rel tanpa properti) / `expected 1 columns, got 3` (1 properti). Parquet: **panic** `row[0]` (copyfrom.rs:100) karena `col_indices` kosong → row kosong. Memutus ability load edge massal (blocker setup benchmark). **FIXED 2026-08-02:** (a) `bind_copy_from` kini meneruskan `is_rel_table` dari `entry.is_rel_table()` dan memvalidasi header CSV utk rel = `columns.len() + 2` (file `[SRC, DST, …props]`); (b) `PhysicalCopyFrom::execute` (`copyfrom.rs`) utk rel table mensintesis 2 kolom leading `from`/`to` bertipe PK type node src/dst (mirror C++ `bindExpectedRelColumns` → `IndexLookupInfo`), sehingga reader memvalidasi `columns.len() + 2` dan tidak lagi menghasilkan row kosong (parquet tidak panic); (c) branch insert rel me-resolve `from`/`to` (PK value) → internal offset via `lookup_by_pk` pada node table src/dst. Test permanen: `test_rel_copy_csv` + `test_rel_copy_csv_has_props` (repro_multihop.rs), keduanya **PASS** (sebelumnya FAIL). | `akar-binder/src/binder/mod.rs`, `akar-catalog/src/lib.rs`, `akar-processor/src/physical/write_ops/copyfrom.rs` | ✅ DONE |
| P48.3 | **Predicate pushdown (multi-scan)** — dorong filter WHERE komparasi (mis. `b.id > 0 AND b.id <= 100`) ke scan node/rel agar tidak membangun cross product penuh. Tanpa ini: 1 query bulk CREATE di 10k node = 10k×10k → **794 s**. **Audit 2026-08-02 (terkonfirmasi):** pass `FilterPushDown` + `PredicatePushDown` **sudah ada** (`akar-optimizer/src/passes/flat/`) tapi keduanya **hanya** menggabungkan Filter yang **berdekatan** dgn `ScanNode` dalam pipeline flat (`filter_pushdown.rs:60-79` mem-fold saat `current_scan` masih aktif; `predicate_pushdown.rs:26-37` hanya utk Filter tepat-dibawah-scan). Untuk query multi-scan, planner menaruh WHERE di **atas** seluruh join tree (`planner.rs:699-706`), dan `build_join_tree` (`join_order.rs:31-85`) memakai `filter_expr` **hanya** untuk mengekstrak join-conditions (cross→hash join), **bukan** mendorong predikat non-join ke scan → cross product penuh dimaterialisasi sebelum filter. **Empiris (repro `test_pred_pushdown_multi_scan_timing`, 1500 node):** single-scan+WHERE = 2.3 ms vs multi-scan+WHERE = **714 ms (305×)**. Single-scan case SUDAH bekerja (filter di-fold ke `scan.predicate`, dievaluasi `scan_filter/scan.rs`). **FIXED 2026-08-03:** `build_join_tree` (`join_order.rs`) kini memecah filter jadi top-level AND-conjuncts (`split_and_conjuncts`), lalu utk tiap `ScanNode` mendorong conjunct yang mereferensikan **tepat satu variabel** yang cocok dengan `scan.alias` ke `scan.predicate` (`push_single_var_predicates`, AND-combined dgn predicate existing). Conjunct multi-variabel (join-condition seperti `a.id = b.id`) atau yang variabelnya tak punya scan ditinggalkan utk Filter top-level (correctness aman). Berlaku utk single-scan dan multi-scan. **Empiris setelah fix:** multi-scan+WHERE = **56.8 ms** (dari 714 ms, ~12.6× lebih cepat; ratio vs single-scan 305× → 21×). Correctness terjaga (count 151,500 = 1500 a × 101 b tetap benar). Unit test: `test_single_var_predicate_pushdown` (join_order.rs). Setelah ini, P46.5 bisa di-reopen. | `akar-planner/src/join_order.rs` | ✅ DONE |
| P48.4 | **Re-open P46.5 benchmark (WCOJ vs HashJoin)** — setelah P48.3, pakai desain kecil yang sudah divalidasi: fan DB (Person 151 / Tag 101, bulk WHERE-comparison CREATE, setup ≈ 4 s; star 10k rows + `Intersect`, chain via rel `r3t` 10k rows tanpa `Intersect`) dan triangle DB (N=41, 6 bulk single-edge CREATE `WHERE c.id > b.id AND b.id > a.id`, setup ≈ 8 s; expected `C(41,3)=10,660`). Assert lama salah (mengharapkan 10k rows dari setup cross product 10k×10k) — sudah dikoreksi ke desain kecil. Jalankan via `--test` mode (criterion hang di mesin ini, Decision #62). | `akar-main/benches/ladybug_suite.rs` | 🔜 PLANNED |
| P48.5 | **Tutup `test_migration_ingestion`** — **Audit 2026-08-02 (terkonfirmasi):** `CREATE NODE TABLE User` di `akar-migrate/src/main.rs:106` dieksekusi tanpa cek eksistensi. Test memakai `--from`=`--to` = dir yang sama yg sudah berisi DB (User dibuat di langkah 1) → `Bind error: Table 'User' already exists`, exit code 1. Proses migrasi tidak idempotent terhadap DB yang sudah dimigrasi. **FIXED 2026-08-03:** `main.rs` kini memeriksa `rust_db.get_table_id(table_name)` sebelum CREATE/COPY tiap node & rel table; jika sudah ada, table di-skip (CREATE + COPY) → migrasi idempotent. `test_migration_ingestion` PASS (sebelumnya FAIL). | `akar-migrate/src/main.rs` | ✅ DONE |
| P48.7 | **Fix Date/Timestamp/Time/Interval scan → ALL NULL** — **Audit 2026-08-02 (terkonfirmasi repro):** `DATE('2024-01-15')` dan `TIMESTAMP(...)` disimpan benar tapi scan kembali sebagai `null`. **Root cause sebenarnya (ditemukan saat repro 2026-08-03):** CREATE DML write path (`handle_ddl` → `BoundCreateDml` di `connection/ddl.rs:420`, dan `CreateDml` di `map_ddl.rs:312`) **hanya** mengevaluasi `Expression::Constant`; `DATE('...')` yang parse sebagai `FunctionCall("DATE",[String])` di-skip → `Value::Null` tersimpan. Bug kedua: `CastTarget::Date`/`Timestamp` di `evaluate_cast` tidak bisa parse String literal → `RETURN DATE('...')` juga NULL. Scan-side: `to_arrow_array` cabang Int64 (`column_chunk.rs:310-326`) tidak punya arm `Value::Date`/`Timestamp` → append_null; legacy `scan_filter/scan.rs:146-155` memetakan Date → String; `scanrel.rs:42-55` discard error. **FIXED 2026-08-03:** (a) helper baru `evaluate_constant_expr` (`set.rs`) — evaluasi Constant via `ast_constant_to_value` + `FunctionCall` via `registry.get_scalar` + `evaluate_scalar` — diwire ke kedua write path (create DML inline + pipeline); (b) `evaluate_cast` kini parse String → Date/Timestamp (`parse_date_string`/`parse_timestamp_string`); (c) `store_value_in_vector` + `build_arrow_from_values` Int64 branch kini menerima Date/Timestamp/TimestampTz/DTime (fix RETURN date literal juga); (d) arm scan-side Date/Timestamp → Int64. Repro `test_date_scan_null` + `test_timestamp_scan_null` **PASS** (sebelumnya FAIL); `cargo test -p akar-main --test repro_multihop` 9 passed (sisa `test_count_variable` = P48.14). | `akar-processor/src/physical/write_ops/set.rs`, `akar-main/src/connection/ddl.rs`, `akar-processor/src/processor/mapper/map_ddl.rs`, `akar-function/src/scalar/cast.rs`, `expression_evaluator.rs`, `column_chunk.rs`, `scan.rs` | ✅ DONE |
| P48.8 | **Fix FTS + WHERE scan index-OOB panic** — **Audit 2026-08-02 (dari kode):** di `scan_filter/scan.rs:326-356`, saat scan punya FTS query + predicate pushdown: `pred_chunk` dibangun dari array **penuh** (`pred_fields.push(arr.clone())`, size = `num_rows`), tapi `rows_to_emit` sudah dipersempit FTS (baris 292-297). Loop `for (i,&keep) in mask.iter()` (mask = `num_rows`) mengindeks `rows_to_emit[i]` yang pendek → **panic index out of bounds** (atau salah filter). | `akar-processor/src/physical/scan_filter/scan.rs` | 🔜 PLANNED |
| P48.9 | **Fix WHERE-predicate eval error → silent ALL-rows** — **Audit 2026-08-02 (dari kode):** `evaluate_expression(...).unwrap_or_else(|_| vec![true; ...])` di `scan.rs:347-348` dan `scan.rs:532-533` membuat error evaluasi (type mismatch, kolom predikat tak cocok) jadi no-op → hasil **salah** bukan error. | `akar-processor/src/physical/scan_filter/scan.rs` | 🔜 PLANNED |
| P48.10 | **Fix `read_art_varint` OOB** — `akar-storage/src/art_index.rs:41-53` membaca `data[*pos]` tanpa bounds check → panic pada index data corrupt/truncated. | `akar-storage/src/art_index.rs` | 🔜 PLANNED |
| P48.11 | **Fix silent 255-byte string truncation** — `vector.rs:351-354`, `vector.rs:491-496`, `common.rs:47-57`: string > 255 byte dipotong diam-diam ke slot 256-byte tanpa error. | `akar-common/src/vector.rs`, `common.rs` | 🔜 PLANNED |
| P48.12 | **Fix `Value::UInt64 as i64` narrowing** — `column_chunk.rs:318`, `scan.rs:193`: UInt64 > `i64::MAX` jadi negatif. | `akar-storage/src/column_chunk.rs`, `akar-processor/src/physical/scan_filter/scan.rs` | 🔜 PLANNED |
| P48.13 | **Fix ART PK-index creation silently skipped** — `map_ddl.rs:67-72` hanya `tracing::warn!` saat pembuatan ART PK-index gagal → index hilang diam-diam. | `akar-main/src/` | 🔜 PLANNED |
| P48.14 | **Fix `COUNT(<variable>)` → selalu 0** — **Audit 2026-08-02 (terkonfirmasi repro):** `MATCH (a:Person) RETURN COUNT(a)` = `Int64(0)` padahal `COUNT(*)` = 5. Root cause: `resolve_agg_col_indices` (`aggregatehashtable.rs:508-540`) gagal match `Expression::Variable("a")` ke field_names (berisi property seperti `a.id`) → `col_indices[i]` = None → fast path scalar COUNT (`splitaggregation.rs:126-132`) tak menambah → 0. Harusnya COUNT(node variable) menghitung baris non-null (selalu = COUNT(*) utk node yang wajib ada). | `akar-processor/src/physical/order_aggregate/aggregatehashtable.rs`, `splitaggregation.rs` | 🔜 PLANNED |

**Temuan scan tambahan (audit 2026-08-02, dari subagent — belum diverifikasi langsung):** SIP semi-mask kemungkinan mati (perf-only, `map_scan.rs:70-72` attach mask id 0 tapi `to_column_major_data*` tak menyertakan kolom InternalID); `catalog.rs:443` `pk_col` default 0 jika tanpa PK; `common.rs:78` NaN comparison = Equal; `misc.rs:42` `get_value(...).unwrap_or(Value::Null)` mask index OOB.
| P48.6 | **README server-mode note** — acceptance criteria P47 yang belum tuntas: dokumentasikan "multi-writer" = multi-thread in-process + multi-process via optional `akar-server`. | `README.md` | 🔜 PLANNED |

**Urutan kerja:** P48.1 → P48.2 → P48.3 → P48.5 → P48.4 → P48.6 → P48.7 → P48.8 → P48.9 → P48.10 → P48.11 → P48.12 → P48.13 → P48.14.
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
