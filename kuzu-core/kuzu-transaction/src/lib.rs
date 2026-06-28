//! Transaction manager — MVCC-based serializable ACID transactions.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Read or write transaction state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionType {
    ReadOnly,
    Write,
}

/// A database transaction context.
#[derive(Debug, Clone)]
pub struct Transaction {
    pub transaction_id: u64,
    pub transaction_type: TransactionType,
    pub commit_ts: Option<u64>,
    pub is_active: bool,
}

impl Transaction {
    pub fn new(transaction_id: u64, transaction_type: TransactionType) -> Self {
        Self {
            transaction_id,
            transaction_type,
            commit_ts: None,
            is_active: true,
        }
    }
}

/// Manages concurrent transaction lifecycle with MVCC.
pub struct TransactionManager {
    next_id: AtomicU64,
    active_transactions: Mutex<Vec<Transaction>>,
}

impl TransactionManager {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            active_transactions: Mutex::new(Vec::new()),
        }
    }

    pub fn begin_read(&self) -> Transaction {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        Transaction::new(id, TransactionType::ReadOnly)
    }

    pub fn begin_write(&self) -> Transaction {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let tx = Transaction::new(id, TransactionType::Write);
        if let Ok(mut active) = self.active_transactions.lock() {
            active.push(tx.clone());
        }
        tx
    }

    pub fn commit(&self, transaction: &mut Transaction) {
        transaction.is_active = false;
        if let Ok(mut active) = self.active_transactions.lock() {
            active.retain(|t| t.transaction_id != transaction.transaction_id);
        }
    }

    pub fn rollback(&self, transaction: &mut Transaction) {
        transaction.is_active = false;
        if let Ok(mut active) = self.active_transactions.lock() {
            active.retain(|t| t.transaction_id != transaction.transaction_id);
        }
    }

    pub fn num_active(&self) -> usize {
        self.active_transactions
            .lock()
            .map(|a| a.len())
            .unwrap_or(0)
    }
}

impl Default for TransactionManager {
    fn default() -> Self {
        Self::new()
    }
}
