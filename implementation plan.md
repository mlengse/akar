# Akar — Forward Implementation Plan

> **Status:** Sprint 19/20. **Hanya berisi pekerjaan yang belum dikerjakan (PLANNED).**
> Semua task yang sudah FIXED & COMMITTED dicover di [`CHANGELOG.md`](CHANGELOG.md) & [`SPEC.md`](SPEC.md) — tidak diduplikasi di sini.
>
> **Gate:** `test [akar-core]` (laporan RustRover, tanpa `--all-features` → `libduckdb-sys` tidak ikut dikompilasi) hijau **1,751 total / 0 ignored / 1,751 passed / 0 failed** (2026-08-17, s.d. P53.37c COMMITTED) — tidak boleh turun.
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
> **Investigasi P53.10 (2026-08-16):** harness P53.9 **4 failed / 49 passed** → 4 gap = 3 bug engine Rust
> (P53.37a UNWIND-after-MATCH, P53.37b SET pipeline, P53.37c DELETE persist) + 1 wrapper (P53.37 shim).
> **P53.37a+b SUDAH COMMITTED** (`70a7eb4`, CHANGELOG `1164ffa`) → `CHANGELOG.md` — UNWIND variable
> dipertahankan melalui join-reorder & Extend (kolom map/struct), SET diaplikasikan di pipeline UNWIND→MATCH;
> regression Rust setara harness A2 (`add_connections_batch`) & A3 (`batch_strengthen_connections`) green.
> **P53.37c SUDAH COMMITTED** → `CHANGELOG.md` — `RETURN count(*)` setelah DELETE kini melaporkan
> jumlah terhapus sebenarnya (audit D1: root cause = count-chunk 1-baris, bukan persistensi — scan MVCC
> sudah memfilter baris soft-deleted). Gate **1,751**. **P53.37 (shim kairos) SUDAH COMMITTED** →
> `CHANGELOG.md` — `kairos/kuzu.py` shim + `__init__.py` import; harness 53/0 (3×, 0 flake); smoke test
> end-to-end all passed. Sisa: P53.38 (reconcile).
> Rincian: `docs/audits/audit-p5310-kairos-dropin-gaps.md`; dokumen asal di `kairos/docs/plans/akar-dropin-p53.10-{findings,implementation-plan}.md`.
> Hanya pekerjaan yang **belum dikerjakan** tercantum di bawah.

| Task | Description | Files | Severity | Status |
|------|-------------|-------|----------|--------|
| P53.10 | **Verifikasi drop-in end-to-end di Kairos** — patch `import kuzu` → `import akar` (modul `kairos/kuzu.py` shim atau langsung), jalankan alur Kairos nyata, fix gap yang muncul dari P53.9. **Blokir:** P53.25. **Breakdown 5 fase → P53.26–P53.38 di bawah.** | kairos (repo terpisah) | **MEDIUM (VAL)** | PLANNED |
| P53.11 | **Packaging & docs** — `maturin build`/`sdist` release, update `akar-python/README.md`, catat batch + commit di `CHANGELOG.md` (format KAC), update `SPEC.md` (ekosistem ekstensi: +Python bindings) & plan (hapus task selesai). | `akar-python/`, `CHANGELOG.md`, `SPEC.md` | **LOW (KISS)** | PLANNED |

### P53.10 breakdown — fix engine (dari audit 2026-08-16)

> Baseline harness `akar-core/akar-python/tests/test_kuzu_compat.py`: **4 failed / 49 passed** → target **0 failed**.
> Fase 1–4 + P53.34b + **P53.37a+b+c** + **P53.37 (shim)** sudah selesai → `CHANGELOG.md` (harness 53/0, 3×).
> Sisa: P53.38 (reconcile).
> Tiap task = 1+ tes Rust baru; gate `test [akar-core]` 1,751 tidak boleh turun.
> Detail root-cause + probe: `docs/audits/audit-p5310-kairos-dropin-gaps.md`.

**Fase 1 — UNWIND clause correctness**: SELESAI → `CHANGELOG.md`.

**Fase 2 — SET / MERGE / DELETE write correctness**: SELESAI → `CHANGELOG.md`.

**Fase 3 — Precision & maintenance lifecycle** (E1/E2/E3): SELESAI → `CHANGELOG.md`.

**Fase 4 — Wrapper polish**: SELESAI → `CHANGELOG.md` (harness 49/4).

**Fase 5 — End-to-end drop-in (scope P53.10)** — P53.37a (A2 `add_connections_batch`) & P53.37b (A3 `batch_strengthen_connections`) **SUDAH COMMITTED** `70a7eb4`; P53.37c (`prune_connection_history` count) **SUDAH COMMITTED**; P53.34b (repair_schema data export) COMMITTED `d2174c8`; **P53.37 (shim kairos + smoke) SUDAH COMMITTED** → semua `CHANGELOG.md` (regression Rust setara harness green, harness 53/0 3×). Sisa: reconcile:

| Task | Description | Files | Severity | Status |
|------|-------------|-------|----------|--------|
| P53.37 | **Shim `kairos/kuzu.py` + smoke alur Kairos nyata** — alias `import kuzu` → `import akar` (sys.modules); smoke: store memories + embeddings, search_semantic/search_bm25, satu siklus dream (strengthen/prune), close/reopen; weight exact 0.95, embedding round-trip. **Blokir:** engine gaps P53.37a–c sudah fix (`70a7eb4` + commit P53.37c) — tinggal re-run harness P53.9 (setelah `maturin develop`) untuk konfirmasi 0 failed. | kairos (`engine.py`, `kuzu.py` shim) | **MEDIUM (VAL)** | COMMITTED |
| P53.38 | **Gate re-check + reconcile** — `test [akar-core]` tetap 1,751; harness P53.9 dijalankan 3× (cek flake order-dependent, khususnya close/reopen); update `CHANGELOG.md` (KAC) + `SPEC.md` + hapus task selesai dari plan. | `CHANGELOG.md`, `SPEC.md`, plan | **LOW (KISS)** | PLANNED |

**Urutan kerja usulan (Sprint 20):**
1. **Berikutnya:** Fase 5 — P53.37 (shim kairos + smoke; butuh re-run harness P53.9 dulu untuk baseline 0 failed) → P53.38 (re-check + reconcile) → P53.11 packaging & docs.
2. Gate: tiap task wajib punya tes Rust baru + gate `test [akar-core]` hijau (1,751 tidak boleh turun); setelah semua gap fix, jalankan ulang harness P53.9 → target 0 failed.

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
| P54.1 | **Naik-kelas `compute_louvain` akar-algo** — dukung edge **weighted** + `seed` + `min_gain`/`max_iterations` + output **modularity** (kairos_core `louvain.rs` lebih kaya dari versi akar). Setelah ini `CALL louvain()` bisa output modularity & node id asli; kairos buang `louvain.rs`-nya. Catatan: CSRAdjacency tak simpan edge weight → weight array/fn (pola `weight_fn` `akar-algo/src/lib.rs:1500-1551`). | `akar-algo/src/lib.rs:1241-1344`, `kairos_core/src/louvain.rs:6-129` | **MEDIUM** | PLANNED |
| P54.2 | **Port spreading activation (single + batch)** ke akar-algo di atas `CSRAdjacency` + weight_fn — propagasi `act*weight*decay`, pruning `threshold`, top-k per seed; build adjacency sekali utk batch; expose `CALL spread_activation(...)`. Port dari kairos_core `spread.rs`; CSR ≪ HashMap adjacency utk graf besar. | `akar-algo/src/lib.rs` (baru), `kairos_core/src/spread.rs:18-187` | **MEDIUM** | PLANNED |
| P54.3 | **Modul kNN multi-signal re-ranker** di akar-vector — `combined = w_e*(cos+1)/2 + w_t*exp(-age*0.01) + w_f*ln(freq+1)/ln(1000) + w_g*graph`, default (0.6/0.15/0.1/0.15), top-k; memakai `akar_vector::cosine_similarity` yang sudah ada. Opsi SQL: dekomposisi `w_e*array_cosine_similarity(...) + ...` (butuh fn temporal/freq). | `akar-vector/src/` (baru), `kairos_core/src/knn.rs:61-179` | **MEDIUM** | PLANNED |
| P54.4 | **Port LSTM 1-layer (forward/train/save/load)** ke crate baru `akar-ml` (atau perluas `akar-llm`) — stand-alone (rand+serde). Keputusan desain: training model masuk engine DB vs library sisi-Python (PyO3). | `akar-core/akar-ml/` (baru), `kairos_core/src/lstm.rs:23-368` | **MEDIUM (butuh keputusan desain)** | PLANNED |
| P54.5 | **Expose kNN/LSTM/spread/louvain sebagai module PyO3** di akar-python agar Kairos ganti ctypes `rust_bridge.py`/`cpp_bridge.py` → `import akar` (stateful/hot-loop paling cocok PyO3; graph analytics bisa CALL). Bergantung P54.1–P54.4. | `akar-python/src/` | **LOW-MEDIUM** | PLANNED |

**Urutan kerja usulan (P54):** P54.1 + P54.2 (graph, independen) → P54.3 (vector) → P54.4 (ml; tunggu keputusan desain) → P54.5 (pyo3; bergantung). Gate tiap batch: `test [akar-core]` tidak boleh turun. Urutan final menunggu persetujuan user.

---

## NEXT ACTIONS — Prioritas Pengerjaan

1. **Sprint 20 — Python-side P53 (pycharm)** — Batch A+B (P53.1–P53.8) s.d. P53.32 COMMITTED; Fase 1–4 + P53.34b (`d2174c8`) + **P53.37a+b (`70a7eb4`) + P53.37c + P53.37 (shim)** COMMITTED → detail di `CHANGELOG.md`, gate **1,751**, harness **53/0** (3×). **Berikutnya: P53.38** (re-check + reconcile) → P53.11 packaging & docs.
2. **Sprint 20 — P54 kairos_core refactor** — P54.1+P54.2 (louvain naik-kelas, spread port) → P54.3 (kNN re-ranker) → P54.4 (LSTM/akar-ml) → P54.5 (PyO3 expose).
3. **Sisa P51** — P51.48–P51.49 (perf connector/parquet) → P51.40–P51.46 (DRY/KISS) → P51.46 verify.
4. **Sprint 21** — topik jangka panjang (di bawah).

### TOPIK JANGKA PANJANG — dipertimbangkan untuk Sprint 21+ (2026)

- FTS ranking (BM25/TF-IDF) & highlight — `akar-fts`
- Konektor graph-native (Neo4j Bolt) — `akar-graph`
- JSON type native + operator — `akar-json`
- Vector index ANN (HNSW/PQ) — `akar-vector`
- Streaming/Chunked query results — `akar-server`
- Cluster/multi-node — arsitektur embedding vs server (2026)
