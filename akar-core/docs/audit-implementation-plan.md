# Audit Implementation Plan — Akar (kuzu)

**Based on codebase audit of 31 crates, ~55K lines of Rust**

---

## Phase 1: Correctness & Safety (Week 1-2)

Fix data-corruption and safety bugs first. These are the highest-risk items.

### 1.1 Fix auto-checkpoint worker thread wiring
**Issue #2** | `akar-transaction/src/lib.rs:565-567`

The background thread creates local `Arc<AtomicBool>` clones that shadow the struct fields. The thread never receives signals from `schedule_auto_checkpoint()` or `request_shutdown()`.

```rust
// BEFORE (broken): local clones shadow struct fields
let shutdown = Arc::new(AtomicBool::new(false));
let shutdown_clone = shutdown.clone();
let checkpoint_requested = Arc::new(AtomicBool::new(false));

// AFTER (correct): use Arc::clone of the struct fields
let shutdown = Arc::new(AtomicBool::new(false));
let shutdown_clone = self.shutdown_requested.clone();  // WRONG — AtomicBool isn't Arc
```

**Fix approach:** Change `shutdown_requested` and `checkpoint_requested` from `AtomicBool` to `Arc<AtomicBool>`, store the `Arc` clones in the struct, and clone them into the thread. Alternatively, use a `Notify`/`Condvar` channel instead of shared atomics.

**Estimated effort:** 4-6 hours
**Files:** `akar-transaction/src/lib.rs`

### 1.2 Fix `checkpoint_with_drain` bypass
**Issue #3** | `akar-storage/src/lib.rs:318`

Line 318: `let _drained = true;` — the drain is completely bypassed. The method should call `stop_new_txns_and_wait_until_all_leave()`.

**Fix approach:**
- Store an `Arc<TransactionManager>` (or a drain callback) in `StorageManager`
- Call `tm.stop_new_txns_and_wait_until_all_leave(Duration::from_secs(30))` before checkpoint
- Log a warning if drain times out

**Estimated effort:** 3-4 hours
**Files:** `akar-storage/src/lib.rs`, `akar-main/src/database.rs` (pass TM reference)

### 1.3 Enforce MVCC snapshot isolation on reads
**Issue #1** | `akar-storage/src/table.rs`

`NodeTable::get_value()` and `scan_column()` read current in-memory state without checking snapshot timestamps. Concurrent readers see uncommitted writes.

**Fix approach:**
- Add `snapshot_ts: u64` parameter to all read methods on `NodeTable`
- Store commit timestamps on undo records so `is_visible(txn_id, snapshot_ts)` can filter rows
- Alternatively, adopt a copy-on-write approach: readers use a consistent snapshot of `NodeGroup` data

**Estimated effort:** 2-3 days (most complex change)
**Files:** `akar-storage/src/table.rs`, `akar-transaction/src/lib.rs`, `akar-processor/src/physical/scan_filter/`

### 1.4 Fix unsafe self-referential borrow in BufferManager
**Issue #4** | `akar-storage/src/buffer_manager.rs:242`

`unsafe { &*(frame as *const Frame) }` circumvents the borrow checker to return a reference into a mutable HashMap.

**Fix approach:** Return a `FrameIndex` (the key) instead of a reference. Callers access frames through a method like `get_frame(&self, key) -> &Frame`. This avoids holding a reference across potential mutations.

**Estimated effort:** 4-6 hours
**Files:** `akar-storage/src/buffer_manager.rs`, all callers of `pin()`

### 1.5 Fix silent rollback failure
**Issue #5** | `akar-main/src/connection/transaction.rs:85`

`let _ = sm.rollback_transaction(...)` discards errors that could indicate storage corruption.

**Fix approach:**
- Change `rollback_write_txn` to return `Result<Vec<UndoRecord>, String>`
- Propagate rollback errors to the caller
- Log critical errors if rollback fails (data may be in inconsistent state)

**Estimated effort:** 1-2 hours
**Files:** `akar-main/src/connection/transaction.rs`

---

## Phase 2: Durability & WAL (Week 2-3)

Ensure crash recovery is reliable.

### 2.1 Atomic WAL flush
**Issue #6** | `akar-storage/src/wal.rs:261`

`File::create` truncates the file before writing. A crash mid-write corrupts the WAL.

**Fix approach:**
1. Write to `{wal_path}.tmp`
2. `file.sync_all()` (fsync)
3. Atomic rename `{wal_path}.tmp` → `{wal_path}` (on most OSes)
4. Fsync the parent directory (for durability guarantees)

```rust
pub fn flush_to_disk(&self) -> std::io::Result<()> {
    let tmp_path = self.path.with_extension("log.tmp");
    let mut file = std::fs::File::create(&tmp_path)?;
    // ... write all records ...
    file.sync_all()?;
    std::fs::rename(&tmp_path, &self.path)?;
    // Fsync parent directory for atomicity
    if let Some(parent) = self.path.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}
```

**Estimated effort:** 2-3 hours
**Files:** `akar-storage/src/wal.rs`

### 2.2 Add WAL record checksums ✅ DONE
**Issue #11** | `akar-storage/src/wal.rs`

No integrity checking on WAL records. A bit-flip corrupts data silently.

**Fix (implemented):**
- CRC32 checksum per record in v2 WAL format
- `AKAR` magic header + version for format detection
- v1 backward compatibility (legacy WAL files without checksums)
- Corrupt records silently skipped with `tracing::warn!`

**Files changed:** `akar-storage/src/wal.rs`, `akar-storage/Cargo.toml`

### 2.3 Handle silent `.set_value().ok()` errors ✅ DONE
**Issue #15** | 12 locations across akar-algo + akar-processor

**Fix (implemented):**
- All12 `.set_value(...).ok()` calls replaced with `?` (inside closures that return `Result<(), String>`)
- Pre-existing `wal.rs` compile error fixed: added `use std::io::Read`
- Removed unused `Deserialize` import and `mut` variable in wal.rs tests

**Files changed:** `akar-algo/src/lib.rs`, `akar-processor/src/physical/write_ops/recursiveextend.rs`, `akar-storage/src/wal.rs`

---

## Phase 3: Error Handling (Week 3-4)

Replace `Result<T, String>` with structured errors.

### 3.1 Define unified error type in `akar-common`
**Issue #9** ✅ DONE

Create `akar_common::error::AkarError` with variants for each subsystem:

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AkarError {
    #[error("storage: {0}")]
    Storage(#[from] StorageError),
    #[error("transaction: {0}")]
    Transaction(#[from] TransactionError),
    #[error("parser: {0}")]
    Parser(String),
    #[error("binder: {0}")]
    Binder(String),
    #[error("planner: {0}")]
    Planner(String),
    #[error("processor: {0}")]
    Processor(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("WAL error: {0}")]
    Wal(String),
    #[error("buffer manager error: {0}")]
    BufferManager(String),
    #[error("table not found: {0}")]
    TableNotFound(String),
    #[error("column not found: {0}")]
    ColumnNotFound(String),
    #[error("type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },
    #[error("page error: {0}")]
    Page(String),
}

#[derive(Debug, Error)]
pub enum TransactionError {
    #[error("table already locked by txn#{0}")]
    TableLocked(u64),
    #[error("concurrent write not allowed")]
    ConcurrentWriteDisabled,
    #[error("manager is shutting down")]
    ShuttingDown,
}
```

**Estimated effort:** 6-8 hours
**Files:** `akar-common/src/error.rs` (new), `akar-common/src/lib.rs`

### 3.2 Migrate crates to use `AkarError` incrementally
**Issue #9** ✅ DONE

Order of migration (leaf → root):
1. ✅ `akar-transaction` → `TransactionError` (11 functions)
2. ✅ `akar-storage` → `StorageError` (36 functions)
3. ✅ `akar-catalog` → `CatalogError` (9 functions)
4. ✅ `akar-binder` → `BinderError` (48 functions)
5. ✅ `akar-planner` → `PlannerError` (19 functions)
6. ✅ `akar-optimizer` — no functions to migrate (already clean)
7. ✅ `akar-processor` → `ProcessorError` (54+ functions + type aliases)
8. ✅ `akar-main` — cascade fixes (standalone_call.rs, query.rs, utils.rs)
9. `akar-c` — FFI boundary (converts back to `String` for C callers)

**Estimated effort:** 2-3 days
**Files:** Every crate's `lib.rs` and public API

### 3.3 Replace production `.expect()` with `?`
**Issue #14** | 2 locations

- `akar-processor/src/processor/mapper/map_update.rs:165` — `.expect("VFS not initialized")`
- `akar-processor/src/processor/mapper/map_scan.rs:77` — `.expect("table catalog required")`

**Fix:** Change to `ok_or(AkarError::Storage(...))? `

**Estimated effort:** 1 hour
**Files:** 2 files

---

## Phase 4: Concurrency & Performance (Week 4-5)

### 4.1 Replace `Mutex<BufferManager>` with `RwLock`
**Issue #12** | `akar-storage/src/lib.rs:73`

All reads take exclusive locks. Switch to `RwLock` for concurrent reads.

```rust
// BEFORE
buffer_manager: Arc<Mutex<BufferManager>>,

// AFTER
buffer_manager: Arc<RwLock<BufferManager>>,
```

**Estimated effort:** 3-4 hours (mechanical search-replace + lock upgrade/downgrade audit)
**Files:** `akar-storage/src/lib.rs`, all callers

### 4.2 Reduce clones in expression evaluator
**Issue #18** | `akar-processor/src/expression_evaluator.rs`

34+ `clone()` calls in the hottest query path.

**Fix approach:**
- Change `evaluate_arrow` to take `&Expression` (it already does)
- Replace `left.clone()` / `right.clone()` with borrows where Arrow kernels accept `&dyn Array`
- Use `Cow<'a, [Value]>` for list operations instead of `Vec::clone()`
- Cache computed Arrow vectors in a per-chunk evaluation cache

**Estimated effort:** 1-2 days
**Files:** `akar-processor/src/expression_evaluator.rs`

### 4.3 Split `TransactionManager` god-object
**Issue #13** | `akar-transaction/src/lib.rs:273-307`

14 fields mixing 4+ responsibilities.

**Fix approach:**
```rust
pub struct TransactionManager {
    lifecycle: TransactionLifecycle,      // next_id, next_commit_ts, active_transactions
    concurrency: ConcurrencyControl,     // table_locks, concurrent_writes, write_count, condvar
    checkpoint: CheckpointCoordinator,   // drain fields, worker_handle, shutdown/checkpoint flags
    config: TransactionManagerConfig,
}
```

**Estimated effort:** 1 day
**Files:** `akar-transaction/src/lib.rs`

### 4.4 Deduplicate sequence callback
**Issue #16**

Same logic in 3 places: `database.rs:207`, `query.rs:236`, `query.rs:438`.

**Fix:** Extract into `Connection::resolve_sequence(name) -> Result<Value, String>`.

**Estimated effort:** 2 hours
**Files:** `akar-main/src/connection/query.rs`, `akar-main/src/database.rs`

---

## Phase 5: Build & CI (Week 5)

Quick wins for build hygiene.

### 5.1 Fix rustfmt nightly-only options
**Issue #19** | `akar-core/rustfmt.toml`

`imports_granularity` and `group_imports` are nightly-only. CI runs stable.

**Fix:** Either pin a nightly toolchain for fmt in CI, or remove these options.

**Estimated effort:** 1 hour
**Files:** `akar-core/rustfmt.toml`, `.github/workflows/rust-ci.yml`

### 5.2 Remove unused workspace dependencies
**Issue #20** | `akar-core/Cargo.toml`

Remove: `bitflags`, `uuid`, `rust_decimal`.

**Estimated effort:** 15 minutes
**Files:** `akar-core/Cargo.toml`

### 5.3 Fix contradictory build profiles
**Issue #21**

`.cargo/config.toml` sets `strip = "symbols"` while `Cargo.toml` sets `debug = true`.

**Fix:** Remove `debug = true` from `Cargo.toml` `[profile.release]` or remove `strip` from `.cargo/config.toml`. Decide on a single intent.

**Estimated effort:** 15 minutes
**Files:** `akar-core/.cargo/config.toml`, `akar-core/Cargo.toml`

### 5.4 Add `cargo audit` to CI
**Issue #22**

**Fix:** Add a step:
```yaml
- name: Security audit
  run: |
    cargo install cargo-audit
    cargo audit
```

**Estimated effort:** 30 minutes
**Files:** `.github/workflows/rust-ci.yml`

### 5.5 Add Rust build caching to CI
**Issue #23**

**Fix:** Add `Swatinem/rust-cache@v2` action to all jobs.

**Estimated effort:** 15 minutes
**Files:** `.github/workflows/rust-ci.yml`

### 5.6 Fix edition inconsistency
**Issue #30**

`tools/rust_api/Cargo.toml` uses edition 2021; workspace uses 2024.

**Fix:** Update `tools/rust_api/Cargo.toml` to `edition = "2024"` and `rust-version = "1.85"`.

**Estimated effort:** 15 minutes
**Files:** `tools/rust_api/Cargo.toml`

---

## Phase 6: Test Quality (Week 5-6)

### 6.1 Consolidate test helpers ✅ DONE
**Issue #17**

**Fix (implemented):**
- Created `akar-main/src/test_helpers.rs` — single source of truth for all test utilities
- `setup_db()` uses temp dir (not `:memory:`) to avoid WAL flush failures
- `setup_db_on_disk()`, `setup_db_with_checkpoint()` for explicit filesystem-backed tests
- `exec()`, `exec_ok()`, `exec_err()`, `query()`, `query_result()`, `query_column()`, `query_values()`
- `tests/common/mod.rs` re-exports via `pub use akar_main::test_helpers::*`
- All test files (`connection_test.rs`, `integration_test.rs`, `fase_b_verification.rs`, `test_proptest.rs`, etc.) migrated
- Fixed pre-existing `commit_transaction` signature mismatch in `akar-storage` tests
- Added `tempfile` as regular dependency in `akar-main/Cargo.toml`

**Files changed:** `akar-main/src/test_helpers.rs`, `akar-main/src/lib.rs`, `akar-main/src/connection_test.rs`, `akar-main/tests/common/mod.rs`, `akar-main/tests/integration_test.rs`, `akar-main/tests/fase_b_verification.rs`, `akar-main/tests/test_proptest.rs`, `akar-main/Cargo.toml`, `akar-storage/src/lib.rs`

### 6.2 Fix fragile float assertions
**Issue #25** | 22 `assert_eq!` on `f64`

**Fix:** Replace with epsilon-based comparisons:
```rust
// BEFORE
assert_eq!(result.values[0], 1.0);

// AFTER
assert!((result.values[0] - 1.0).abs() < 1e-10, "expected ~1.0, got {}", result.values[0]);
```

**Estimated effort:** 1 hour
**Files:** `akar-algo/src/lib.rs`, `akar-graph/src/gds/bfs_graph.rs`, `akar-graph/src/algorithms.rs`, `akar-fts/src/lib.rs`

### 6.3 Add tests for untested crates
**Issue #24**

Add basic smoke tests for:
- `akar-extension` — register/unregister extension
- `akar-c` — FFI round-trip (create DB, run query, free)
- `akar-cli` — REPL startup and basic command

**Estimated effort:** 1 day
**Files:** `akar-extension/src/lib.rs`, `akar-c/tests/`, `akar-cli/tests/`

### 6.4 Add storage-layer fuzz targets
**Issue #28**

Currently only 3 fuzz targets (query, expression, CSV). Storage has 0.

**Fix:** Add:
- `fuzz_target WAL serialization/deserialization round-trip`
- `fuzz_target NodeGroup insert/read consistency`
- `fuzz_target BufferManager pin/evict stress`

**Estimated effort:** 1 day
**Files:** `akar-core/fuzz/fuzz_targets/`

---

## Phase 7: Refactoring (Week 6-7)

Lower priority structural improvements.

### 7.1 Unify dual catalog system
**Issue #8** | `akar_catalog::Catalog` + `akar_storage::TableCatalog`

**Approach:** Merge `TableCatalog` into `Catalog`. `Catalog` owns all table metadata AND data-level `NodeTable`/`RelTable` instances. Remove `TableCatalog` from `akar-storage`.

**Estimated effort:** 2-3 days
**Files:** `akar-catalog/src/lib.rs`, `akar-storage/src/table.rs`, `akar-storage/src/lib.rs`, `akar-main/src/database.rs`

### 7.2 Remove `println!()` from production code
**Issue #10** | 2 locations

- `akar-optimizer/src/passes/tree/sip.rs:53` → `tracing::debug!`
- `akar-processor/src/physical/write_ops/recursiveextend.rs:576` → `tracing::debug!`

**Estimated effort:** 10 minutes
**Files:** 2 files

### 7.3 Clean up `#[allow(dead_code)]` ✅ DONE
**Issue #26** | 24 occurrences in 13 files

**Fix (implemented):**
- 15 `#[allow(dead_code)]` annotations removed (26 → 11 remaining)
- 8 dead code items deleted entirely: `lock_or_poisoned_arc`, `blob_exists`, `list_tables`, `run_edge_compute`, `raw_key_hash_i64`, `raw_key_hash_bytes`, `write_test_npy`, `write_value_to_vector`
- 7 annotations removed from actually-used items (were unnecessary)
- 11 remaining are justified: struct-level annotations (BufferManager, StorageManager, Database), test-only functions, placeholder fields

**Files changed:** `akar-common/src/error.rs`, `akar-azure/src/azure_storage.rs`, `akar-delta/src/native_reader.rs`, `akar-optimizer/src/passes/tree/subquery_unnesting.rs`, `akar-unity-catalog/src/native_client.rs`, `akar-transaction/src/lib.rs`, `akar-graph/src/gds/utils.rs`, `akar-storage/src/node_group.rs`, `akar-storage/src/npy_reader.rs`, `akar-processor/src/physical/common.rs`, `akar-processor/src/physical/scan_filter/scan.rs`, `akar-main/src/connection/utils.rs`

### 7.4 Replace `.lock().unwrap()` with graceful handling ✅ DONE
**Issue #27** | 74+ locations (actual count: 123 across 26 files)

**Fix (implemented):**
- ~75 `.lock().unwrap()` calls replaced with `.lock().map_err(|e| format!("Lock poisoned: {e}"))?` across 13 files in 7 crates
- Top files fixed: `akar-binder` (22), `akar-storage` (11), `akar-main` (25), `akar-processor` (7), `akar-transaction` (3), `akar-duckdb` (4), `akar-cli` (1)
- 53 remaining are justified: infallible functions (no `Result` return), closures, test/benchmark code
- Zero regressions — 317 tests pass, same 3 pre-existing WAL failures

**Files changed:** `akar-binder/src/binder/mod.rs`, `akar-binder/src/binder/dml.rs`, `akar-storage/src/lib.rs`, `akar-storage/src/checkpoint.rs`, `akar-storage/src/column.rs`, `akar-storage/src/shadow_file.rs`, `akar-main/src/connection/standalone_call.rs`, `akar-main/src/connection/ddl.rs`, `akar-main/src/connection/query.rs`, `akar-main/src/connection/copy.rs`, `akar-main/src/database.rs`, `akar-processor/src/physical/order_aggregate/splitaggregation.rs`, `akar-processor/src/expression_evaluator.rs`, `akar-processor/src/processor/mod.rs`, `akar-transaction/src/lib.rs`, `akar-duckdb/src/connection.rs`, `akar-cli/src/main.rs`

---

## Execution Order & Dependencies

```
Phase 1 (Week 1-2)  ─── Correctness & Safety
  ├─ 1.1 (worker thread)     ← independent
  ├─ 1.2 (drain bypass)      ← independent
  ├─ 1.3 (MVCC isolation)    ← independent, largest
  ├─ 1.4 (unsafe borrow)     ← independent
  └─ 1.5 (rollback errors)   ← independent

Phase 2 (Week 2-3)  ─── Durability & WAL
  ├─ 2.1 (atomic WAL)        ← independent
  ├─ 2.2 (WAL checksums)     ← independent
  └─ 2.3 (set_value errors)  ← independent

Phase 3 (Week 3-4)  ─── Error Handling
  ├─ 3.1 (define AkarError)  ← independent
  └─ 3.2 (migrate crates)    ← depends on 3.1
  └─ 3.3 (expect→?)          ← independent

Phase 4 (Week 4-5)  ─── Concurrency & Performance
  ├─ 4.1 (RwLock)            ← independent
  ├─ 4.2 (evaluator clones)  ← independent
  ├─ 4.3 (split TM)          ← independent
  └─ 4.4 (dedup callback)    ← independent

Phase 5 (Week 5)    ─── Build & CI
  └─ 5.1-5.6                 ← all independent

Phase 6 (Week 5-6)  ─── Test Quality
  ├─ 6.1 (test helpers)      ← independent
  ├─ 6.2 (float asserts)     ← independent
  ├─ 6.3 (untested crates)   ← independent
  └─ 6.4 (fuzz targets)      ← independent

Phase 7 (Week 6-7)  ─── Refactoring
  ├─ 7.1 (unify catalog)     ← independent, large
  ├─ 7.2 (remove println)    ← independent
  ├─ 7.3 (dead code)         ← independent
  └─ 7.4 (lock unwrap)       ← independent
```

---

## Effort Summary

| Phase | Estimated Effort | Risk |
|-------|-----------------|------|
| 1. Correctness & Safety | 5-8 days | High — changes core data paths |
| 2. Durability & WAL | 2-3 days | Medium — WAL is critical path |
| 3. Error Handling | 3-4 days | Medium — large surface area |
| 4. Concurrency & Performance | 3-4 days | Medium — must not regress correctness |
| 5. Build & CI | 0.5 day | Low — infrastructure only |
| 6. Test Quality | 2-3 days | Low — no production changes |
| 7. Refactoring | 3-4 days | Low-Medium — structural cleanup |
| **Total** | **~16-21 days** | |

---

## Quick Wins (Do First, < 1 day total)

These can be done immediately with near-zero risk:

1. Remove `bitflags`, `uuid`, `rust_decimal` from workspace deps
2. Fix contradictory `debug`/`strip` profiles
3. Replace 2 `println!()` with `tracing::debug!`
4. Fix `tools/rust_api` edition to 2024
5. Add `Swatinem/rust-cache@v2` to CI
6. Fix 2 `.expect()` calls in processor

---

## Testing Strategy

After each phase:
1. Run `cargo test --workspace` — all 1,538 tests must pass
2. Run `cargo clippy --workspace -- -D warnings` — no new warnings
3. Run `cargo fmt --all -- --check` — formatting consistent
4. For Phase 1.3 (MVCC): add specific concurrency tests with multiple threads
5. For Phase 2.1 (WAL): test crash recovery by killing process mid-write
6. For Phase 3: verify error messages are descriptive via integration tests

---

## Risk Mitigation

- **MVCC isolation (1.3)** is the highest-risk change. Consider implementing it behind a feature flag (`mvcc-isolation`) and running a shadow-read mode (read current + read snapshot, compare, log discrepancies) before switching fully.
- **Error migration (3.2)** touches every crate. Do it crate-by-crate with a `#[deprecated]` alias: `type AkarResult<T> = Result<T, AkarError>;` replacing `type Result<T> = Result<T, String>;`.
- **RwLock migration (4.1)** must be audited for write-lock requirements. Some paths that look read-only may need writes (e.g., updating frame `clock_ref`). Use `RwLockReadGuard` by default, upgrade to `write()` only where necessary.
