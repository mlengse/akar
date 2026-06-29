# Plan: Concurrent Multi-Writer Transaction Model (Full Scope)

> **STATUS: ✅ SELESAI — 2026-06-30**
> Semua Phase A (A1-A9), Phase B (B1-B8), dan Phase C (C1-C5) telah diimplementasikan.
> Lihat `kuzu-transaction/src/lib.rs`, `kuzu-storage/src/`, dan `kuzu-main/src/connection.rs`.
> Perubahan utama: `TransactionManager` dengan checkpoint drain, `Connection::query()` dengan txn lifecycle,
> MVCC version chains, LocalWAL, DashMap TableCatalog, background auto-checkpoint worker.
> Test: `cargo test --workspace` ✅ (300+ tests), `cargo clippy --workspace` ✅.

## TL;DR

Refactor Kuzu Core Rust from **Single-Writer Constraint** (`max_concurrent_writers: 1` with fail-fast) to **Concurrent Multi-Writer** matching the Vela Partners C++ fork. All FASE 1-3 in one cycle. Default mode: **`concurrent_writes=true`** (unlimited writers). Uses **`dashmap`** for lock-free TableCatalog reads, per-transaction `LocalWAL`, MVCC version chains on `ColumnChunk`, and two-phase checkpoint drain.

**Approach**: Direct port of Vela C++ architecture, adapted to Rust idioms. One complete cycle covering all three phases.

---

## PHASE A: Writer Admission + Lock-Free Catalog Infrastructure

Removes single-writer restriction, adds block/wait-based admission, replaces `Mutex<TableCatalog>` with `DashMap`-based lock-free concurrent catalog.

| Step | File | Perubahan |
|------|------|-----------|
| **A1** | `kuzu-transaction/src/lib.rs` | Tambah `concurrent_writes: AtomicBool` (default `true`). Hapus `max_concurrent_writers`. `allow_concurrent_writes()` return `concurrent_writes.load()`. |
| **A2** | `kuzu-main/src/database.rs` | `SystemConfig` — tambah `concurrent_writes: bool` (default `true`). Ekspos via `SET concurrent_writes` Cypher. |
| **A3** | `kuzu-transaction/src/lib.rs` | Refactor `begin_write()` — hapus pengecekan writer count ketika multi-writer. Di single-writer mode, block via `Condvar`. `CommitResult` — hapus `Aborted`, pakai blocking. |
| **A4** | `kuzu-storage/src/local_wal.rs` **(NEW)** | Port `LocalWAL` — per-transaction in-memory WAL buffer. Method: `log_insert`, `log_delete`, `log_update`, `log_begin_tx`, `log_commit`. `flush_to(writer)` — serialize + bulk copy. |
| **A5** | `kuzu-storage/src/wal.rs` | Method `log_committed_wal(&LocalWAL)` — lock `self.mtx`, bulk-copy LocalWAL buffer ke file global, flush+sync. |
| **A6** | `kuzu-storage/Cargo.toml` | Add `dashmap = "6"` dependency. |
| **A7** | `kuzu-storage/src/table.rs` | Replace `HashMap<u64, NodeTable>` + `Mutex<TableCatalog>` with **`DashMap<u64, NodeTable>`** + **`DashMap<u64, RelTable>`** + `EvMap` untuk name→id. `TableCatalog` jadi lock-free untuk reads. |
| **A8** | `kuzu-storage/src/lib.rs` | `StorageManager.table_catalog` jadi `Arc<TableCatalog>` (tanpa Mutex/RwLock — DashMap internal lock-free). `table_catalog()` return `Arc<TableCatalog>`. |
| **A9** | `kuzu-processor/src/processor.rs`, `kuzu-main/src/connection.rs`, `kuzu-main/src/database.rs` | Update semua caller — ganti `.lock().unwrap()` dengan direct access ke DashMap methods. |

---

## PHASE B: MVCC Version Storage + Transaction Pipeline Wiring

Menambahkan version chains (VersionInfo/UpdateInfo) dan integrasi transaction lifecycle ke `Connection::query()`.

| Step | File | Perubahan |
|------|------|-----------|
| **B1** | `kuzu-storage/src/version_info.rs` **(NEW)** | `VersionInfo` — tracking insert/delete visibility per vector. `VectorVersionInfo { inserted: DashSet<row_idx>, deleted: DashSet<row_idx> }`. Method: `insert(txn, row)`, `delete(txn, row)`, `is_visible(txn, snap_ts, commit_history)`. |
| **B2** | `kuzu-storage/src/update_info.rs` **(NEW)** | `UpdateInfo` — MVCC version chain. `VectorUpdateInfo { version: txn_t, data: Vec<u8>, prev: Option<Box<...>> }`. Lock-free append via `AtomicPtr` atau `Mutex` ringan. `get_version(snapshot_ts)` — traverse chain. |
| **B3** | `kuzu-storage/src/node_group.rs` | Integrasi `VersionInfo`. `append_row()` catat insert. `get_value()` cek visibility. |
| **B4** | `kuzu-storage/src/column_chunk.rs` | Integrasi `UpdateInfo`. `set_value()` preserve old data via VectorUpdateInfo. `get_value()` cek version chain. |
| **B5** | `kuzu-storage/src/table.rs` | `scan_column()` & `get_value()` terima `snapshot_ts: Option<u64>`. Default: latest committed. Cek VersionInfo + UpdateInfo. |
| **B6** | `kuzu-transaction/src/lib.rs` | Tambah `local_storage` dan `local_wal` ke `Transaction` struct. Per-txn `UndoRecords` diperluas. |
| **B7** | `kuzu-main/src/connection.rs` | **Integrasi transaction lifecycle**: `query()` → `begin_write()` → execute (write ke LocalStorage + LocalWAL) → `commit()` via `StorageManager::commit_transaction()` → rollback on error. |
| **B8** | `kuzu-transaction/src/lib.rs` | `commit()` baru: panggil `transaction.commit(&wal)` — flush LocalStorage → UndoBuffer commit → LocalWAL logCommit → WAL::log_committed_wal. |

---

## PHASE C: Checkpoint Drain + Background Auto-Checkpoint

Port dua-phase drain dan background worker dari Vela.

| Step | File | Perubahan |
|------|------|-----------|
| **C1** | `kuzu-transaction/src/lib.rs` | Tambah: `mtx_for_starting_new_txns: Mutex<()>`, `mtx_for_checkpoint: Mutex<()>`, `cv_active_txns_changed: Condvar`, `active_txn_count: AtomicU32`. Implementasi `stop_new_txns_and_wait_until_all_leave(timeout)` — dua-phase gate. |
| **C2** | `kuzu-storage/src/lib.rs` | `checkpoint_with_drain()` — panggil drain TM dulu, baru lanjut ke ShadowFile→BM→WAL flow. |
| **C3** | `kuzu-transaction/src/lib.rs` | Background thread `auto_checkpoint_worker`. Signal via `schedule_auto_checkpoint()`. Shutdown cleanly via `Drop`. |
| **C4** | `kuzu-main/src/database.rs`, `connection.rs` | Ganti inline `maybe_auto_checkpoint()` dengan signal ke background worker. Threshold-based. |
| **C5** | `kuzu-main/src/connection.rs`, `database.rs` | Ekspos `BEGIN TRANSACTION`, `COMMIT`, `ROLLBACK` Cypher ke user. |

---

## Diagram Alur Commit (Setelah Refactor)

```mermaid
flowchart TD
    A[Connection::query WRITE] --> B[TM::begin_write]
    B --> C[Transaction.create: LocalStorage + LocalWAL + UndoBuffer]
    C --> D[Execute: writes buffered ke LocalStorage + LocalWAL]
    D --> E[TM::commit / Connection::commit]
    E --> F[Transaction::commit]
    F --> G[LocalStorage.flush_to_tables via DashMap]
    G --> H[UndoBuffer::commit (assign commitTS)]
    H --> I[LocalWAL::logCommit]
    I --> J[WAL::log_committed_wal (bulk-copy ke global WAL)]
    J --> K[TM::clear_transaction + decrement counters + notify condvar]
    K --> L{Checkpoint needed?}
    L -->|Yes| M[schedule_auto_checkpoint → background worker]
    L -->|No| N[Done]
    
    M --> O[acquire mtx_for_checkpoint]
    O --> P[stop_new_txns + drain active txns]
    P --> Q[ShadowFile::apply → BM]
    Q --> R[WAL::logAndFlushCheckpoint]
    R --> S[Release mtx_for_starting_new_txns]
```

## Relevant Files

| File | Perubahan |
|------|-----------|
| `kuzu-core/kuzu-transaction/src/lib.rs` | **A1, A3, B6, B8, C1, C3** — main refactor |
| `kuzu-core/kuzu-storage/src/local_wal.rs` | **A4** — NEW: per-transaction LocalWAL |
| `kuzu-core/kuzu-storage/src/wal.rs` | **A5** — WAL::log_committed_wal |
| `kuzu-core/kuzu-storage/Cargo.toml` | **A6** — add dashmap dependency |
| `kuzu-core/kuzu-storage/src/table.rs` | **A7, B5** — DashMap TableCatalog + MVCC reads |
| `kuzu-core/kuzu-storage/src/lib.rs` | **A8, C2** — Arc<TableCatalog>, checkpoint_with_drain |
| `kuzu-core/kuzu-storage/src/version_info.rs` | **B1** — NEW: MVCC insert/delete tracking |
| `kuzu-core/kuzu-storage/src/update_info.rs` | **B2** — NEW: MVCC update version chains |
| `kuzu-core/kuzu-storage/src/node_group.rs` | **B3** — VersionInfo integration |
| `kuzu-core/kuzu-storage/src/column_chunk.rs` | **B4** — UpdateInfo integration |
| `kuzu-core/kuzu-storage/src/local_storage.rs` | **B7** — minor: flush via DashMap |
| `kuzu-core/kuzu-processor/src/processor.rs` | **A9** — direct DashMap reads |
| `kuzu-core/kuzu-main/src/connection.rs` | **A9, B7, C4, C5** — transaction lifecycle, CHECKPOINT/BEGIN/COMMIT |
| `kuzu-core/kuzu-main/src/database.rs` | **A2, C4, C5** — SystemConfig + config exposure |
| `kuzu-core/kuzu-storage/src/shadow_file.rs` | Unchanged (already per-txn) |

## Verification

1. **PHASE A**: `cargo build --workspace` passes; 2 threads `begin_write()` concurrently both succeed; `LocalWAL` buffers correctly; `DashMap<TableCatalog>` supports concurrent reads without locking
2. **PHASE B**: Concurrent inserts to same table from 2 threads succeed; concurrent updates to same row via MVCC version chains; reader sees snapshot-consistent view during concurrent writes
3. **PHASE C**: Checkpoint drain waits for all active transactions; background auto-checkpoint works under concurrent writes; crash + recovery with concurrent writes is correct
4. **Integration test port**: Port `DefaultConcurrentAgentMemoryUnderAutoCheckpoint` from Vela C++ — 4 concurrent writers + reader + auto-checkpoint
5. **Regression**: All existing single-writer tests still pass (concurrent_writes boleh diset false)

## Scope Boundaries

| Included | Excluded (Deferred) |
|---|---|
| Concurrent writer admission (block/wait, no hard cap) | Page-level locking in BufferManager (keep global Mutex) |
| `DashMap`-based lock-free TableCatalog | ART Index (separate phase) |
| Per-transaction `LocalWAL` + bulk merge ke global WAL | HNSW vector integration (separate phase) |
| MVCC version chains (`VersionInfo` + `UpdateInfo`) | Disk spilling / stream-merge (separate phase) |
| Two-phase checkpoint drain + background auto-checkpoint | Distributed transactions |
| `BEGIN`/`COMMIT`/`ROLLBACK` Cypher commands | Full serializable isolation (snapshot cukup) |
| Transaction lifecycle di `Connection::query()` | |

## Key Design Decisions

1. **`dashmap` instead of `RwLock<TableCatalog>`** — lock-free reads, concurrent writes ke tabel berbeda tanpa contention. Sesuai preferensi user.
2. **Block/wait instead of fail-fast** — via `Condvar`, lebih aplikasi-friendly.
3. **`concurrent_writes=true` sebagai default** — mengikuti Vela C++.
4. **Per-transaction LocalWAL** — langsung port dari Vela, hindari kontensi WAL selama write.
5. **MVCC version chains di ColumnChunk** — selaras dengan Vela yang pakai `VectorUpdateInfo` per 1024 rows.
6. **Snapshot isolation** — sesuai Vela; serializable bisa ditambah kemudian via SSI.
7. **Transaction lifecycle di `Connection::query()`** — integration gap utama yang diisi.
