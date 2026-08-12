# Changelog — Akar

> Riwayat revisi Sprint 19 (P51/P52), dipisah dari `implementation plan.md` agar plan hanya berisi pekerjaan yang belum dikerjakan. Fase selesai P1–P50 + AUDIT → [`SPEC.md`](SPEC.md) & git history.
> Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) · Versi: [SemVer 2.0.0](https://semver.org/). Gate = `test [akar-core]`.

## [Unreleased]

### Added

- GDS table functions membaca graf katalog nyata — closure menerima `Option<&dyn GraphDataSource>` lewat `TableFunction::CustomTableWithGraph` + `CatalogGraphSource` (P52.46) — `0290a8c`.

### Fixed

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
