//! Transaction manager — MVCC-based serializable ACID transactions.
//!
//! Uses timestamp-based MVCC:
//! - Each transaction gets a unique begin timestamp (tx_id).
//! - Reads see a consistent snapshot based on their tx_id.
//! - Writes create new versions; old versions kept for concurrent readers.
//! - Serializable isolation via timestamp ordering + conflict detection.
//!
//! Integration with the storage engine: the `StorageManager::commit_transaction()`
//! method orchestrates the full commit pipeline (WAL → LocalStorage flush →
//! ShadowFile apply → checkpoint). Call it between `TransactionManager::
//! prepare_commit()` and `TransactionManager::finish_commit()` so the durable
//! pipeline runs before the transaction is published as committed (P51.29).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use akar_common::error::TransactionError;

/// Type of transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionType {
    ReadOnly,
    Write,
}

/// Status of a transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionStatus {
    Active,
    Committed,
    RolledBack,
}

/// Type of undo operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoType {
    /// Restore a single cell to its previous value (column-level undo).
    Update,
    /// Delete a row that was inserted (row-level undo).
    Insert,
    /// Restore all columns of a deleted row (row-level undo).
    Delete,
}

/// An undo record for rolling back a write transaction.
#[derive(Debug, Clone)]
pub struct UndoRecord {
    pub table_id: u64,
    pub row_id: u64,
    pub column: u32,
    pub old_data: Vec<u8>,
    pub undo_type: UndoType,
}

impl UndoRecord {
    pub fn update(table_id: u64, row_id: u64, column: u32, old_data: Vec<u8>) -> Self {
        Self {
            table_id,
            row_id,
            column,
            old_data,
            undo_type: UndoType::Update,
        }
    }

    pub fn insert(table_id: u64, row_id: u64) -> Self {
        Self {
            table_id,
            row_id,
            column: 0,
            old_data: Vec::new(),
            undo_type: UndoType::Insert,
        }
    }

    pub fn delete(table_id: u64, row_id: u64, old_row_data: Vec<u8>) -> Self {
        Self {
            table_id,
            row_id,
            column: 0,
            old_data: old_row_data,
            undo_type: UndoType::Delete,
        }
    }
}

/// A database transaction context with MVCC state.
#[derive(Debug, Clone)]
pub struct Transaction {
    pub transaction_id: u64,
    pub transaction_type: TransactionType,
    pub commit_ts: Option<u64>,
    pub status: TransactionStatus,
    pub undo_records: Vec<UndoRecord>,
    pub modified_tables: Vec<u64>,
    /// Snapshot timestamp for MVCC read isolation.
    /// Set to `Some(commit_ts)` at transaction begin time. Readers at this
    /// snapshot see only data committed at or before this timestamp.
    pub snapshot_ts: Option<u64>,
    /// Row-level write set for conflict detection.
    /// Each entry is `(table_id, row_id)` — used by OCC to detect
    /// write-write conflicts at commit time.
    pub written_rows: Vec<(u64, u64)>,
}

impl Transaction {
    /// Create a new active transaction with the given ID and type.
    pub fn new(transaction_id: u64, transaction_type: TransactionType) -> Self {
        Self {
            transaction_id,
            transaction_type,
            commit_ts: None,
            status: TransactionStatus::Active,
            undo_records: Vec::new(),
            modified_tables: Vec::new(),
            snapshot_ts: None,
            written_rows: Vec::new(),
        }
    }

    pub fn record_undo(&mut self, table_id: u64, row_id: u64, column: u32, old_data: Vec<u8>) {
        self.undo_records
            .push(UndoRecord::update(table_id, row_id, column, old_data));
        if !self.modified_tables.contains(&table_id) {
            self.modified_tables.push(table_id);
        }
    }

    pub fn record_insert_undo(&mut self, table_id: u64, row_id: u64) {
        self.undo_records.push(UndoRecord::insert(table_id, row_id));
        if !self.modified_tables.contains(&table_id) {
            self.modified_tables.push(table_id);
        }
    }

    pub fn record_delete_undo(&mut self, table_id: u64, row_id: u64, old_row_data: Vec<u8>) {
        self.undo_records
            .push(UndoRecord::delete(table_id, row_id, old_row_data));
        if !self.modified_tables.contains(&table_id) {
            self.modified_tables.push(table_id);
        }
    }

    /// Record a row-level write for OCC conflict detection.
    /// Called after each update/insert/delete to build the write set.
    pub fn record_write(&mut self, table_id: u64, row_id: u64) {
        self.written_rows.push((table_id, row_id));
    }

    pub fn is_active(&self) -> bool {
        self.status == TransactionStatus::Active
    }
    pub fn is_committed(&self) -> bool {
        self.status == TransactionStatus::Committed
    }

    pub fn description(&self) -> String {
        let s = match self.status {
            TransactionStatus::Active => "active",
            TransactionStatus::Committed => "committed",
            TransactionStatus::RolledBack => "rolled_back",
        };
        format!(
            "txn#{} [{}] {} — {} undo records",
            self.transaction_id,
            match self.transaction_type {
                TransactionType::ReadOnly => "RO",
                TransactionType::Write => "RW",
            },
            s,
            self.undo_records.len()
        )
    }
}

/// Result of a commit attempt — always succeeds (blocking ensures no abort).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitResult {
    Committed { commit_ts: u64 },
}

/// Configuration for the transaction manager.
#[derive(Debug, Clone)]
pub struct TransactionManagerConfig {
    /// When true (default), multiple write transactions can run concurrently.
    /// When false, only one write transaction at a time is allowed (single-writer mode).
    pub concurrent_writes: bool,
}

impl Default for TransactionManagerConfig {
    fn default() -> Self {
        Self {
            concurrent_writes: true,
        }
    }
}

/// Transaction mode for a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionMode {
    /// Auto-commit: each query implicitly begins and commits.
    Auto,
    /// Manual: explicit BEGIN/COMMIT/ROLLBACK required.
    Manual,
}

/// Per-connection transaction context that manages the active transaction
/// and enforces AUTO/MANUAL mode semantics.
///
/// In AUTO mode, each write query is wrapped in an implicit transaction:
/// begin → execute → commit. Read queries proceed without a transaction.
///
/// In MANUAL mode, the user must call BEGIN TRANSACTION before any write
/// and COMMIT/ROLLBACK to finalize. Writes without an active transaction
/// return an error.
pub struct TransactionContext {
    /// Reference to the global transaction manager.
    manager: Arc<TransactionManager>,
    /// Current transaction mode.
    mode: TransactionMode,
    /// The active write transaction, if any.
    active_txn: Option<Transaction>,
}

impl TransactionContext {
    pub fn new(manager: Arc<TransactionManager>) -> Self {
        Self {
            manager,
            mode: TransactionMode::Auto,
            active_txn: None,
        }
    }

    /// Set the transaction mode.
    pub fn set_mode(&mut self, mode: TransactionMode) {
        self.mode = mode;
    }

    /// Get the current mode.
    pub fn mode(&self) -> TransactionMode {
        self.mode
    }

    /// Whether there is an active write transaction.
    pub fn has_active_txn(&self) -> bool {
        self.active_txn.is_some()
    }

    /// Get the active transaction ID, if any.
    pub fn active_txn_id(&self) -> Option<u64> {
        self.active_txn.as_ref().map(|t| t.transaction_id)
    }

    /// Begin a write transaction (explicit, for MANUAL mode).
    /// Returns error if a transaction is already active.
    pub fn begin_manual(&mut self) -> Result<&Transaction, TransactionError> {
        if self.active_txn.is_some() {
            return Err(TransactionError::NoActiveTransaction);
        }
        let txn = self.manager.begin_write()?;
        self.active_txn = Some(txn);
        Ok(self.active_txn.as_ref().unwrap())
    }

    /// Begin a write transaction (implicit, for AUTO mode DDL/DML).
    /// If already active, returns the existing transaction.
    pub fn begin_implicit(&mut self) -> Result<&Transaction, TransactionError> {
        if let Some(ref txn) = self.active_txn {
            return Ok(txn);
        }
        let txn = self.manager.begin_write()?;
        self.active_txn = Some(txn);
        Ok(self.active_txn.as_ref().unwrap())
    }

    /// Commit the active transaction.
    /// Returns the undo records and commit timestamp.
    pub fn commit(&mut self) -> Result<(Vec<UndoRecord>, u64), TransactionError> {
        let mut txn = self.active_txn.take().ok_or(TransactionError::NoActiveTransaction)?;
        let result = self.manager.commit(&mut txn)?;
        match result {
            CommitResult::Committed { commit_ts } => Ok((txn.undo_records, commit_ts)),
        }
    }

    /// Rollback the active transaction.
    /// Returns undo records for rollback application.
    pub fn rollback(&mut self) -> Result<Vec<UndoRecord>, TransactionError> {
        let mut txn = self.active_txn.take().ok_or(TransactionError::NoActiveTransaction)?;
        Ok(self.manager.rollback(&mut txn))
    }

    /// Auto-commit: begin (if not active), execute, commit.
    /// Used in AUTO mode for DDL/DML statements.
    pub fn auto_commit<F, T>(&mut self, exec_fn: F) -> Result<T, TransactionError>
    where
        F: FnOnce(&Transaction) -> Result<T, TransactionError>,
    {
        let _txn = self.begin_implicit()?;
        let _txn_id = self.active_txn_id().unwrap();
        let result = exec_fn(self.active_txn.as_ref().unwrap())?;
        self.commit()?;
        Ok(result)
    }

    /// Record an undo operation on the active transaction.
    pub fn record_undo(
        &mut self,
        table_id: u64,
        row_id: u64,
        column: u32,
        old_data: Vec<u8>,
    ) -> Result<(), TransactionError> {
        if let Some(ref mut txn) = self.active_txn {
            txn.record_undo(table_id, row_id, column, old_data);
            Ok(())
        } else {
            Err(TransactionError::NoActiveTransaction)
        }
    }

    /// Record an insert undo on the active transaction (for rollback: delete the row).
    pub fn record_insert_undo(&mut self, table_id: u64, row_id: u64) -> Result<(), TransactionError> {
        if let Some(ref mut txn) = self.active_txn {
            txn.record_insert_undo(table_id, row_id);
            Ok(())
        } else {
            Err(TransactionError::NoActiveTransaction)
        }
    }

    /// Record a delete undo on the active transaction (for rollback: restore the row).
    pub fn record_delete_undo(
        &mut self,
        table_id: u64,
        row_id: u64,
        old_row_data: Vec<u8>,
    ) -> Result<(), TransactionError> {
        if let Some(ref mut txn) = self.active_txn {
            txn.record_delete_undo(table_id, row_id, old_row_data);
            Ok(())
        } else {
            Err(TransactionError::NoActiveTransaction)
        }
    }

    /// Record a row-level write for OCC conflict detection.
    pub fn record_write(&mut self, table_id: u64, row_id: u64) -> Result<(), TransactionError> {
        if let Some(ref mut txn) = self.active_txn {
            txn.record_write(table_id, row_id);
            Ok(())
        } else {
            Err(TransactionError::NoActiveTransaction)
        }
    }

    /// Lock a table on the active transaction.
    pub fn lock_table(&self, table_id: u64) -> Result<(), TransactionError> {
        let txn_id = self.active_txn_id().ok_or(TransactionError::NoActiveTransaction)?;
        self.manager.lock_table(txn_id, table_id)
    }
}

impl Drop for TransactionContext {
    fn drop(&mut self) {
        // Auto-rollback any uncommitted transaction.
        if let Some(mut txn) = self.active_txn.take() {
            let _ = self.manager.rollback(&mut txn);
        }
    }
}

/// Manages concurrent transaction lifecycle with MVCC timestamp ordering.
///
/// - Write transactions are serialized (one at a time by default).
/// - Read transactions can proceed concurrently with writers.
/// - Write-write conflicts are detected and cause abort.
///
/// # Checkpoint Drain
///
/// Supports two-phase checkpoint drain: when a checkpoint is requested,
/// new transactions are gated (`mtx_for_starting_new_txns`) and the
/// system waits for all active transactions to finish before proceeding
/// with the checkpoint. A background worker thread handles auto-checkpoint.
pub struct TransactionManager {
    lifecycle: TransactionLifecycle,
    concurrency: ConcurrencyControl,
    checkpoint: CheckpointCoordinator,
}

// ---------------------------------------------------------------------------
// Sub-struct #1: Transaction Lifecycle — ID assignment, timestamps, registry
// ---------------------------------------------------------------------------

struct TransactionLifecycle {
    next_id: AtomicU64,
    next_commit_ts: AtomicU64,
    active_transactions: Mutex<HashMap<u64, Transaction>>,
    commit_history: Mutex<HashMap<u64, u64>>,
    active_txn_count: AtomicU32,
}

impl TransactionLifecycle {
    fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            next_commit_ts: AtomicU64::new(1),
            active_transactions: Mutex::new(HashMap::new()),
            commit_history: Mutex::new(HashMap::new()),
            active_txn_count: AtomicU32::new(0),
        }
    }

    fn next_txn_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    fn snapshot_ts(&self) -> u64 {
        self.next_commit_ts.load(Ordering::Acquire)
    }

    fn assign_commit_ts(&self) -> u64 {
        self.next_commit_ts.fetch_add(1, Ordering::SeqCst)
    }

    fn register(&self, txn: &Transaction) {
        if let Ok(mut active) = self.active_transactions.lock() {
            active.insert(txn.transaction_id, txn.clone());
        }
        self.active_txn_count.fetch_add(1, Ordering::Release);
    }

    fn deregister(&self, txn_id: u64) {
        if let Ok(mut active) = self.active_transactions.lock() {
            active.remove(&txn_id);
        }
        self.active_txn_count.fetch_sub(1, Ordering::AcqRel);
    }

    fn push_commit_history(&self, txn_id: u64, commit_ts: u64) {
        if let Ok(mut history) = self.commit_history.lock() {
            history.insert(txn_id, commit_ts);
        }
    }

    fn is_visible(&self, txn_id: u64, snapshot_ts: u64) -> bool {
        if let Ok(history) = self.commit_history.lock() {
            return history.get(&txn_id).is_some_and(|&commit_ts| commit_ts <= snapshot_ts);
        }
        false
    }

    fn commit_history_snapshot(&self) -> HashMap<u64, u64> {
        self.commit_history.lock().map(|h| h.clone()).unwrap_or_default()
    }

    fn current_commit_ts(&self) -> u64 {
        self.next_commit_ts.load(Ordering::Acquire)
    }

    fn num_active(&self) -> usize {
        self.active_transactions.lock().map(|a| a.len()).unwrap_or(0)
    }

    fn active_snapshot(&self) -> Result<HashMap<u64, Transaction>, TransactionError> {
        self.active_transactions
            .lock()
            .map(|a| a.clone())
            .map_err(|e| TransactionError::LockPoisoned(e.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Sub-struct #2: Concurrency Control — table locks, writer serialization,
//                row-level OCC conflict tracking
// ---------------------------------------------------------------------------

/// Row-level conflict tracker for Optimistic Concurrency Control.
///
/// Tracks which rows each active write transaction has modified.
/// At commit time, the write set is validated against this tracker —
/// if another active txn wrote to any of the same rows, a `WriteConflict`
/// error is returned.
struct RowConflictTracker {
    /// Map: (table_id, row_id) → set of txn_ids that wrote to this row
    active_writes: Mutex<HashMap<(u64, u64), HashSet<u64>>>,
}

impl RowConflictTracker {
    fn new() -> Self {
        Self {
            active_writes: Mutex::new(HashMap::new()),
        }
    }

    /// Register a row write for a transaction.
    fn register_write(&self, table_id: u64, row_id: u64, txn_id: u64) {
        if let Ok(mut writes) = self.active_writes.lock() {
            writes.entry((table_id, row_id)).or_default().insert(txn_id);
        }
    }

    /// Validate a transaction's write set at commit time.
    ///
    /// Returns `Ok(())` if no conflicts, or `Err(WriteConflict)` if another
    /// active transaction wrote to any of the same rows.
    fn validate_write_set(&self, written_rows: &[(u64, u64)], committing_txn: u64) -> Result<(), TransactionError> {
        let writes = self
            .active_writes
            .lock()
            .map_err(|e| TransactionError::LockPoisoned(e.to_string()))?;
        for &(table_id, row_id) in written_rows {
            if let Some(writer_set) = writes.get(&(table_id, row_id)) {
                // Check if any writer OTHER than the committing txn is active
                for &writer_txn in writer_set {
                    if writer_txn != committing_txn {
                        return Err(TransactionError::WriteConflict {
                            table_id,
                            row_id,
                            conflicting_txn: writer_txn,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Remove all writes by a transaction (called on commit/rollback).
    fn clear_txn_writes(&self, txn_id: u64) {
        if let Ok(mut writes) = self.active_writes.lock() {
            for writer_set in writes.values_mut() {
                writer_set.retain(|&id| id != txn_id);
            }
            // Remove entries with empty writer sets
            writes.retain(|_, writer_set| !writer_set.is_empty());
        }
    }
}

struct ConcurrencyControl {
    table_locks: Mutex<HashMap<u64, u64>>,
    concurrent_writes: AtomicBool,
    active_write_count: Mutex<u32>,
    writer_condvar: Condvar,
    row_tracker: RowConflictTracker,
}

impl ConcurrencyControl {
    fn new(concurrent_writes: bool) -> Self {
        Self {
            table_locks: Mutex::new(HashMap::new()),
            concurrent_writes: AtomicBool::new(concurrent_writes),
            active_write_count: Mutex::new(0),
            writer_condvar: Condvar::new(),
            row_tracker: RowConflictTracker::new(),
        }
    }

    fn allow_concurrent_writes(&self) -> bool {
        self.concurrent_writes.load(Ordering::Acquire)
    }

    fn acquire_write(&self) -> Result<(), TransactionError> {
        if !self.allow_concurrent_writes() {
            let mut count = self
                .active_write_count
                .lock()
                .map_err(|e| TransactionError::LockPoisoned(e.to_string()))?;
            while *count > 0 {
                count = self
                    .writer_condvar
                    .wait(count)
                    .map_err(|e| TransactionError::LockPoisoned(e.to_string()))?;
            }
            *count = 1;
        } else {
            let mut count = self
                .active_write_count
                .lock()
                .map_err(|e| TransactionError::LockPoisoned(e.to_string()))?;
            *count += 1;
        }
        Ok(())
    }

    fn release_write(&self) {
        if let Ok(mut count) = self.active_write_count.lock() {
            if *count > 0 {
                *count -= 1;
            }
        }
        self.writer_condvar.notify_one();
    }

    #[allow(clippy::collapsible_if)]
    fn lock_table(&self, txn_id: u64, table_id: u64) -> Result<(), TransactionError> {
        let mut locks = self
            .table_locks
            .lock()
            .map_err(|e| TransactionError::LockPoisoned(e.to_string()))?;
        if let Some(&owner) = locks.get(&table_id) {
            if owner != txn_id {
                return Err(TransactionError::TableLocked {
                    table_id,
                    owner_txn: owner,
                });
            }
        }
        locks.insert(table_id, txn_id);
        Ok(())
    }

    fn release_locks(&self, txn_id: u64) {
        if let Ok(mut locks) = self.table_locks.lock() {
            locks.retain(|_, &mut owner| owner != txn_id);
        }
    }
}

// ---------------------------------------------------------------------------
// Sub-struct #3: Checkpoint Coordinator — drain protocol, background worker
// ---------------------------------------------------------------------------

struct CheckpointCoordinator {
    mtx_for_starting_new_txns: Mutex<()>,
    #[allow(dead_code)]
    mtx_for_checkpoint: Mutex<()>,
    cv_active_txns_changed: Condvar,
    checkpoint_requested: Arc<AtomicBool>,
    shutdown_requested: Arc<AtomicBool>,
    worker_handle: Option<JoinHandle<()>>,
}

impl CheckpointCoordinator {
    fn new() -> Self {
        Self {
            mtx_for_starting_new_txns: Mutex::new(()),
            mtx_for_checkpoint: Mutex::new(()),
            cv_active_txns_changed: Condvar::new(),
            checkpoint_requested: Arc::new(AtomicBool::new(false)),
            shutdown_requested: Arc::new(AtomicBool::new(false)),
            worker_handle: None,
        }
    }

    fn notify_active_txns_changed(&self) {
        self.cv_active_txns_changed.notify_all();
    }

    /// Block until new transactions are allowed to start.
    ///
    /// Briefly acquires `mtx_for_starting_new_txns` (releasing immediately).
    /// While a checkpoint drain holds that mutex, this call blocks, so no new
    /// transaction can slip in during the drain wait — previously the gate was
    /// never touched by `begin_read`/`begin_write`, so `stop_new_txns_and_wait_
    /// until_all_leave` could never actually stop new transactions (P52.53).
    fn gate_new_transaction(&self) {
        let _gate = self.mtx_for_starting_new_txns.lock().unwrap_or_else(|p| p.into_inner());
    }

    fn schedule_auto_checkpoint(&self) {
        self.checkpoint_requested.store(true, Ordering::Release);
        self.cv_active_txns_changed.notify_all();
    }

    fn stop_new_txns_and_wait_until_all_leave(&self, active_txn_count: &AtomicU32, timeout: Duration) -> bool {
        let mut gate = match self.mtx_for_starting_new_txns.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };

        let start = std::time::Instant::now();
        while active_txn_count.load(Ordering::Acquire) > 0 {
            if start.elapsed() >= timeout {
                return false;
            }
            let result = self
                .cv_active_txns_changed
                .wait_timeout(gate, Duration::from_millis(100));
            match result {
                Ok((g, _)) => gate = g,
                Err(_) => return false,
            }
        }
        true
    }

    fn is_checkpoint_requested(&self) -> bool {
        self.checkpoint_requested.load(Ordering::Acquire)
    }

    fn clear_checkpoint_requested(&self) {
        self.checkpoint_requested.store(false, Ordering::Release);
    }

    fn is_shutdown_requested(&self) -> bool {
        self.shutdown_requested.load(Ordering::Acquire)
    }

    fn start_worker<F>(&mut self, checkpoint_fn: F)
    where
        F: Fn() -> std::io::Result<()> + Send + 'static,
    {
        let shutdown_clone = Arc::clone(&self.shutdown_requested);
        let checkpoint_clone = Arc::clone(&self.checkpoint_requested);

        let handle = thread::Builder::new()
            .name("akar-auto-checkpoint".into())
            .spawn(move || {
                tracing::info!("Auto-checkpoint worker started");
                loop {
                    if shutdown_clone.load(Ordering::Acquire) {
                        tracing::info!("Auto-checkpoint worker shutting down");
                        break;
                    }
                    if checkpoint_clone.load(Ordering::Acquire) {
                        checkpoint_clone.store(false, Ordering::Release);
                        match checkpoint_fn() {
                            Ok(()) => tracing::debug!("Auto-checkpoint completed"),
                            Err(e) => tracing::warn!("Auto-checkpoint failed: {e}"),
                        }
                    }
                    thread::sleep(Duration::from_millis(1000));
                }
            })
            .expect("Failed to spawn auto-checkpoint worker");

        self.worker_handle = Some(handle);
    }

    fn request_shutdown(&self) {
        self.shutdown_requested.store(true, Ordering::Release);
        self.cv_active_txns_changed.notify_all();
    }

    fn join_worker(&mut self) {
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}

// ---------------------------------------------------------------------------
// TransactionManager — top-level orchestrator
// ---------------------------------------------------------------------------

impl TransactionManager {
    /// Create a transaction manager with default configuration.
    pub fn new() -> Self {
        Self::new_with_config(TransactionManagerConfig::default())
    }

    /// Create a transaction manager with the given configuration.
    pub fn new_with_config(config: TransactionManagerConfig) -> Self {
        Self {
            lifecycle: TransactionLifecycle::new(),
            concurrency: ConcurrencyControl::new(config.concurrent_writes),
            checkpoint: CheckpointCoordinator::new(),
        }
    }

    pub fn allow_concurrent_writes(&self) -> bool {
        self.concurrency.allow_concurrent_writes()
    }

    pub fn set_concurrent_writes(&self, enabled: bool) {
        self.concurrency.concurrent_writes.store(enabled, Ordering::Release);
    }

    pub fn begin_read(&self) -> Transaction {
        self.checkpoint.gate_new_transaction();
        let id = self.lifecycle.next_txn_id();
        let snapshot_ts = self.lifecycle.snapshot_ts();
        let mut tx = Transaction::new(id, TransactionType::ReadOnly);
        tx.snapshot_ts = Some(snapshot_ts);
        self.lifecycle.register(&tx);
        tx
    }

    pub fn begin_write(&self) -> Result<Transaction, TransactionError> {
        self.checkpoint.gate_new_transaction();
        let id = self.lifecycle.next_txn_id();
        let snapshot_ts = self.lifecycle.snapshot_ts();
        let mut tx = Transaction::new(id, TransactionType::Write);
        tx.snapshot_ts = Some(snapshot_ts);
        self.concurrency.acquire_write()?;
        self.lifecycle.register(&tx);
        Ok(tx)
    }

    pub fn lock_table(&self, txn_id: u64, table_id: u64) -> Result<(), TransactionError> {
        self.concurrency.lock_table(txn_id, table_id)
    }

    /// Record a row-level write for OCC conflict detection.
    pub fn record_write(&self, txn_id: u64, table_id: u64, row_id: u64) {
        // Register in the conflict tracker
        self.concurrency.row_tracker.register_write(table_id, row_id, txn_id);
        // Also update the Transaction's own written_rows for validation
        if let Ok(mut active) = self.lifecycle.active_transactions.lock() {
            if let Some(txn) = active.get_mut(&txn_id) {
                txn.written_rows.push((table_id, row_id));
            }
        }
    }

    /// Validate a transaction's write set against all active writers.
    /// Called before commit to detect write-write conflicts.
    /// Reads the write set from the active_transactions map (which is updated by record_write).
    fn validate_write_set(&self, txn_id: u64) -> Result<(), TransactionError> {
        let written_rows = {
            let active = self
                .lifecycle
                .active_transactions
                .lock()
                .map_err(|e| TransactionError::LockPoisoned(e.to_string()))?;
            match active.get(&txn_id) {
                Some(txn) if !txn.written_rows.is_empty() => txn.written_rows.clone(),
                _ => return Ok(()),
            }
        };
        self.concurrency.row_tracker.validate_write_set(&written_rows, txn_id)
    }

    pub fn commit(&self, transaction: &mut Transaction) -> Result<CommitResult, TransactionError> {
        if transaction.transaction_type == TransactionType::ReadOnly {
            transaction.status = TransactionStatus::Committed;
            transaction.commit_ts = Some(self.lifecycle.assign_commit_ts());
            self.lifecycle.deregister(transaction.transaction_id);
            self.concurrency.release_locks(transaction.transaction_id);
            return Ok(CommitResult::Committed {
                commit_ts: transaction.commit_ts.unwrap(),
            });
        }

        // OCC: validate write set before committing
        if let Err(e) = self.validate_write_set(transaction.transaction_id) {
            // Validation failed — clean up tracker and release resources
            transaction.status = TransactionStatus::RolledBack;
            self.lifecycle.deregister(transaction.transaction_id);
            self.concurrency.release_locks(transaction.transaction_id);
            self.concurrency
                .row_tracker
                .clear_txn_writes(transaction.transaction_id);
            self.concurrency.release_write();
            self.checkpoint.notify_active_txns_changed();
            return Err(e);
        }

        transaction.status = TransactionStatus::Committed;
        let commit_ts = self.lifecycle.assign_commit_ts();
        transaction.commit_ts = Some(commit_ts);
        self.lifecycle
            .push_commit_history(transaction.transaction_id, commit_ts);
        self.lifecycle.deregister(transaction.transaction_id);
        self.concurrency.release_locks(transaction.transaction_id);
        self.concurrency
            .row_tracker
            .clear_txn_writes(transaction.transaction_id);
        self.concurrency.release_write();
        self.checkpoint.notify_active_txns_changed();
        Ok(CommitResult::Committed { commit_ts })
    }

    /// Phase 1 of a two-phase commit for a write transaction (P51.29).
    ///
    /// Runs OCC validation, marks the transaction committed, assigns its
    /// commit timestamp and detaches it from the lifecycle — but does NOT yet
    /// release its table locks, clear its row-write registrations or release
    /// the global write lock. This lets the caller run the storage engine's
    /// durable commit pipeline (WAL + persist) in between, while a checkpoint
    /// drain triggered there no longer waits on this transaction (it is no
    /// longer counted as active).
    ///
    /// If the durable pipeline fails, call [`TransactionManager::rollback`] to
    /// abandon the commit (the commit-history entry is only pushed by
    /// [`TransactionManager::finish_commit`], so a failed durable phase leaves
    /// no committed record behind). On success, call `finish_commit`.
    pub fn prepare_commit(&self, transaction: &mut Transaction) -> Result<CommitResult, TransactionError> {
        if transaction.transaction_type == TransactionType::ReadOnly {
            transaction.status = TransactionStatus::Committed;
            transaction.commit_ts = Some(self.lifecycle.assign_commit_ts());
            self.lifecycle.deregister(transaction.transaction_id);
            self.concurrency.release_locks(transaction.transaction_id);
            return Ok(CommitResult::Committed {
                commit_ts: transaction.commit_ts.unwrap(),
            });
        }

        // OCC: validate write set before any durable work is done.
        if let Err(e) = self.validate_write_set(transaction.transaction_id) {
            // Validation failed — clean up tracker and release resources
            transaction.status = TransactionStatus::RolledBack;
            self.lifecycle.deregister(transaction.transaction_id);
            self.concurrency.release_locks(transaction.transaction_id);
            self.concurrency
                .row_tracker
                .clear_txn_writes(transaction.transaction_id);
            self.concurrency.release_write();
            self.checkpoint.notify_active_txns_changed();
            return Err(e);
        }

        transaction.status = TransactionStatus::Committed;
        let commit_ts = self.lifecycle.assign_commit_ts();
        transaction.commit_ts = Some(commit_ts);
        self.lifecycle.deregister(transaction.transaction_id);
        Ok(CommitResult::Committed { commit_ts })
    }

    /// Phase 2 of a two-phase commit (P51.29): publish the commit to the
    /// commit history and release the resources still held after
    /// [`TransactionManager::prepare_commit`] — table locks, the row-write
    /// registrations and the global write lock.
    pub fn finish_commit(&self, transaction: &Transaction) {
        self.lifecycle
            .push_commit_history(transaction.transaction_id, transaction.commit_ts.unwrap_or(0));
        self.concurrency.release_locks(transaction.transaction_id);
        self.concurrency
            .row_tracker
            .clear_txn_writes(transaction.transaction_id);
        self.concurrency.release_write();
        self.checkpoint.notify_active_txns_changed();
    }

    pub fn rollback(&self, transaction: &mut Transaction) -> Vec<UndoRecord> {
        transaction.status = TransactionStatus::RolledBack;
        let records = transaction.undo_records.clone();
        self.lifecycle.deregister(transaction.transaction_id);
        self.concurrency.release_locks(transaction.transaction_id);
        self.concurrency
            .row_tracker
            .clear_txn_writes(transaction.transaction_id);
        self.concurrency.release_write();
        self.checkpoint.notify_active_txns_changed();
        records
    }

    pub fn is_visible(&self, txn_id: u64, snapshot_ts: u64) -> bool {
        self.lifecycle.is_visible(txn_id, snapshot_ts)
    }

    pub fn commit_history_snapshot(&self) -> HashMap<u64, u64> {
        self.lifecycle.commit_history_snapshot()
    }

    pub fn current_commit_ts(&self) -> u64 {
        self.lifecycle.current_commit_ts()
    }

    pub fn num_active(&self) -> usize {
        self.lifecycle.num_active()
    }

    pub fn active_snapshot(&self) -> Result<HashMap<u64, Transaction>, TransactionError> {
        self.lifecycle.active_snapshot()
    }

    pub fn schedule_auto_checkpoint(&self) {
        self.checkpoint.schedule_auto_checkpoint();
    }

    pub fn stop_new_txns_and_wait_until_all_leave(&self, timeout: Duration) -> bool {
        self.checkpoint
            .stop_new_txns_and_wait_until_all_leave(&self.lifecycle.active_txn_count, timeout)
    }

    pub fn is_checkpoint_requested(&self) -> bool {
        self.checkpoint.is_checkpoint_requested()
    }

    pub fn clear_checkpoint_requested(&self) {
        self.checkpoint.clear_checkpoint_requested();
    }

    pub fn is_shutdown_requested(&self) -> bool {
        self.checkpoint.is_shutdown_requested()
    }

    pub fn start_auto_checkpoint_worker<F>(&mut self, checkpoint_fn: F)
    where
        F: Fn() -> std::io::Result<()> + Send + 'static,
    {
        self.checkpoint.start_worker(checkpoint_fn);
    }

    pub fn request_shutdown(&self) {
        self.checkpoint.request_shutdown();
    }
}

impl Drop for TransactionManager {
    fn drop(&mut self) {
        self.request_shutdown();
        self.checkpoint.join_worker();
    }
}

impl Default for TransactionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_begin_read() {
        let tm = TransactionManager::new();
        let tx = tm.begin_read();
        assert!(tx.is_active());
        assert_eq!(tx.transaction_type, TransactionType::ReadOnly);
    }

    #[test]
    fn test_begin_write() {
        let tm = TransactionManager::new();
        let tx = tm.begin_write().unwrap();
        assert!(tx.is_active());
        assert_eq!(tx.transaction_type, TransactionType::Write);
    }

    #[test]
    fn test_commit_read() {
        let tm = TransactionManager::new();
        let mut tx = tm.begin_read();
        assert!(matches!(tm.commit(&mut tx), Ok(CommitResult::Committed { .. })));
        assert!(tx.is_committed());
    }

    #[test]
    fn test_commit_write() {
        let tm = TransactionManager::new();
        let mut tx = tm.begin_write().unwrap();
        tm.lock_table(tx.transaction_id, 1).unwrap();
        tx.record_undo(1, 0, 0, vec![0]);
        assert!(matches!(tm.commit(&mut tx), Ok(CommitResult::Committed { .. })));
        assert!(tx.is_committed());
    }

    #[test]
    fn test_rollback() {
        let tm = TransactionManager::new();
        let mut tx = tm.begin_write().unwrap();
        tx.record_undo(1, 42, 0, vec![1, 2, 3]);
        let records = tm.rollback(&mut tx);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].row_id, 42);
        assert_eq!(tx.status, TransactionStatus::RolledBack);
    }

    #[test]
    fn test_concurrent_writer_limit_single_writer_mode() {
        // With concurrent_writes=false, a second writer blocks until the first finishes.
        let config = TransactionManagerConfig {
            concurrent_writes: false,
        };
        let tm = std::sync::Arc::new(TransactionManager::new_with_config(config));
        let tm_clone = tm.clone();

        let handle = std::thread::spawn(move || {
            // This should succeed after tx1 commits (blocking wait)
            let mut tx2 = tm_clone.begin_write().unwrap();
            assert!(tx2.is_active());
            assert_eq!(tx2.transaction_type, TransactionType::Write);
            tm_clone.commit(&mut tx2).unwrap();
        });

        let mut tx1 = tm.begin_write().unwrap();
        assert!(tx1.is_active());
        // Give the thread time to block on begin_write
        std::thread::sleep(std::time::Duration::from_millis(50));
        // tx1 is still active — thread is blocked
        tm.commit(&mut tx1).unwrap();
        // Now thread should unblock and succeed
        handle.join().expect("Thread should complete");
    }

    #[test]
    fn test_concurrent_writes_allowed() {
        // With concurrent_writes=true (default), multiple writers are allowed.
        let tm = TransactionManager::new();
        let _tx1 = tm.begin_write().unwrap();
        let _tx2 = tm.begin_write().unwrap();
        // Both succeed — no error
        assert!(tm.begin_write().is_ok());
    }

    #[test]
    fn test_write_write_conflict() {
        let config = TransactionManagerConfig {
            concurrent_writes: true,
        };
        let tm = TransactionManager::new_with_config(config);
        let mut tx1 = tm.begin_write().unwrap();
        tm.lock_table(tx1.transaction_id, 1).unwrap();
        let mut tx2 = tm.begin_write().unwrap();
        // tx2 should fail to lock table 1 since tx1 already holds it
        assert!(tm.lock_table(tx2.transaction_id, 1).is_err());
        tm.commit(&mut tx1).unwrap();
        // After tx1 commits, tx2 can lock table 1
        assert!(tm.lock_table(tx2.transaction_id, 1).is_ok());
        tm.commit(&mut tx2).unwrap();
    }

    #[test]
    fn test_no_conflict_different_tables() {
        let config = TransactionManagerConfig {
            concurrent_writes: true,
        };
        let tm = TransactionManager::new_with_config(config);
        let mut tx1 = tm.begin_write().unwrap();
        tm.lock_table(tx1.transaction_id, 1).unwrap();
        let mut tx2 = tm.begin_write().unwrap();
        // Different table — no conflict
        assert!(tm.lock_table(tx2.transaction_id, 2).is_ok());
        assert!(matches!(tm.commit(&mut tx1), Ok(CommitResult::Committed { .. })));
        assert!(matches!(tm.commit(&mut tx2), Ok(CommitResult::Committed { .. })));
    }

    #[test]
    fn test_mvcc_visibility() {
        let tm = TransactionManager::new();
        let mut tx = tm.begin_write().unwrap();
        tx.record_undo(1, 0, 0, vec![0]);
        let CommitResult::Committed { commit_ts } = tm.commit(&mut tx).unwrap();
        assert!(tm.is_visible(tx.transaction_id, commit_ts));
        assert!(tm.is_visible(tx.transaction_id, commit_ts + 1));
    }

    #[test]
    fn test_description() {
        let tm = TransactionManager::new();
        let tx = tm.begin_read();
        assert!(tx.description().contains("RO"));
        assert!(tx.description().contains("active"));
    }

    #[test]
    fn test_num_active_changes() {
        let tm = TransactionManager::new();
        let mut tx1 = tm.begin_read();
        let mut tx2 = tm.begin_write().unwrap();
        assert_eq!(tm.num_active(), 2);
        tm.commit(&mut tx2).unwrap();
        assert_eq!(tm.num_active(), 1);
        let _ = tm.commit(&mut tx1); // read-only, always succeeds
        assert_eq!(tm.num_active(), 0);
    }

    #[test]
    fn test_row_level_write_conflict() {
        let config = TransactionManagerConfig {
            concurrent_writes: true,
        };
        let tm = TransactionManager::new_with_config(config);
        let mut tx1 = tm.begin_write().unwrap();
        let mut tx2 = tm.begin_write().unwrap();

        // Both write to the same row (table=1, row=10)
        tm.record_write(tx1.transaction_id, 1, 10);
        tm.record_write(tx2.transaction_id, 1, 10);

        // tx1 tries to commit first — should fail because tx2 also wrote to same row
        let result = tm.commit(&mut tx1);
        assert!(result.is_err(), "Expected WriteConflict but got {:?}", result);
        match result.unwrap_err() {
            TransactionError::WriteConflict {
                table_id,
                row_id,
                conflicting_txn,
            } => {
                assert_eq!(table_id, 1);
                assert_eq!(row_id, 10);
                assert_eq!(conflicting_txn, tx2.transaction_id);
            }
            other => panic!("Expected WriteConflict, got {:?}", other),
        }

        // tx2 commits — should succeed (tx1 already rolled back from failed commit)
        assert!(matches!(tm.commit(&mut tx2), Ok(CommitResult::Committed { .. })));
    }

    #[test]
    fn test_no_row_conflict_different_rows() {
        let config = TransactionManagerConfig {
            concurrent_writes: true,
        };
        let tm = TransactionManager::new_with_config(config);
        let mut tx1 = tm.begin_write().unwrap();
        let mut tx2 = tm.begin_write().unwrap();

        // Different rows — no conflict
        tm.record_write(tx1.transaction_id, 1, 10);
        tm.record_write(tx2.transaction_id, 1, 20);

        assert!(matches!(tm.commit(&mut tx1), Ok(CommitResult::Committed { .. })));
        assert!(matches!(tm.commit(&mut tx2), Ok(CommitResult::Committed { .. })));
    }

    #[test]
    fn test_rollback_clears_row_writes() {
        let config = TransactionManagerConfig {
            concurrent_writes: true,
        };
        let tm = TransactionManager::new_with_config(config);
        let mut tx1 = tm.begin_write().unwrap();
        let mut tx2 = tm.begin_write().unwrap();

        // tx1 writes to row 10 and rolls back
        tm.record_write(tx1.transaction_id, 1, 10);
        tm.rollback(&mut tx1);

        // tx2 writes to same row — should succeed since tx1 rolled back
        tm.record_write(tx2.transaction_id, 1, 10);
        assert!(matches!(tm.commit(&mut tx2), Ok(CommitResult::Committed { .. })));
    }

    #[test]
    fn test_insert_row_level_no_conflict_different_rows() {
        let config = TransactionManagerConfig {
            concurrent_writes: true,
        };
        let tm = TransactionManager::new_with_config(config);
        let mut tx1 = tm.begin_write().unwrap();
        let mut tx2 = tm.begin_write().unwrap();

        // Two concurrent inserts with different primary keys map to
        // different internal row IDs — no OCC conflict (row-level, not table-level).
        tm.record_write(tx1.transaction_id, 1, 100);
        tm.record_write(tx2.transaction_id, 1, 101);

        assert!(matches!(tm.commit(&mut tx1), Ok(CommitResult::Committed { .. })));
        assert!(matches!(tm.commit(&mut tx2), Ok(CommitResult::Committed { .. })));
    }

    #[test]
    fn test_insert_same_primary_key_write_conflict() {
        let config = TransactionManagerConfig {
            concurrent_writes: true,
        };
        let tm = TransactionManager::new_with_config(config);
        let mut tx1 = tm.begin_write().unwrap();
        let mut tx2 = tm.begin_write().unwrap();

        // Both transactions insert a node with the SAME primary key value,
        // which resolves to the same internal row ID. The second commit
        // must produce a WriteConflict.
        tm.record_write(tx1.transaction_id, 1, 100);
        tm.record_write(tx2.transaction_id, 1, 100);

        let result = tm.commit(&mut tx1);
        assert!(result.is_err(), "Expected WriteConflict but got {:?}", result);
        match result.unwrap_err() {
            TransactionError::WriteConflict {
                table_id,
                row_id,
                conflicting_txn,
            } => {
                assert_eq!(table_id, 1);
                assert_eq!(row_id, 100);
                assert_eq!(conflicting_txn, tx2.transaction_id);
            }
            other => panic!("Expected WriteConflict, got {:?}", other),
        }

        assert!(matches!(tm.commit(&mut tx2), Ok(CommitResult::Committed { .. })));
    }

    #[test]
    fn test_row_write_with_table_lock() {
        let config = TransactionManagerConfig {
            concurrent_writes: true,
        };
        let tm = TransactionManager::new_with_config(config);
        let mut tx1 = tm.begin_write().unwrap();
        let mut tx2 = tm.begin_write().unwrap();

        // Both lock the same table AND write to the same row
        tm.lock_table(tx1.transaction_id, 1).unwrap();
        tm.record_write(tx1.transaction_id, 1, 10);

        // tx2 fails at table lock level
        assert!(tm.lock_table(tx2.transaction_id, 1).is_err());

        // tx1 commits
        assert!(matches!(tm.commit(&mut tx1), Ok(CommitResult::Committed { .. })));

        // Now tx2 can lock the table and write
        assert!(tm.lock_table(tx2.transaction_id, 1).is_ok());
        tm.record_write(tx2.transaction_id, 1, 10);
        assert!(matches!(tm.commit(&mut tx2), Ok(CommitResult::Committed { .. })));
    }
}
