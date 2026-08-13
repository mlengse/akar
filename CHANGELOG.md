# Changelog — Akar

> Riwayat revisi Sprint 19 (P51/P52), dipisah dari `implementation plan.md` agar plan hanya berisi pekerjaan yang belum dikerjakan. Fase selesai P1–P50 + AUDIT → [`SPEC.md`](SPEC.md) & git history.
> Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) · Versi: [SemVer 2.0.0](https://semver.org/). Gate = `test [akar-core]`.

## [Unreleased]

### Added

- GDS table functions membaca graf katalog nyata — closure menerima `Option<&dyn GraphDataSource>` lewat `TableFunction::CustomTableWithGraph` + `CatalogGraphSource` (P52.46) — `0290a8c`.

### Changed

- **P51.47 (perf)** — plan-cache hit kini berbagi `Arc<BoundStatement>`/`Arc<Vec<LogicalOperator>>` sehingga tak ada deep-clone operator tree per query; handler processor (sequence, schema-DDL, query, subquery, standalone-call registry) dibangun sekali per Connection dan di-share via `OnceLock` — `create_processor` tak lagi rebuild ~27 handler Arc tiap query. Handler dicache di `Connection` (bukan `Database`) agar `Arc<Database>` kuat di dalam handler tak membentuk reference cycle yang mengunci file lock selamanya. Validasi: `test_plan_cache_no_hit_regression` isolasi ratio hit/miss **0.914** (full-suite 0.950) — `340dbd0` — gate **1,647 (1,642 passed)**.

### Fixed

- **Batch bug P51.28–P51.36** — `DROP TABLE` kini hanya menghapus sequence milik tabel itu sendiri (enumerasi kolom SERIAL exact-match menggantikan prefix-filter `{name}_*_serial` yang menghapus sequence tabel ber-prefix sama, P51.28); commit tulis jadi two-phase: `prepare_commit` (validasi OCC + assign commit_ts + deregister agar checkpoint drain tak menunggu txn sendiri) → durable pipeline (`WAL` → `persist_all_tables`) → `finish_commit` (publish commit_history + release lock); `commit_write_txn` di-reorder mengikuti urutan itu (P51.29); UNION kini menjalankan kedua sisi dengan snapshot MVCC (`with_snapshot`) sehingga tak mencampur baris committed/uncommitted (P51.30); prepared statement diperluas: koleksi + substitusi parameter di pattern properties (MATCH/OPTIONAL MATCH/CREATE), ORDER BY, SET, DELETE, UNWIND, FOREACH sub-statements, CREATE DML, dan MERGE (termasuk ON CREATE/ON MATCH) (P51.31); `LIMIT`/`SKIP` non-u64 (negatif/overflow) kini error tegas, bukan silent-drop (P51.31); `value_to_constant` → `Result<Constant, String>` — UInt64/Int128 overflow error, tipe BLOB ditolak, tak lagi fallback `Constant::Null` (P51.32); CLI: completer aman multibyte (`floor_char_boundary`), Double/Float ditampilkan presisi penuh (bukan `{:.4}`), JSON escape lengkap (`\`, `\n`, `\r`, `\t`, kontrol <0x20) (P51.35); WASM: konversi JS number ke i64 dibatasi rentang pasti, `Database::new` memory-only di browser dengan pesan ramah (P51.36) — 9 tes baru (4 integrasi, 3 parser, 2 unit) — `f901425` — gate **1,647 (1,642 passed)**.
- **P51.17** — HNSW `insert` kini menghormati `_id` sebagai node id: `HnswIndex.nodes` diganti `BTreeMap<usize, HnswNode>` (id sparse diizinkan) sehingga hasil `search` selalu mengembalikan row id asli — sebelumnya node index posisional dipakai sebagai row id dan rusak bila baris NULL/non-vector di-skip saat populate (lookup lalu membaca baris yang salah). `VectorIndexTable::save` menulis node id sebenarnya agar id sparse tahan persist/load (file format lama tetap terbaca). 2 tes baru (`test_insert_honors_sparse_ids`, `test_vector_index_roundtrip_preserves_ids`) — `d5b13c1` — gate **1,638 (1,632 passed)**.
- **Batch G P52 (rekonsiliasi audit)** — art_index: `num_entries` kini ter-increment saat dup-key insert, `serialized_tree_size` dihitung tanpa materialisasi (anti-OOM), prefilter range pakai `upper_inclusive` (P52.51); wal_replayer: first-pass dead dihapus + batch data di-discard saat Rollback (P52.52); checkpoint drain gate kini di-lock `begin_read`/`begin_write` (P52.53); UnwindDedup key (variable, expression) + hanya dedup berurutan (P52.54); LimitPushDown guard ORDER BY/top-k/aggregate (P52.55); hasil agregat punya `field_names` (P52.56); graph output_writer ikuti predecessor chain + depth-guard + dst benar (P52.57); scan Int128/InternalID tak lagi NULL diam-diam (P52.58); akar-llm embedding non-numeric → error & arg override provider/model aktif (P52.59); httpfs `http_get` body dibatasi 64 MiB (P52.60); FFI akar-c ekspor `error_message` via out-param + `akar_error_message_free` (P52.61) — `ef4f143` — gate **1,636 (1,631 passed)**.
- **Batch F P52** — akar-json: doc header dikoreksi (fungsi yang benar-benar ada saja), `json_extract` path hilang → NULL bukan error (query abort), `json_structure` inspeksi semua elemen array (bukan `arr[0]`), depth-guard pada `structure_inner`/`contains_inner` (P52.49); `SET spill_threshold=0` kini benar-benar menonaktifkan spilling (bukan fallback ke default) & `CALL current_setting('concurrent_writes')` baca toggle runtime live (P52.50) — `ca28ffd` — gate **1,628 (1,623 passed)**.
- **Batch 5 P51** — optimizer: DP join reorder dibatasi per-segmen pipeline (tak menyebrangi batas WITH/ORDER BY/LIMIT, P51.23); kondisi join diekstrak dari konjungsi AND di WHERE (P51.24); constant-folding float pakai `==` exact (bukan epsilon) agar selaras runtime (P51.25); aritmetika i64 pakai checked ops agar tak panic/wrap saat planning (P51.26) — `5050b21` — gate **1,619 (1,614 passed)**.
- **Batch HIGH P51** — remote client race dikunci satu mutex (write+read, P51.9); prepared-writes kini ber-OCC (P51.10, tercakup P52.18); CLI script-mode terdeteksi via `stdin.is_terminal()` bukan env/`cfg!(windows)` (P51.13); Delta `load_delta_table` me-replay semua versi log (P51.15); HNSW beam search dibatasi (heap, prune ef-th, search per-layer) — O(ef·M) bukan O(N²) (P51.16); FTS `tokenize()` kompilasi regex sekali via `LazyLock` (P51.18); `persist_all_tables` redundant per-statement dihapus (P51.27) — `3237ad7` — gate **1,619 (1,614 passed)**.
- **P52.18** — isolasi txn putus dari SQL write path: tulis kini SELALU dibungkus txn (mode single & concurrent writer); `txn_id` dialirkan ke operator tulis (insert/delete/set/merge/copyfrom/batchinsert) via `insert_row_with_txn`/`delete_row_with_txn` sehingga `VersionInfo`/`commit_history` terisi dan `is_row_visible` nyata (uncommitted insert/delete tak terlihat reader lain); undo ditangkap tiap op tulis + DDL inline (CreateDml/Merge) dan diterapkan saat rollback/konflik OCC — termasuk rel table (`rollback_transaction` di-extend) — `b35fdf4` — gate **1,619 (1,614 passed)**.
- Batch E: storage/execution/vector/graph + DML DELETE/SET row-id bug (P52.29/P52.31/P52.38–P52.45/P52.47–P52.48/P52.62) — `61b5e0e` — gate 1,607 (1,602 passed).
- Batch E/F: connectors + extension compile fixes — azure/httpfs/duckdb/postgres/iceberg/neo4j (P52.30/P52.32–P52.37/P52.66 + P51.19/P52.33/P52.34) — `7c601a5` — gate **1,619 (1,614 passed)**.

## [0.1.5] - 2026-08-11

### Changed

- Docs: mark release v0.1.5 — `a4380d3` (gate 1,594 hijau).

### Fixed

- CI: test `ice disk path` dibuat platform-independent — `0304a3c`.

## [0.1.4] - 2026-08-10

### Changed

- Baseline final (worktree committed) — gate **1,594 hijau**.
- Rilis v0.1.4: sync backlog crates.io → HEAD bottom-up, 15 crate naik versi — `e158dd3`.

### Fixed

- Batch C: connection txn safety + EXPORT/IMPORT + remote drain (P52.12–P52.15/P52.19/P52.23–P52.28) — `e3c7b5d` — gate 1,581 hijau.
- Batch D: storage correctness (P52.16/P52.17/P52.20/P52.21/P52.22) — `2498794` — gate 1,588 hijau.

## [0.1.3] - 2026-08-10

### Fixed

- Batch 2: MERGE/CREATE multi-pattern + parser + binder cleanup (P51.7/P51.8/P51.37/P51.38) — `a2210e2` — gate 1,548 hijau.
- Batch 3: FFI panic containment + query-result destroy + server bounds (P51.11/P51.12/P51.33/P51.34) — `aca6520` — gate 1,551 hijau.
- Batch 4: connectors — injection, temp-file, all-rows, chunk-fill, DRY (P51.14/P51.20/P51.21/P51.22/P51.39) — `8a7f2df` — gate 1,569 hijau.
- Batch A: execution-engine silent-wrong-result (P52.1/P52.3/P52.8/P52.9/P52.10/P52.11) — `d62c55b` — gate 1,569 hijau.
- Batch B: optimizer passes; 4 → NO-OP terdokumentasi (P52.2/P52.4/P52.5/P52.6/P52.7) — `8564faa` — gate 1,578 hijau.

## [0.1.2] - 2026-08-09

### Changed

- Rilis v0.1.2 (4 crate) — `fa52bf2`.

### Fixed

- Batch 1: correctness SQL inti (P51.1–P51.6) — `9c173da` — gate 1,543 hijau.

---

**Status publish (2026-08-10):** semua 31 crate publishable live di crates.io — akar-storage & akar-main @ 0.1.3, akar-binder/planner/processor/postgres @ 0.1.2, akar-optimizer @ 0.1.2, akar-common/parser @ 0.1.1, akar-server/httpfs/duckdb/sqlite/azure/delta/iceberg/unity-catalog @ 0.1.1, sisanya @ 0.1.0 (rilis v0.1.4 sync backlog crates.io → HEAD, bottom-up, 2026-08-10); akar-c `publish=false`; repo GitHub **public**; tag **v0.1.1** + **v0.1.2** + **v0.1.3** + **v0.1.4** + **v0.1.5** + GitHub Releases live.

[Unreleased]: https://github.com/mlengse/akar/compare/v0.1.5...HEAD
[0.1.5]: https://github.com/mlengse/akar/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/mlengse/akar/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/mlengse/akar/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/mlengse/akar/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/mlengse/akar/releases/tag/v0.1.1
