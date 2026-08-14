# Changelog — Akar

> Riwayat revisi Sprint 19 (P51/P52), dipisah dari `implementation plan.md` agar plan hanya berisi pekerjaan yang belum dikerjakan. Fase selesai P1–P50 + AUDIT → [`SPEC.md`](SPEC.md) & git history.
> Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) · Versi: [SemVer 2.0.0](https://semver.org/). Gate = `test [akar-core]`.

## [Unreleased]

### Added

- **Batch A+B akar-python (drop-in KuzuDB, P53.1–P53.8)** — translation layer `translation.rs` + `param_interp.rs` + executor `lib.rs`: DDL idempoten — `CREATE NODE/REL TABLE IF NOT EXISTS` → cek `CALL show_tables()` lalu skip/create; `DROP TABLE IF EXISTS` → swallow "not found" (P53.1); `FLOAT[n]` → `FLOAT[]` + registri dims dari DDL (P53.2); `ALTER TABLE ... ADD col TYPE DEFAULT <lit>` → strip DEFAULT + swallow "column already exists" (P53.3); `CALL CREATE/DROP/QUERY_VECTOR_INDEX` → SQL Akar (brute-force MATCH + ORDER BY ekspresi cosine penuh, workaround limit-alias) (P53.4); syntax Kuzu `CREATE VECTOR INDEX IF NOT EXISTS ... FOR/ON/OPTIONS` (P53.5); multi-statement split + `INSTALL`/`LOAD`/`UNINSTALL EXTENSION` no-op (P53.6); interpolasi `$param` → literal ter-escape (string/int/float/bool/list/dict/null, validasi nama `[A-Za-z_][A-Za-z0-9_]*`) (P53.7); `RETURN node` → dict via registri schema + fallback `table_info` (P53.8). Infra tes: pyo3 `auto-initialize` (memungkinkan `cargo test`), `rustfmt.toml` 120-kolom + `cargo fmt`, tes integrasi end-to-end `kairos_ensure_schema_bootstrap_is_idempotent` (bootstrap `_ensure_schema` 2× idempoten + DROP IF EXISTS + ALTER ADD DEFAULT). — `b93e966` — gate **1,649 (1,649 passed)**.
- GDS table functions membaca graf katalog nyata — closure menerima `Option<&dyn GraphDataSource>` lewat `TableFunction::CustomTableWithGraph` + `CatalogGraphSource` (P52.46) — `0290a8c`.

### Removed

- **5 doc-test `ignore` blok** — snippet ilustratif yang tak bisa di-compile standalone dihapus dari doc comment (`akar-storage/column_chunk.rs`, `lazy_scanner.rs`, `spiller.rs` ×2, `akar-main/prepared_statement.rs`) agar tak tercatat sebagai "ignored" test yang membingungkan; tersisa 8 doc-test lulus (0 ignored). Bonus: `test_migration_ingestion` kini jalankan binary `akar-migrate` via `env!("CARGO_BIN_EXE_akar-migrate")` (bukan nested `cargo run` yang memicu rebuild). — `6d6cded` — gate **1,648 (1,648 passed)**.

### Changed

- **P51.47 (perf)** — plan-cache hit kini berbagi `Arc<BoundStatement>`/`Arc<Vec<LogicalOperator>>` sehingga tak ada deep-clone operator tree per query; handler processor (sequence, schema-DDL, query, subquery, standalone-call registry) dibangun sekali per Connection dan di-share via `OnceLock` — `create_processor` tak lagi rebuild ~27 handler Arc tiap query. Handler dicache di `Connection` (bukan `Database`) agar `Arc<Database>` kuat di dalam handler tak membentuk reference cycle yang mengunci file lock selamanya. Validasi: `test_plan_cache_no_hit_regression` isolasi ratio hit/miss **0.914** (full-suite 0.950) — `340dbd0` — gate **1,647 (1,642 passed)**.

### Fixed

- **P53.1 blocker DDL `BOOLEAN`** — `primitive_type` grammar mencoba `"BOOL"` sebelum `"BOOLEAN"` (pest alternatif berurutan tanpa longest-match) → `protected BOOLEAN` (kairos `_ensure_schema`) ter-potong jadi `BOOL`+`EAN`, gagal parse. Urutan dibalik: `"BOOLEAN"` sebelum `"BOOL"`. Regression test `test_create_node_table_boolean_types`. — `b93e966` — gate **1,649 (1,649 passed)**.
- **P53.12 (complex-type query output)** — kolom & literal List/Struct/Map kini round-trip melewati pipeline Arrow alih-alih kolaps NULL (4 lapis: `scan.rs` `logical_to_physical` memetakan List/Array/Map/Struct/Union ke `PhysicalTypeID::List`/`Struct`; `build_arrow_array` (scan) & `to_arrow_array` (storage) + `ArrowVector::from_legacy` + `build_arrow_from_values` (evaluator) membangun `ListArray`/`StructArray` via helper baru `arrow_array_from_values` yang menangani tipe kompleks/nested rekursif; proyeksi map/function dialihkan ke `evaluate_arrow` (bukan ValueVector yang tak punya side-storage); `evaluate_expression_for_row` & `evaluate_constant_expr` (write path CREATE/SET) mengevaluasi literal List/Map rekursif sehingga `FLOAT[]` tersimpan nyata; `DataChunk::get_value` punya arm List/Struct). Regression test feature-gated `test_vector_index.rs`: `RETURN n.embedding`→List, `RETURN {id:n.id}`→Struct, `array_cosine_similarity`→Double; `test_nested_types.rs` di-update (list/struct kini round-trip; `size()`, `list_concat`, nested list bekerja). Catatan: subscript `t.lst[i]` masih di-drop parser (gap terpisah). — `46de39d` — gate **1,653 (1,648 passed)**.
- **P53.13 (hang)** — `bind_create_vector_index` self-deadlock: guard `catalog` #1 tak di-drop sebelum `lock()` #2 (re-entrant `MutexGuard` = hang `WaitOnAddress`). `CREATE VECTOR INDEX` kini selalu ter-bind (sebelumnya hang untuk metric valid). Guard #1 di-scope. Regression test `test_vector_index.rs` (watchdog 15s). Audit 24 call-site `catalog.lock()` — hanya fungsi ini yang double-lock. — `24e9610` — gate **1,649 (1,644 passed)**.
- **P53.14 (flake FTS)** — `PhysicalCreateFtsIndex::execute` memegang DashMap `Ref` (`source_table`) sambil acquire write lock (`create_node_table`/`get_*_mut`) → read→write di shard sama = re-entrant deadlock; seed acak DashMap → tabrakan shard intermiten (flake "lolos isolasi, hang suite penuh"). Fix: snapshot schema+data dalam block, `Ref` di-drop sebelum write. Reproduksi load: hang sebelum fix, 10/10 lolos sesudah; gate penuh 3m42s bersih. — `24e9610` — gate **1,649 (1,644 passed)**.
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
