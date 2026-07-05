# Plan: Kuzu Rust — Fase Crash Recovery + Storage Foundation

**TL;DR:** Port 5 komponen kritis dari C++ `src/storage/` yang belum ada di Rust: Undo Buffer, WAL Replayer, Page Manager, FileHandle, dan StorageManager. Tanpa ini, database tidak durable (no crash recovery, no proper rollback, no page lifecycle). Target: 17-24 jam.

---

## Fase A: Undo Buffer & Rollback Safety

### A1. Undo Buffer (`kuzu-storage/src/undo_buffer.rs` — new file)
Port dari `src/storage/undo_buffer.cpp`. UndoBuffer menyimpan snapshot data sebelum write, digunakan untuk rollback transaksi.

**Design:**
- `UndoBuffer` struct: `records: Vec<UndoRecord>`, `commit()` (clear), `rollback()` (apply reverse)
- `UndoRecord`: `table_id`, `row_id`, `column`, `old_data: Vec<u8>`
- `apply_undo()`: restore `old_data` ke storage untuk setiap record

**Reference:** `kuzu-transaction/src/lib.rs` already defines `UndoRecord` — reuse that type.

**Files to create:**
- `kuzu-storage/src/undo_buffer.rs` — UndoBuffer struct + apply logic

**Files to modify:**
- `kuzu-storage/src/lib.rs` — add `pub mod undo_buffer;`, export `UndoBuffer`
- `kuzu-transaction/src/lib.rs` — wire `UndoBuffer` into `Transaction::rollback()` flow

### A2. WAL Replayer (`kuzu-storage/src/wal_replayer.rs` — new file)
Port dari `src/storage/wal/wal_replayer.cpp` + Ladybug `ladybug/src/storage/wal/records/`.

**Design:**
- `WALReplayer` struct with `replay(wal_path, storage_manager)` method
- Reads WAL file sequentially, deserializes records, applies each to StorageManager
- Extend `WALRecord` enum with DDL variants: `CreateTable`, `DropTable`, `AlterTable`, `CreateIndex`, `DropIndex`, `CreateSequence`
- Replay order: Insert → Update → Delete → DDL; skip rolled-back transactions

**Reference:** `kuzu-storage/src/wal.rs` — existing `WALRecord` enum (8 variants), extend it.

**Files to create:**
- `kuzu-storage/src/wal_replayer.rs` — WALReplayer struct, record deserialization, replay dispatch

**Files to modify:**
- `kuzu-storage/src/wal.rs` — add DDL variants to `WALRecord` enum, add serialization format
- `kuzu-storage/src/lib.rs` — add `pub mod wal_replayer;`

---

## Fase B: Page Management & File I/O

### B1. Page Manager (`kuzu-storage/src/page_manager.rs` — new file)
Port dari `src/storage/page_manager.cpp`. Mengelola alokasi/dealokasi halaman via FreeSpaceManager.

**Design:**
- `PageManager` struct wraps `FreeSpaceManager` + `FileHandle`
- `allocate_page() -> u64` — allocate new page, returns page index
- `free_page(page_idx)` — mark page as free
- Internal: on allocation, try `FreeSpaceManager::pop_free_pages(1)` first; if none, extend file

**Reference:** `kuzu-storage/src/free_space_manager.rs` — already has `FreeSpaceManager` with `add_free_pages`, `pop_free_pages`.

**Files to create:**
- `kuzu-storage/src/page_manager.rs` — PageManager struct

**Files to modify:**
- `kuzu-storage/src/lib.rs` — add `pub mod page_manager;`, export `PageManager`

### B2. FileHandle (`kuzu-storage/src/file_handle.rs` — new file)
Port dari `src/storage/file_handle.cpp`. Abstraksi page I/O — membaca/menulis halaman ke file.

**Design:**
- `FileHandle` struct: `file: File`, `page_size: usize`, `num_pages: AtomicU64`
- `read_page(page_idx) -> Vec<u8>` — seek to `page_idx * page_size`, read `page_size` bytes
- `write_page(page_idx, data)` — seek + write
- `extend_file(num_pages)` — grow file by `num_pages * page_size`
- Thread-safe via internal `Mutex<File>` or OS-level pread/pwrite

**Reference:** `kuzu-storage/src/page.rs` — existing `Page` struct (4KB), use same page size constant.

**Files to create:**
- `kuzu-storage/src/file_handle.rs` — FileHandle struct

**Files to modify:**
- `kuzu-storage/src/lib.rs` — add `pub mod file_handle;`, export `FileHandle`

---

## Fase C: StorageManager Orchestrator

### C1. StorageManager Enhancement (`kuzu-storage/src/lib.rs` — extend existing)
Port dari `src/storage/storage_manager.cpp`. Sudah ada skeleton `StorageManager` di `kuzu-storage/src/lib.rs`, perlu diperkuat.

**Design:**
- Wire `PageManager`, `FileHandle`, `UndoBuffer`, `WALReplayer` ke `StorageManager`
- `StorageManager::open(db_path)` — buka database: baca header, replay WAL jika perlu, init BufferManager dengan FileHandle
- `StorageManager::commit_transaction(txn)` — full commit pipeline:
  1. Append commit record to WAL
  2. Flush WAL to disk
  3. Apply LocalStorage → tables (iterate modified_tables, write column chunks)
  4. Apply ShadowFile → BufferManager (evict dirty pages)
  5. Checkpoint if WAL exceeds threshold
- `StorageManager::rollback_transaction(txn)` — apply UndoBuffer, release pages
- Database header: magic bytes, version, page size, last checkpoint offset

**Reference:** `kuzu-storage/src/checkpoint.rs` — existing `checkpoint()` function; `kuzu-storage/src/shadow_file.rs` — ShadowFile structure.

**Files to modify:**
- `kuzu-storage/src/lib.rs` — extend `StorageManager` struct with new fields, implement `open()`, `commit_transaction()`, `rollback_transaction()`
- `kuzu-storage/src/version_info.rs` — add `StorageVersionInfo` struct (port from `storage_version_info.cpp`)

---

## Fase D: Integration & Wiring

### D1. Wire into Connection
Update `kuzu-main/src/connection.rs` to use the new StorageManager APIs.

**Changes:**
- Database initialization: call `StorageManager::open()` instead of manual setup
- `commit_write_txn()`: delegate to `StorageManager::commit_transaction()`
- `rollback_write_txn()`: delegate to `StorageManager::rollback_transaction()`
- Remove manual WAL flush, LocalStorage flush, ShadowFile apply from Connection

### D2. Wire into TransactionManager
Update `kuzu-transaction/src/lib.rs` to integrate UndoBuffer.

**Changes:**
- `Transaction::begin_write()`: create UndoBuffer
- On data modification: record undo before write (via `record_undo()`)
- `TransactionManager::rollback()`: apply undo records via StorageManager

---

## Relevant Files

### New files to create:
- `kuzu-storage/src/undo_buffer.rs` — UndoBuffer + apply logic (A1)
- `kuzu-storage/src/wal_replayer.rs` — WAL replay engine (A2)
- `kuzu-storage/src/page_manager.rs` — Page allocation/deallocation (B1)
- `kuzu-storage/src/file_handle.rs` — Page-level file I/O (B2)

### Files to modify:
- `kuzu-storage/src/lib.rs` — extend StorageManager, add new modules, exports (C1)
- `kuzu-storage/src/wal.rs` — add DDL WALRecord variants (A2)
- `kuzu-storage/src/version_info.rs` — add StorageVersionInfo (C1)
- `kuzu-transaction/src/lib.rs` — wire UndoBuffer into rollback (A1, D2)
- `kuzu-main/src/connection.rs` — use StorageManager APIs (D1)
- `kuzu-main/src/database.rs` — StorageManager::open() on init (D1)

### Reference files (read for patterns):
- `kuzu-storage/src/free_space_manager.rs` — FSM API used by PageManager
- `kuzu-storage/src/page.rs` — PAGE_SIZE constant, Page struct
- `kuzu-storage/src/buffer_manager.rs` — BufferManager API
- `kuzu-storage/src/checkpoint.rs` — checkpoint() function signature
- `kuzu-storage/src/shadow_file.rs` — ShadowFile API
- `kuzu-main/src/connection.rs` — current commit/rollback flow (lines ~60-140)
- `src/storage/undo_buffer.cpp` — C++ reference for UndoBuffer
- `src/storage/wal_replayer.cpp` — C++ reference for WAL replay
- `src/storage/page_manager.cpp` — C++ reference for PageManager
- `src/storage/file_handle.cpp` — C++ reference for FileHandle

---

## Verification

### Per-fase:
1. **A1 (Undo Buffer):** Unit test — write row → record undo → rollback → verify row reverted
2. **A2 (WAL Replayer):** Unit test — create WAL with known records → crash → reopen → verify data restored
3. **B1 (Page Manager):** Unit test — allocate 10 pages → free 3 → allocate again → verify reuse
4. **B2 (FileHandle):** Unit test — write page → read page → verify roundtrip
5. **C1 (StorageManager):** Integration test — create table → insert rows → commit → crash → reopen → verify rows intact

### End-to-end:
6. `cargo test --workspace` — all 898 existing tests still pass (no regressions)
7. `cargo clippy --workspace` — 0 warnings on new code
8. Manual: `cargo run -p kuzu-cli` → CREATE TABLE → INSERT → crash (kill -9) → reopen → MATCH returns rows

---

## Decisions

- **UndoBuffer scope:** Per-transaction, cleared on commit. Not persisted (WAL handles crash recovery, undo handles in-flight rollback).
- **WALRecord DDL variants:** Add 5 new variants (CreateTable, DropTable, AlterTable, CreateIndex, DropIndex) to match Ladybug's approach but simpler — no type-spec code generation, just manual serialization.
- **Page size:** 4096 bytes (matching existing `kuzu-storage/src/page.rs` constant).
- **FileHandle thread safety:** Use OS-level `pread`/`pwrite` (via `std::os::unix::fs::FileExt` on Linux, `std::os::windows::fs::FileExt` on Windows) for lock-free concurrent I/O. Fallback to `Mutex<File>` + seek on platforms without positional I/O.
- **WAL replay on open:** Automatic — `StorageManager::open()` always replays WAL if it exists and is non-empty. This matches C++ behavior.
- **Out of scope for this phase:** DiskArray, column type specializations (string/list/struct columns), dictionary encoding, HyperLogLog, Ladybug-specific optimizer passes. These come in later phases.

---

## Execution Order (sequential — each depends on prior)

```
A1 (Undo Buffer) ──┐
                    ├──> C1 (StorageManager) ──> D1 (Connection wiring) ──> D2 (TxnManager wiring)
A2 (WAL Replayer) ─┤
                    │
B1 (Page Manager) ──┤
                    │
B2 (FileHandle) ────┘
```

- A1, A2, B1, B2 are independent of each other — can be done in parallel
- C1 depends on A1 + A2 + B1 + B2
- D1 depends on C1
- D2 depends on A1 + C1

---

## Follow-up Phases (not in this plan)

- **Fase P1:** 12 table functions (show_tables, table_info, show_functions, show_indexes, etc.) — 12-16 jam
- **Fase P2:** TransactionContext (AUTO/MANUAL), checkpoint worker, conflict detection — 13-18 jam
- **Fase P3:** Full HashJoin/Aggregate pipelines, DDL operators, COPY pipeline — 28-39 jam
- **Fase P4:** Ladybug optimizer passes (OrderByPushDown, UnwindDedup, CountRelTable), ANALYZE, PERCENTILE — 11-16 jam
- **Fase P5:** CI/CD, PlanPrinter, ClientContext, column specializations — 18-25 jam
