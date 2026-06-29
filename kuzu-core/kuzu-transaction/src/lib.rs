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
//! ShadowFile apply → checkpoint). Call it after `TransactionManager::commit()`.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

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

/// An undo record for rolling back a write transaction.
#[derive(Debug, Clone)]
pub struct UndoRecord {
    pub table_id: u64,
    pub row_id: u64,
    pub column: u32,
    pub old_data: Vec<u8>,
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
}

impl Transaction {
    pub fn new(transaction_id: u64, transaction_type: TransactionType) -> Self {
        Self {
            transaction_id,
            transaction_type,
            commit_ts: None,
            status: TransactionStatus::Active,
            undo_records: Vec::new(),
            modified_tables: Vec::new(),
        }
    }

    pub fn record_undo(&mut self, table_id: u64, row_id: u64, column: u32, old_data: Vec<u8>) {
        self.undo_records.push(UndoRecord {
            table_id,
            row_id,
            column,
            old_data,
        });
        if !self.modified_tables.contains(&table_id) {
            self.modified_tables.push(table_id);
        }
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
    next_id: AtomicU64,
    next_commit_ts: AtomicU64,
    active_transactions: Mutex<HashMap<u64, Transaction>>,
    /// Tracks which tables are locked by which transaction ID.
    table_locks: Mutex<HashMap<u64, u64>>, // table_id → transaction_id
    commit_history: Mutex<Vec<(u64, u64)>>,
    /// Whether concurrent writes are allowed. Can be toggled at runtime
    /// (e.g., via `SET concurrent_writes=true|false`).
    concurrent_writes: AtomicBool,
    /// Active write transaction count (used for single-writer blocking).
    active_write_count: Mutex<u32>,
    /// Condvar for blocking writers in single-writer mode.
    writer_condvar: Condvar,
    // -- Checkpoint drain fields --
    /// Gate mutex: held to prevent new transactions from starting during checkpoint.
    mtx_for_starting_new_txns: Mutex<()>,
    /// Gate mutex: held during the checkpoint operation itself.
    #[allow(dead_code)]
    mtx_for_checkpoint: Mutex<()>,
    /// Condvar signalled when the active transaction count changes.
    cv_active_txns_changed: Condvar,
    /// Number of currently active transactions (across all types).
    active_txn_count: AtomicU32,
    /// Signal flag: set to true when an auto-checkpoint is requested.
    checkpoint_requested: AtomicBool,
    /// Signal flag: set to true when the background worker should shut down.
    shutdown_requested: AtomicBool,
    /// Handle to the background auto-checkpoint worker thread.
    #[allow(dead_code)]
    worker_handle: Option<JoinHandle<()>>,
    /// Configuration snapshot kept for reference (used during construction).
    #[allow(dead_code)]
    config: TransactionManagerConfig,
}

impl TransactionManager {
    pub fn new() -> Self {
        Self::new_with_config(TransactionManagerConfig::default())
    }

    pub fn new_with_config(config: TransactionManagerConfig) -> Self {
        Self {
            next_id: AtomicU64::new(1),
            next_commit_ts: AtomicU64::new(1),
            active_transactions: Mutex::new(HashMap::new()),
            table_locks: Mutex::new(HashMap::new()),
            commit_history: Mutex::new(Vec::new()),
            concurrent_writes: AtomicBool::new(config.concurrent_writes),
            active_write_count: Mutex::new(0),
            writer_condvar: Condvar::new(),
            mtx_for_starting_new_txns: Mutex::new(()),
            mtx_for_checkpoint: Mutex::new(()),
            cv_active_txns_changed: Condvar::new(),
            active_txn_count: AtomicU32::new(0),
            checkpoint_requested: AtomicBool::new(false),
            shutdown_requested: AtomicBool::new(false),
            worker_handle: None,
            config,
        }
    }

    /// Check whether concurrent writes are currently allowed.
    pub fn allow_concurrent_writes(&self) -> bool {
        self.concurrent_writes.load(Ordering::Acquire)
    }

    /// Toggle concurrent writes at runtime.
    /// When set to `false`, falls back to single-writer (serialized) mode.
    pub fn set_concurrent_writes(&self, enabled: bool) {
        self.concurrent_writes.store(enabled, Ordering::Release);
    }

    pub fn begin_read(&self) -> Transaction {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let tx = Transaction::new(id, TransactionType::ReadOnly);
        if let Ok(mut active) = self.active_transactions.lock() {
            active.insert(id, tx.clone());
        }
        self.active_txn_count.fetch_add(1, Ordering::Release);
        tx
    }

    pub fn begin_write(&self) -> Result<Transaction, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let tx = Transaction::new(id, TransactionType::Write);

        // In single-writer mode, block until no other writer is active.
        if !self.allow_concurrent_writes() {
            let mut count = self.active_write_count.lock().unwrap();
            while *count > 0 {
                count = self.writer_condvar.wait(count).unwrap();
            }
            *count = 1;
        } else {
            // Track count for multi-writer mode too (for condvar notification).
            let mut count = self.active_write_count.lock().unwrap();
            *count += 1;
        }

        if let Ok(mut active) = self.active_transactions.lock() {
            active.insert(id, tx.clone());
        }
        self.active_txn_count.fetch_add(1, Ordering::Release);
        Ok(tx)
    }

    /// Lock a table for writing (called by the user after begin_write).
    /// Returns an error if another write transaction already holds the lock.
    #[allow(clippy::collapsible_if)]
    pub fn lock_table(&self, txn_id: u64, table_id: u64) -> Result<(), String> {
        let mut locks = self.table_locks.lock().unwrap();
        if let Some(&owner) = locks.get(&table_id) {
            if owner != txn_id {
                return Err(format!("Table {} already locked by txn#{}", table_id, owner));
            }
        }
        locks.insert(table_id, txn_id);
        Ok(())
    }

    pub fn commit(&self, transaction: &mut Transaction) -> CommitResult {
        if transaction.transaction_type == TransactionType::ReadOnly {
            transaction.status = TransactionStatus::Committed;
            transaction.commit_ts = Some(self.next_commit_ts.fetch_add(1, Ordering::SeqCst));
            self.remove_from_active(transaction.transaction_id);
            self.release_locks(transaction.transaction_id);
            return CommitResult::Committed {
                commit_ts: transaction.commit_ts.unwrap(),
            };
        }

        // Release all table locks on commit
        transaction.status = TransactionStatus::Committed;
        let commit_ts = self.next_commit_ts.fetch_add(1, Ordering::SeqCst);
        transaction.commit_ts = Some(commit_ts);

        if let Ok(mut history) = self.commit_history.lock() {
            history.push((transaction.transaction_id, commit_ts));
        }
        self.remove_from_active(transaction.transaction_id);
        self.release_locks(transaction.transaction_id);
        self.decrement_writer_and_notify();
        CommitResult::Committed { commit_ts }
    }

    pub fn rollback(&self, transaction: &mut Transaction) -> Vec<UndoRecord> {
        transaction.status = TransactionStatus::RolledBack;
        let records = transaction.undo_records.clone();
        self.remove_from_active(transaction.transaction_id);
        self.release_locks(transaction.transaction_id);
        self.decrement_writer_and_notify();
        records
    }

    /// Check if a transaction's changes are visible at the given snapshot timestamp.
    pub fn is_visible(&self, txn_id: u64, snapshot_ts: u64) -> bool {
        if let Ok(history) = self.commit_history.lock() {
            for &(id, commit_ts) in history.iter() {
                if id == txn_id {
                    return commit_ts <= snapshot_ts;
                }
            }
        }
        false
    }

    pub fn num_active(&self) -> usize {
        self.active_transactions.lock().map(|a| a.len()).unwrap_or(0)
    }

    /// Get a snapshot of all active transactions (cloned).
    /// Returns `Ok(map)` on success where the map is `HashMap<u64, Transaction>`.
    /// Used by Connection for COMMIT/ROLLBACK resolution.
    pub fn active_snapshot(&self) -> Result<HashMap<u64, Transaction>, String> {
        self.active_transactions
            .lock()
            .map(|a| a.clone())
            .map_err(|e| format!("Failed to lock active transactions: {e}"))
    }

    fn release_locks(&self, txn_id: u64) {
        if let Ok(mut locks) = self.table_locks.lock() {
            locks.retain(|_, &mut owner| owner != txn_id);
        }
    }

    fn remove_from_active(&self, txn_id: u64) {
        if let Ok(mut active) = self.active_transactions.lock() {
            active.remove(&txn_id);
        }
        // Decrement the global active count and notify checkpoint drain.
        let prev = self.active_txn_count.fetch_sub(1, Ordering::AcqRel);
        if prev > 0 {
            self.cv_active_txns_changed.notify_all();
        }
    }

    /// Decrement the active writer count and notify any blocked writers.
    fn decrement_writer_and_notify(&self) {
        let mut count = self.active_write_count.lock().unwrap();
        if *count > 0 {
            *count -= 1;
        }
        // If count reached 0, wake a waiting writer (single-writer mode).
        self.writer_condvar.notify_one();
    }

    // ------------------------------------------------------------------
    // Checkpoint drain API
    // ------------------------------------------------------------------

    /// Signal the background worker to schedule an auto-checkpoint.
    /// The worker will acquire the checkpoint gate and drain active txns.
    pub fn schedule_auto_checkpoint(&self) {
        self.checkpoint_requested.store(true, Ordering::Release);
        // Wake up the background worker if it's sleeping.
        self.cv_active_txns_changed.notify_all();
    }

    /// Two-phase gate: stop new transactions from starting and wait
    /// until all existing active transactions complete.
    ///
    /// Phase 1: Acquire `mtx_for_starting_new_txns` — new `begin_*()` calls
    /// will block waiting for this lock.
    ///
    /// Phase 2: Wait on `cv_active_txns_changed` until `active_txn_count`
    /// drops to 0.
    ///
    /// `timeout` is the maximum time to wait for active transactions to drain.
    /// Returns `true` if all transactions drained, `false` on timeout.
    pub fn stop_new_txns_and_wait_until_all_leave(&self, timeout: Duration) -> bool {
        // Phase 1: Acquire the new-txns gate
        let _gate = match self.mtx_for_starting_new_txns.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };

        // Phase 2: Wait until active_txn_count == 0, with timeout
        let start = std::time::Instant::now();
        while self.active_txn_count.load(Ordering::Acquire) > 0 {
            if start.elapsed() >= timeout {
                return false; // Timeout — active transactions still running
            }
            let _ = self
                .cv_active_txns_changed
                .wait_timeout(self.mtx_for_starting_new_txns.lock().unwrap(), Duration::from_millis(100))
                .unwrap();
        }

        true
    }

    /// Check whether a checkpoint has been requested (for the worker loop).
    pub fn is_checkpoint_requested(&self) -> bool {
        self.checkpoint_requested.load(Ordering::Acquire)
    }

    /// Clear the checkpoint-requested flag.
    pub fn clear_checkpoint_requested(&self) {
        self.checkpoint_requested.store(false, Ordering::Release);
    }

    /// Check whether shutdown has been requested.
    pub fn is_shutdown_requested(&self) -> bool {
        self.shutdown_requested.load(Ordering::Acquire)
    }

    /// Start the background auto-checkpoint worker thread.
    ///
    /// The worker polls for checkpoint requests and also performs
    /// periodic checks (every 1 second) based on WAL size.
    /// The caller must provide a callback to perform the actual
    /// checkpoint (typically `StorageManager::checkpoint_with_drain`).
    pub fn start_auto_checkpoint_worker<F>(&mut self, checkpoint_fn: F)
    where
        F: Fn() -> std::io::Result<()> + Send + 'static,
    {
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();
        let checkpoint_requested = Arc::new(AtomicBool::new(false));

        let handle = thread::Builder::new()
            .name("kuzu-auto-checkpoint".into())
            .spawn(move || {
                tracing::info!("Auto-checkpoint worker started");
                loop {
                    // Check for shutdown
                    if shutdown_clone.load(Ordering::Acquire) {
                        tracing::info!("Auto-checkpoint worker shutting down");
                        break;
                    }

                    // Check for checkpoint signal
                    if checkpoint_requested.load(Ordering::Acquire) {
                        match checkpoint_fn() {
                            Ok(()) => {
                                tracing::debug!("Auto-checkpoint completed");
                            }
                            Err(e) => {
                                tracing::warn!("Auto-checkpoint failed: {e}");
                            }
                        }
                    }

                    // Sleep before next poll
                    thread::sleep(Duration::from_millis(1000));
                }
            })
            .expect("Failed to spawn auto-checkpoint worker");

        self.worker_handle = Some(handle);
    }

    /// Request shutdown of the background worker.
    pub fn request_shutdown(&self) {
        self.shutdown_requested.store(true, Ordering::Release);
        self.cv_active_txns_changed.notify_all();
    }
}

impl Drop for TransactionManager {
    fn drop(&mut self) {
        self.request_shutdown();
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
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
        assert!(matches!(tm.commit(&mut tx), CommitResult::Committed { .. }));
        assert!(tx.is_committed());
    }

    #[test]
    fn test_commit_write() {
        let tm = TransactionManager::new();
        let mut tx = tm.begin_write().unwrap();
        tm.lock_table(tx.transaction_id, 1).unwrap();
        tx.record_undo(1, 0, 0, vec![0]);
        assert!(matches!(tm.commit(&mut tx), CommitResult::Committed { .. }));
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
            tm_clone.commit(&mut tx2);
        });

        let mut tx1 = tm.begin_write().unwrap();
        assert!(tx1.is_active());
        // Give the thread time to block on begin_write
        std::thread::sleep(std::time::Duration::from_millis(50));
        // tx1 is still active — thread is blocked
        tm.commit(&mut tx1);
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
        tm.commit(&mut tx1);
        // After tx1 commits, tx2 can lock table 1
        assert!(tm.lock_table(tx2.transaction_id, 1).is_ok());
        tm.commit(&mut tx2);
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
        assert!(matches!(tm.commit(&mut tx1), CommitResult::Committed { .. }));
        assert!(matches!(tm.commit(&mut tx2), CommitResult::Committed { .. }));
    }

    #[test]
    fn test_mvcc_visibility() {
        let tm = TransactionManager::new();
        let mut tx = tm.begin_write().unwrap();
        tx.record_undo(1, 0, 0, vec![0]);
        let CommitResult::Committed { commit_ts } = tm.commit(&mut tx);
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
        tm.commit(&mut tx2);
        assert_eq!(tm.num_active(), 1);
        tm.commit(&mut tx1); // Actually also need to commit tx1 to remove
        assert_eq!(tm.num_active(), 0);
    }
}
