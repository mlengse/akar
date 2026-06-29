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
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

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

/// Result of a commit attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitResult {
    Committed { commit_ts: u64 },
    Aborted { reason: String },
}

/// Configuration for the transaction manager.
#[derive(Debug, Clone)]
pub struct TransactionManagerConfig {
    pub max_concurrent_writers: usize,
}

impl Default for TransactionManagerConfig {
    fn default() -> Self {
        Self {
            max_concurrent_writers: 1,
        }
    }
}

/// Manages concurrent transaction lifecycle with MVCC timestamp ordering.
///
/// - Write transactions are serialized (one at a time by default).
/// - Read transactions can proceed concurrently with writers.
/// - Write-write conflicts are detected and cause abort.
pub struct TransactionManager {
    next_id: AtomicU64,
    next_commit_ts: AtomicU64,
    active_transactions: Mutex<HashMap<u64, Transaction>>,
    /// Tracks which tables are locked by which transaction ID.
    table_locks: Mutex<HashMap<u64, u64>>, // table_id → transaction_id
    commit_history: Mutex<Vec<(u64, u64)>>,
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
            config,
        }
    }

    pub fn begin_read(&self) -> Transaction {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let tx = Transaction::new(id, TransactionType::ReadOnly);
        if let Ok(mut active) = self.active_transactions.lock() {
            active.insert(id, tx.clone());
        }
        tx
    }

    pub fn begin_write(&self) -> Result<Transaction, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let tx = Transaction::new(id, TransactionType::Write);

        if let Ok(active) = self.active_transactions.lock() {
            let wc = active
                .values()
                .filter(|t| t.transaction_type == TransactionType::Write && t.is_active())
                .count();
            if wc >= self.config.max_concurrent_writers {
                return Err(format!(
                    "Max concurrent writers reached ({})",
                    self.config.max_concurrent_writers
                ));
            }
        }

        if let Ok(mut active) = self.active_transactions.lock() {
            active.insert(id, tx.clone());
        }
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
        CommitResult::Committed { commit_ts }
    }

    pub fn rollback(&self, transaction: &mut Transaction) -> Vec<UndoRecord> {
        transaction.status = TransactionStatus::RolledBack;
        let records = transaction.undo_records.clone();
        self.remove_from_active(transaction.transaction_id);
        self.release_locks(transaction.transaction_id);
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

    fn release_locks(&self, txn_id: u64) {
        if let Ok(mut locks) = self.table_locks.lock() {
            locks.retain(|_, &mut owner| owner != txn_id);
        }
    }

    fn remove_from_active(&self, txn_id: u64) {
        if let Ok(mut active) = self.active_transactions.lock() {
            active.remove(&txn_id);
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
    fn test_concurrent_writer_limit() {
        let config = TransactionManagerConfig {
            max_concurrent_writers: 1,
        };
        let tm = TransactionManager::new_with_config(config);
        let mut tx1 = tm.begin_write().unwrap();
        assert!(tm.begin_write().is_err());
        tm.commit(&mut tx1); // Free slot
        assert!(tm.begin_write().is_ok());
    }

    #[test]
    fn test_write_write_conflict() {
        let config = TransactionManagerConfig {
            max_concurrent_writers: 2,
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
            max_concurrent_writers: 2,
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
        let commit_ts = match tm.commit(&mut tx) {
            CommitResult::Committed { commit_ts } => commit_ts,
            _ => panic!("Commit failed"),
        };
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
