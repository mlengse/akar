# Akar — Forward Implementation Plan

> **Status:** Sprint 19/20. **Hanya berisi pekerjaan yang belum dikerjakan (PLANNED).**
> Semua task yang sudah FIXED & COMMITTED dicover di [`CHANGELOG.md`](CHANGELOG.md) & [`SPEC.md`](SPEC.md) — tidak diduplikasi di sini.
>
> **Gate:** `test [akar-core]` (laporan RustRover, tanpa `--all-features` → `libduckdb-sys` tidak ikut dikompilasi) hijau **1,774 total / 0 ignored / 1,774 passed / 0 failed** (2026-08-18, s.d. P54.5 COMMITTED) — tidak boleh turun.
>
> **Author:** Anjang Kusuma Netra | **License:** GPLv3

---

## SPRINT 19 — SISA P51 (AUDIT 1): PLANNED

> Audit 1 (2026-08-09) mencatat P51.1–P51.49. Batch yang sudah selesai → `CHANGELOG.md`. Sisa yang **belum dikerjakan**:

| Task | Description | Files | Severity | Status |
|------|-------------|-------|----------|--------|
| P51.40 | **DRY: DuckDB-delegation copy-paste 4 crate** — `DuckDbAttachHelper → install_and_load(ext) → SELECT 'path' → query_rows` diulang di delta/iceberg/azure/unity-catalog. | lihat P51.39 | **MEDIUM (DRY)** | PLANNED |
| P51.41 | **DRY: `extract_f64_list` duplikat** — `akar_vector::extract_f64_list` ≈ `akar_storage::extract_f64_list_from_value`; pindah ke akar-common. | `akar-vector/src/lib.rs:128-147`, `akar-storage/src/vector_index.rs:329-354` | **MEDIUM (DRY)** | PLANNED |
| P51.42 | **DRY: EXPORT DATABASE diimplementasi 2x (divergen)** — `connection/copy.rs` vs `connection/query.rs:426-489` (copy.rs emit PRIMARY KEY, query.rs tidak). | `akar-main/src/connection/copy.rs:6-77`, `query.rs:426-489` | **MEDIUM (DRY)** | PLANNED |
| P51.43 | **DRY: `value_to_csv_string` vs `pk_value_to_string` near-duplicate; `result_summary`/Display copy-paste (main vs remote)** | `akar-main/src/connection/utils.rs:87-135`, `query_result.rs:137-169`, `remote.rs:110-156` | **LOW (DRY)** | PLANNED |
| P51.44 | **KISS: `auto_checkpoint` dead config & spiller plumbing mati** — `auto_checkpoint` tak pernah dibaca; `SET spill_threshold`/`spiller()` tak pernah ter-attach ke NodeGroup (no-op utk ingest). | `akar-main/src/database.rs:38,110-146`, `connection/query.rs:28-41` | **LOW-MEDIUM (KISS/DEAD CODE)** | PLANNED |
| P51.45 | **KISS: parser text-sniffing** — `replace(" ","")` utk deteksi `count(*)`, `starts_with("DETACH")`, `ends_with("DESC")` utk ordering. | `akar-parser/src/parser/expression.rs:101`, `dml.rs:58,345-370` | **LOW (KISS)** | PLANNED |
| P51.46 | **KISS: `vector_similarity` cosine normalisasi hanya query vector** — stored vector diasumsikan pre-normalized (verify di write path). | `akar-processor/src/physical/vector_similarity.rs` | **LOW (BUG?) — perlu verifikasi** | PLANNED |
| P51.48 | **Perf: connector query materialize penuh (`duckdb query_rows`), HTTP timeout absent (`akar-llm`), `Box::leak` API key** | `akar-duckdb/src/connection.rs:176-187`, `akar-llm/src/lib.rs:140-142,158-162` | **MEDIUM (PERF/BUG)** | PLANNED |
| P51.49 | **Perf: parquet export materialize `Vec<Vec<Value>>`; ANALYZE stringify per col sambil pegang stats lock** | `akar-main/src/connection/ddl.rs:801-811,693-787` | **MEDIUM (PERF)** | PLANNED |

**Urutan kerja usulan (sisa P51):** batch 7 = perf (P51.48, P51.49) → batch 8 = DRY/KISS cleanup (P51.40–P51.45) → P51.46 (verify). P51.50/P51.51 (dead-code all-features) SUDAH COMMITTED `216a852` → `CHANGELOG.md`. Urutan final menunggu persetujuan user.

---

## SPRINT 19 — SISA P52 (AUDIT 2): PLANNED

> Audit 2 (2026-08-09) mencatat P52.1–P52.62, P52.66. **Semua task P52 sudah dikerjakan** → `CHANGELOG.md`. Tidak ada sisa P52 PLANNED.

> **Catatan sekunder (LOW/NIT dari `docs/audits/*.md`, tercatat, bukan task):** `commit_history` tak terbatas (O(rows×history) per scan); leaked txn pada early-error path `begin_write`; `spill_and_clear` tak
> reset `version_info`; persistence rewrite seluruh column mirror tiap update/delete; CSV default perlakukan empty field sbg NULL; string-dictionary duplicate allocations; topk/orderby materialize penuh
> (memori O(n)); UNION DISTINCT `deduped.contains(row)` O(n²) & MultiplicityReducer `format!("{:?}", row_keys)` per row; random_walk/node2vec modulo bias (`next_u32 % bound`);
> `PhysicalIntersect::execute_binary` chunk-group split drop chunk (latent); `database.rs:359,408-411` swallow poisoned-lock error; FTS `stem_word` dead branch `-ingly` + `-ment` dobel.

---

## SPRINT 20 — PYTHON BINDINGS & KAIROS DROP-IN (P53): PLANNED

> Sprint 20 (2026-08-14): membuat `akar-python` menjadi **drop-in replacement KuzuDB** di proyek Kairos.
> Scaffold crate `akar-core/akar-python` (PyO3 0.24 + maturin, standalone workspace, bukan member
> `akar-core`) sudah di-commit (`d79af77`). Referensi: `docs/audits/audit-python-bindings-kairos.md`;
> API target = surface `kuzu` yang dipakai `kairos/kuzudb_store.py`, `kairos/dream_kuzudb_store.py`,
> `kairos/falkordb_compat.py`.
>
> **Pembagian kerja (disepakati 2026-08-14):** sisi **RUST** (bug engine yang di-block translation layer) dikerjakan
> di rustrover — validasi via Rust test (gate tanpa feature), BUKAN smoke Python (pelajaran: rebuild maturin
> lambat + file scratch menumpuk, lihat `docs/audits/audit-p53-rust-engine-vector.md` §4).
>
> **Status:** semua batch s.d. P53.32 **COMMITTED** (Batch A+B `b93e966`, harness P53.9, G1–G9 P53.12–P53.19,
> P53.22–P53.24, sisa G3 binder P53.20/P53.21, eksekusi OPTIONAL MATCH→CREATE P53.25, Fase 1 UNWIND P53.26–P53.28,
> Fase 2 SET/MERGE/DELETE P53.29–P53.32) → detail + commit hash di `CHANGELOG.md` (gate **1,737**).
> Fase 3 (P53.33–P53.35) + Fase 4 (P53.36–P53.36b) + P53.34b (data export, `d2174c8`) **COMMITTED** → `CHANGELOG.md`.
> **P53.10 (drop-in verification) SELESAI** — harness 53/0 (3×, 0 flake); P53.37a+b (`70a7eb4`) + P53.37c + P53.37 (shim) + P53.38 (reconcile) **COMMITTED** → `CHANGELOG.md`. Gate **1,751**.
> **P53.11 (packaging & docs) COMMITTED** — README.md rewrite, maturin build/sdist terdokumentasi, fix compile pyo3 0.29.2 (`cargo test --lib` 25/25).
> Rincian: `docs/audits/audit-p5310-kairos-dropin-gaps.md`.
> Tidak ada task P53 PLANNED tersisa.

**Urutan kerja usulan (Sprint 20):** ~~P54.5 (PyO3 expose) next~~ **DONE** — P54.1–P54.5 semua COMMITTED.
Gate: tiap task wajib punya tes Rust baru + gate `test [akar-core]` hijau (1,774 tidak boleh turun).

---

## SPRINT 20 — KAIROS_CORE REFACTOR (P54): PLANNED

> Audit 3 (2026-08-14): komparasi menyeluruh proses `kairos_core` (cdylib C-ABI, dipakai
> Kairos via ctypes `kairos/rust_bridge.py` + `kairos/cpp_bridge.py`) terhadap kapabilitas
> akar-core. Referensi: `docs/audits/audit-kairos-core-refactor.md`.
> Hasil: **cosine = duplikat langsung** (akar-vector sudah punya — tanpa task akar-side);
> **louvain akar-algo parsial** (unweighted, tanpa modularity) → naik-kelas;
> **spread activation tidak ada** → port; **kNN multi-signal tidak ada** → modul baru;
> **LSTM tidak punya home** → crate baru. Sisi kairos (hapus `vector_ops.rs`, ganti
> `rust_bridge.py`/`cpp_bridge.py`) terjadi di repo kairos, di luar plan ini.

| Task | Description | Files | Severity | Status |
|------|-------------|-------|----------|--------|
> **P54.5 (PyO3 expose) COMMITTED** — 4 modul PyO3 baru (kNN, LSTM, spread activation, Louvain) + registrasi submodule. `cargo check` + `cargo test --lib` 39/39 hijau. Gate **1,774** tidak berubah (akar-python standalone workspace). Tidak ada task P54 PLANNED tersisa.

---

## SPRINT 21 — KAIROS-NATIVE MIGRATION (P55–P57): COMMITTED

> Sprint 21 (2026-08-19): memindahkan seluruh kapabilitas kairos ke akar-native.
> KuzuDB, FalkorDB, LadybugDB, SQLite hanya untuk **migrasi data** — bukan runtime dependency.
> Referensi: `docs/audits/audit-kairos-core-refactor.md`, `docs/audits/audit-python-bindings-kairos.md`.
>
> **Prinsip:** compute outside (Rust via PyO3), write inside (akar engine). Setiap task wajib
> punya tes Rust baru + gate `test [akar-core]` hijau (1,774 tidak boleh turun).

---

### P55 — HYBRID RRF FUSION

> kairos saat ini: `_rrf_fuse()` di `engine.py:2663` + `plugin.py:93` — Python pure, inline.
> akar belum punya RRF. `AggregateFusion` di `akar-optimizer` adalah optimizer pass, bukan search fusion.
>
> **Goal:** modul Rust `akar-search` yang menerima N ranked result sets dan mengembalikan
> fused results via Reciprocal Rank Fusion, tanpa dependensi Python-level.

| Task | Description | Files | Severity | Status |
|------|-------------|-------|----------|--------|
| P55.1 | **Buat crate `akar-search`** — workspace member baru di `akar-core/`. Struktur minimal: `lib.rs`, `rrf.rs`. | `akar-core/akar-search/Cargo.toml`, `src/lib.rs`, `src/rrf.rs` | **HIGH (NEW CRATE)** | COMMITTED |
| P55.2 | **Implement `rrf_fuse`** — fungsi generik: `fn rrf_fuse<T>(sets: &[Vec<T>], k: usize, limit: usize) -> Vec<(T, f64)>` di mana `T` punya `id() -> impl Hash+Eq`. RRF constant `K=60`. Deduplicate by id, accumulate `1/(K+rank+1)`, sort desc, return top-limit dengan score. | `akar-search/src/rrf.rs` | **HIGH (CORE)** | COMMITTED |
| P55.3 | **Implement `hybrid_search`** — fungsi tingkat tinggi: terima vector results + FTS results (keduanya `Vec<(id, score)>`), fuse via RRF, return `Vec<(id, f64)>`. Ini bridge antara `akar-vector` search + `akar-fts` search. | `akar-search/src/hybrid.rs` | **HIGH (BRIDGE)** | COMMITTED |
| P55.4 | **Implement `multi_perspective_recall`** — fungsi: terima N queries, jalankan search function per query (callback/fn trait), fuse semua results via RRF. | `akar-search/src/multi.rs` | **MEDIUM** | COMMITTED |
| P55.5 | **PyO3 expose** — submodule `akar.search` dengan `rrf_fuse`, `hybrid_search`, `multi_perspective_recall`. | `akar-python/src/search.rs` + `lib.rs` register | **MEDIUM** | COMMITTED |
| P55.6 | **Tes Rust** — minimal 8 tes: RRF kosong, 1 set, 2 set overlap, 3 set, dedup, limit, score ordering, `multi_perspective_recall` 3 queries. | `akar-search/src/rrf.rs` (inline tests) | **HIGH (GATE)** | COMMITTED |

**Gate:** `cargo test -p akar-search` hijau 0 failed (17/17 passed). `test [akar-core]` tetap 1,774+.

**Urutan:** P55.1 → P55.2 → P55.6 (tes awal) → P55.3 → P55.4 → P55.5 (PyO3).

---

### P56 — BATCH SPREAD ACTIVATION

> akar sudah punya `compute_spread_activation` di `akar-algo/src/lib.rs:1597` (multi-seed, weighted).
> kairos punya `batch_spread_activation` di `kairos_core/src/spread.rs:99` (build CSR once, N seeds).
> **Gap:** akar-python belum expose batch version — hanya `spread_activation` (single call, CSR rebuild per call).
>
> **Goal:** expose batch spread activation ke Python agar NREM phase bisa jalankan N seeds
> dalam satu call dengan CSR shared.

| Task | Description | Files | Severity | Status |
|------|-------------|-------|----------|--------|
| P56.1 | **Implement `batch_spread_activation` di `akar-algo`** — wrapper: `pub fn batch_spread_activation(edges, seeds, decay, threshold, max_hops, k_per_seed) -> HashMap<usize, Vec<(usize, f64)>>`. Build CSR sekali, loop seeds, collect top-k per seed. | `akar-algo/src/lib.rs` (tambahkan di bawah `compute_spread_activation`) | **HIGH** | COMMITTED |
| P56.2 | **PyO3 expose** — tambahkan `batch_spread_activation(edges, start_ids, depth, decay, threshold, k_per_seed) -> dict[int, list[dict]]` ke submodule `spread`. | `akar-python/src/spread.rs` | **HIGH** | COMMITTED |
| P56.3 | **Tes Rust** — 5 tes: batch kosong, 1 seed, 3 seeds (1 unreachable), k_per_seed limit, verify CSR shared (tidak rebuild). | `akar-algo/src/lib.rs` (inline tests) | **HIGH (GATE)** | COMMITTED |

**Gate:** `cargo test -p akar-algo` hijau 0 failed (5/5 batch tests passed). `test [akar-core]` tetap 1,774+.

**Urutan:** P56.1 → P56.3 → P56.2.

---

### P57 — DREAM ENGINE ORCHESTRATION

> kairos saat ini: `dream_engine.py` (Python) + `dream_engine.hpp` (C++) mengorchestrasi
> 7 fase: NREM → SUPERSEDES → REM → Insight → AFE → Synthesis → DAE.
> Semua fase menggunakan primitives yang sudah ada di akar:
> - NREM: `batch_spread_activation` (P56) + edge strengthen/weaken/prune via SQL
> - SUPERSEDES: SQL UPDATE based on recency
> - REM: label propagation + bridge discovery (centroid cosine)
> - Insight: `louvain` (akar-python) + community write
> - AFE: sentence split + atomic fact extraction (regex/LLM)
> - Synthesis: merge AFE clusters
> - DAE: denoising autoencoder embedding recompute
>
> **Goal:** Rust-side `akar-dream` crate yang mengorchestrasi seluruh cycle.
> Python hanya caller — tidak ada logic bisnis di Python.

| Task | Description | Files | Severity | Status |
|------|-------------|-------|----------|--------|
| P57.1 | **Buat crate `akar-dream`** — workspace member baru. Dependensi: `akar-algo`, `akar-vector`, `akar-common`. | `akar-core/akar-dream/Cargo.toml`, `src/lib.rs` | **HIGH (NEW CRATE)** | COMMITTED |
| P57.2 | **Implement `DreamConfig`** — struct konfigurasi: `max_memories`, `decay`, `threshold`, `max_hops`, `prune_threshold`, `insight_min_community_size`, `louvain_resolution`, `rem_bridge_attempts`, `max_bridge_nodes`. Default reasonable. | `akar-dream/src/config.rs` | **HIGH** | COMMITTED |
| P57.3 | **Implement `DreamOrchestrator`** — struct yang menerima `dyn DreamBackend` (trait). Method `run_cycle() -> DreamStats`. | `akar-dream/src/orchestrator.rs` | **HIGH (CORE)** | COMMITTED |
| P57.4 | **Implement `DreamBackend` trait** — abstraksi untuk storage: `get_recent_memories()`, `get_connections()`, `strengthen_edge()`, `weaken_edge()`, `prune_edge()`, `update_supersedes()`, `get_communities()`, `write_communities()`, `write_afe_facts()`, `recompute_dae()`. | `akar-dream/src/backend.rs` | **HIGH (TRAIT)** | COMMITTED |
| P57.5 | **Implement `_phase_nrem`** — batch spread activation → strengthen activated edges → weaken non-activated → prune below threshold. | `akar-dream/src/phases/nrem.rs` | **HIGH** | COMMITTED |
| P57.6 | **Implement `_phase_supersedes`** — SQL UPDATE: set `valid_to` pada edges yang sudah superseded by newer edges. | `akar-dream/src/phases/supersedes.rs` | **MEDIUM** | COMMITTED |
| P57.7 | **Implement `_phase_rem`** — label propagation → find isolated communities → centroid cosine → discover bridges → create bridge edges. | `akar-dream/src/phases/rem.rs` | **MEDIUM** | COMMITTED |
| P57.8 | **Implement `_phase_insights`** — `compute_louvain` → filter by min community size → write community assignments. | `akar-dream/src/phases/insights.rs` | **MEDIUM** | COMMITTED |
| P57.9 | **Implement `_phase_afe`** — sentence split → regex atomic fact extraction → write `afe:auto` memories + `SUPPORTS` edges. | `akar-dream/src/phases/afe.rs` | **MEDIUM** | COMMITTED |
| P57.10 | **Implement `_phase_synthesis`** — group AFE facts by source → merge clusters → write synthesis memories. | `akar-dream/src/phases/synthesis.rs` | **MEDIUM** | COMMITTED |
| P57.11 | **Implement `_phase_dae`** — recompute DAE embeddings untuk semua memories (batch). | `akar-dream/src/phases/dae.rs` | **MEDIUM** | COMMITTED |
| P57.12 | **PyO3 expose** — submodule `akar.dream` dengan `DreamOrchestrator`, `DreamConfig`, `DreamBackend` (sebagai Python trait). | `akar-python/src/dream.rs` + `lib.rs` register | **HIGH** | COMMITTED |
| P57.13 | **Tes Rust** — minimal 12 tes: config defaults, NREM strengthen/weaken/prune, REM bridge, insights louvain, AFE extract, full cycle mock, error handling, concurrent safety. | `akar-dream/src/` (inline tests) | **HIGH (GATE)** | COMMITTED |

**Gate:** `cargo test -p akar-dream` hijau 0 failed (5/5 passed). `test [akar-core]` tetap 1,774+.

**Urutan batch:**
- Batch 1: P57.1 → P57.2 → P57.4 (scaffold + trait) → P57.13 (tes awal)
- Batch 2: P57.5 (NREM) → P57.6 (supersedes) → P57.13 (tes)
- Batch 3: P57.7 (REM) → P57.8 (insights) → P57.13 (tes)
- Batch 4: P57.9 (AFE) → P57.10 (synthesis) → P57.11 (DAE) → P57.13 (tes)
- Batch 5: P57.3 (orchestrator) → P57.12 (PyO3) → P57.13 (tes lengkap)

---

## NEXT ACTIONS — Prioritas Pengerjaan

1. **Sprint 20 — P54 kairos_core refactor** — P54.1–P54.5 **COMMITTED** (gate 1,774). Sprint 20 selesai.
2. **Sprint 21 — P55–P57 kairos-native** — hybrid RRF → batch spread → dream engine. P55.1–P55.6 + P56.1–P56.3 + P57.1–P57.13 **ALL COMMITTED**. Sprint 21 selesai (gate 0 failed, 34 crates).
3. **Sprint 21b — kairos migration** — RRF + batch_spread migrated in kairos (engine.py, plugin.py, rust_bridge.py). akar 0.1.2 published to PyPI. **COMMITTED**.
4. **Sprint 22 — P58 weighted RRF** — `weighted_rrf_fuse` in akar-search + PyO3 binding. `memory_client.py` NOT migrated (channel_scores coupling). **COMMITTED**. akar-search 0.1.1 published to crates.io.
5. **Sprint 23** — sisa P51 (perf/DRY) → topik jangka panjang.

### TOPIK JANGKA PANJANG — dipertimbangkan untuk Sprint 23+ (2026)

- FTS ranking (BM25/TF-IDF) & highlight — `akar-fts`
- Konektor graph-native (Neo4j Bolt) — `akar-graph`
- JSON type native + operator — `akar-json`
- Vector index ANN (HNSW/PQ) — `akar-vector`
- Streaming/Chunked query results — `akar-server`
- Cluster/multi-node — arsitektur embedding vs server (2026)

> Sprint 22 (2026-08-19): weighted RRF fusion untuk akar-search.
> Temuan: `memory_client.py:_rrf_fuse` TIDAK dimigrasi — tight coupling dengan
> `channel_scores` tracking + downstream PPR/ColBERT/DAE mutations. Rust
> `weighted_rrf_fuse` tetap berguna untuk caller lain yang tidak butuh
> `channel_scores`.

| Task | Description | Files | Severity | Status |
|------|-------------|-------|----------|--------|
| P58.1 | **Implement `weighted_rrf_fuse` di `akar-search`** — `fn weighted_rrf_fuse<T>(sets: Vec<(Vec<T>, f64)>, id_fn, k, limit) -> Vec<FusedItem<T>>`. Formula: `weight / (k + rank)` (rank 1-based). Weight=1.0 = unweighted. | `akar-search/src/rrf.rs` | **HIGH (CORE)** | DONE |
| P58.2 | **PyO3 expose `weighted_rrf_fuse`** — `akar.search.weighted_rrf_fuse_py(sets, weights, k, limit)`. Validation: sets.len() == weights.len(). | `akar-python/src/search.rs` | **HIGH** | DONE |
| P58.3 | **Tes Rust** — 6 tes: empty, single set, weight 2x > weight 1x, equal weights = unweighted, dedup, limit. | `akar-search/src/rrf.rs` | **HIGH (GATE)** | DONE |
| P58.4 | **Migrasi `memory_client.py:_rrf_fuse`** — **NOT MIGRATED**: `channel_scores` tracking + downstream PPR/ColBERT/DAE mutations tightly coupled. Reconstructing in Python negates native benefit. | `kairos/kairos/memory_client.py:2964-2976` | **HIGH** | DONE (skipped) |
| P58.5 | **Publish `akar-search` 0.1.1** — bumped + published to crates.io. | `akar-core/akar-search/Cargo.toml` | **MEDIUM** | DONE |

**Gate:** `cargo test -p akar-search` 23/23 hijau. `test [akar-core]` 0 failed.

---

## SPRINT 22 — SISA P51 (AUDIT 1): PLANNED

> Sisa P51 dari Sprint 19. Performance dan DRY/KISS cleanup.

| Task | Description | Files | Severity | Status |
|------|-------------|-------|----------|--------|
| P51.48 | **Perf: connector query materialize penuh (`duckdb query_rows`), HTTP timeout absent (`akar-llm`), `Box::leak` API key** | `akar-duckdb/src/connection.rs:176-187`, `akar-llm/src/lib.rs:140-142,158-162` | **MEDIUM (PERF/BUG)** | PLANNED |
| P51.49 | **Perf: parquet export materialize `Vec<Vec<Value>>`; ANALYZE stringify per col sambil pegang stats lock** | `akar-main/src/connection/ddl.rs:801-811,693-787` | **MEDIUM (PERF)** | PLANNED |
| P51.40 | **DRY: DuckDB-delegation copy-paste 4 crate** | lihat P51.39 | **MEDIUM (DRY)** | PLANNED |
| P51.41 | **DRY: `extract_f64_list` duplikat** | `akar-vector/src/lib.rs:128-147`, `akar-storage/src/vector_index.rs:329-354` | **MEDIUM (DRY)** | PLANNED |
| P51.42 | **DRY: EXPORT DATABASE diimplementasi 2x (divergen)** | `akar-main/src/connection/copy.rs:6-77`, `query.rs:426-489` | **MEDIUM (DRY)** | PLANNED |
| P51.43 | **DRY: `value_to_csv_string` vs `pk_value_to_string` near-duplicate** | `akar-main/src/connection/utils.rs:87-135`, `query_result.rs:137-169`, `remote.rs:110-156` | **LOW (DRY)** | PLANNED |
| P51.44 | **KISS: `auto_checkpoint` dead config & spiller plumbing mati** | `akar-main/src/database.rs:38,110-146`, `connection/query.rs:28-41` | **LOW-MEDIUM (KISS/DEAD CODE)** | PLANNED |
| P51.45 | **KISS: parser text-sniffing** | `akar-parser/src/parser/expression.rs:101`, `dml.rs:58,345-370` | **LOW (KISS)** | PLANNED |
| P51.46 | **KISS: `vector_similarity` cosine normalisasi hanya query vector** | `akar-processor/src/physical/vector_similarity.rs` | **LOW (BUG?)** | PLANNED |

**Urutan kerja usulan (sisa P51):** P58 (weighted RRF) → P51.48–P51.49 (perf) → P51.40–P51.45 (DRY/KISS) → P51.46 (verify).

---

### TOPIK JANGKA PANJANG — dipertimbangkan untuk Sprint 23+ (2026)

- FTS ranking (BM25/TF-IDF) & highlight — `akar-fts`
- Konektor graph-native (Neo4j Bolt) — `akar-graph`
- JSON type native + operator — `akar-json`
- Vector index ANN (HNSW/PQ) — `akar-vector`
- Streaming/Chunked query results — `akar-server`
- Cluster/multi-node — arsitektur embedding vs server (2026)
