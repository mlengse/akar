//! Unified error types for the Akar graph database.
//!
//! Every crate in the workspace returns `Result<T, AkarError>` (or a
//! subsystem-specific error that converts into `AkarError` via `From`).
//!
//! The hierarchy is:
//! ```text
//! AkarError
//! ├── Storage(StorageError)
//! ├── Transaction(TransactionError)
//! ├── Catalog(CatalogError)
//! ├── Binder(String)
//! ├── Planner(String)
//! ├── Processor(String)
//! ├── Parser(String)
//! ├── Io(std::io::Error)
//! └── Internal(String)
//! ```

use std::fmt;

// ---------------------------------------------------------------------------
// Top-level error
// ---------------------------------------------------------------------------

/// The single error type returned by all public Akar APIs.
#[derive(Debug)]
pub enum AkarError {
    /// Storage layer failure (WAL, buffer manager, table not found, etc.)
    Storage(StorageError),
    /// Transaction lifecycle failure (lock conflict, shutdown, etc.)
    Transaction(TransactionError),
    /// Catalog operation failure
    Catalog(CatalogError),
    /// Binder (semantic analysis) failure
    Binder(String),
    /// Logical planner failure
    Planner(String),
    /// Query processor / execution failure
    Processor(String),
    /// Parser failure
    Parser(String),
    /// I/O error
    Io(std::io::Error),
    /// Internal invariant violation (should never happen)
    Internal(String),
}

impl fmt::Display for AkarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(e) => write!(f, "storage: {e}"),
            Self::Transaction(e) => write!(f, "transaction: {e}"),
            Self::Catalog(e) => write!(f, "catalog: {e}"),
            Self::Binder(s) => write!(f, "binder: {s}"),
            Self::Planner(s) => write!(f, "planner: {s}"),
            Self::Processor(s) => write!(f, "processor: {s}"),
            Self::Parser(s) => write!(f, "parser: {s}"),
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Internal(s) => write!(f, "internal: {s}"),
        }
    }
}

impl std::error::Error for AkarError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(e) => Some(e),
            Self::Transaction(e) => Some(e),
            Self::Catalog(e) => Some(e),
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for AkarError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<StorageError> for AkarError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl From<TransactionError> for AkarError {
    fn from(e: TransactionError) -> Self {
        Self::Transaction(e)
    }
}

impl From<CatalogError> for AkarError {
    fn from(e: CatalogError) -> Self {
        Self::Catalog(e)
    }
}

/// Convenience alias used throughout the workspace.
pub type Result<T> = std::result::Result<T, AkarError>;

// ---------------------------------------------------------------------------
// Storage errors
// ---------------------------------------------------------------------------

/// Errors originating from the storage layer.
#[derive(Debug)]
pub enum StorageError {
    /// Write-ahead log failure
    Wal(String),
    /// Buffer manager failure
    BufferManager(String),
    /// Table not found in catalog
    TableNotFound(String),
    /// Column not found in table
    ColumnNotFound(String),
    /// Type mismatch between expected and actual
    TypeMismatch { expected: String, actual: String },
    /// Page / node-group error
    Page(String),
    /// Shadow file apply failure
    ShadowFile(String),
    /// Undo buffer failure
    Undo(String),
    /// Local storage flush failure
    LocalStorage(String),
    /// Spiller failure
    Spiller(String),
    /// Index error (ART, hash, vector)
    Index(String),
    /// CSV / Parquet / NPY reader error
    Reader(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wal(s) => write!(f, "WAL: {s}"),
            Self::BufferManager(s) => write!(f, "buffer manager: {s}"),
            Self::TableNotFound(s) => write!(f, "table not found: {s}"),
            Self::ColumnNotFound(s) => write!(f, "column not found: {s}"),
            Self::TypeMismatch { expected, actual } => {
                write!(f, "type mismatch: expected {expected}, got {actual}")
            }
            Self::Page(s) => write!(f, "page: {s}"),
            Self::ShadowFile(s) => write!(f, "shadow file: {s}"),
            Self::Undo(s) => write!(f, "undo: {s}"),
            Self::LocalStorage(s) => write!(f, "local storage: {s}"),
            Self::Spiller(s) => write!(f, "spiller: {s}"),
            Self::Index(s) => write!(f, "index: {s}"),
            Self::Reader(s) => write!(f, "reader: {s}"),
        }
    }
}

impl std::error::Error for StorageError {}

/// Allow `?` in functions still returning `Result<T, String>`.
impl From<StorageError> for String {
    fn from(e: StorageError) -> String {
        format!("storage: {e}")
    }
}

// ---------------------------------------------------------------------------
// Transaction errors
// ---------------------------------------------------------------------------

/// Errors originating from the transaction manager.
#[derive(Debug)]
pub enum TransactionError {
    /// Table already locked by another transaction
    TableLocked { table_id: u64, owner_txn: u64 },
    /// Concurrent write not allowed by config
    ConcurrentWriteDisabled,
    /// Transaction manager is shutting down
    ShuttingDown,
    /// No active write transaction
    NoActiveTransaction,
    /// Lock poison (a thread panicked while holding a lock)
    LockPoisoned(String),
}

impl fmt::Display for TransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TableLocked { table_id, owner_txn } => {
                write!(f, "table {table_id} already locked by txn#{owner_txn}")
            }
            Self::ConcurrentWriteDisabled => write!(f, "concurrent write not allowed"),
            Self::ShuttingDown => write!(f, "transaction manager is shutting down"),
            Self::NoActiveTransaction => write!(f, "no active write transaction"),
            Self::LockPoisoned(s) => write!(f, "lock poisoned: {s}"),
        }
    }
}

impl std::error::Error for TransactionError {}

/// Allow `?` in functions still returning `Result<T, String>`.
impl From<TransactionError> for String {
    fn from(e: TransactionError) -> String {
        format!("transaction: {e}")
    }
}

// ---------------------------------------------------------------------------
// Catalog errors
// ---------------------------------------------------------------------------

/// Errors originating from the catalog layer.
#[derive(Debug)]
pub enum CatalogError {
    /// Table already exists
    AlreadyExists(String),
    /// Table / entry not found
    NotFound(String),
    /// Column already exists on table
    ColumnAlreadyExists { table: String, column: String },
    /// Column not found on table
    ColumnNotFound { table: String, column: String },
    /// Invalid catalog operation
    InvalidOperation(String),
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyExists(s) => write!(f, "already exists: {s}"),
            Self::NotFound(s) => write!(f, "not found: {s}"),
            Self::ColumnAlreadyExists { table, column } => {
                write!(f, "column '{column}' already exists on table '{table}'")
            }
            Self::ColumnNotFound { table, column } => {
                write!(f, "column '{column}' not found on table '{table}'")
            }
            Self::InvalidOperation(s) => write!(f, "invalid operation: {s}"),
        }
    }
}

impl std::error::Error for CatalogError {}

/// Allow `?` in functions still returning `Result<T, String>`.
impl From<CatalogError> for String {
    fn from(e: CatalogError) -> String {
        format!("catalog: {e}")
    }
}

// ---------------------------------------------------------------------------
// Helper: lock_or_poisoned
// ---------------------------------------------------------------------------

/// Acquire a mutex guard, converting poison errors into `AkarError::Transaction`.
pub fn lock_or_poisoned<T>(mutex: &std::sync::Mutex<T>) -> crate::error::Result<std::sync::MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|e| AkarError::Transaction(TransactionError::LockPoisoned(e.to_string())))
}

/// Acquire a `parking_lot::Mutex`-style guard (if used) — same semantics.
/// For `std::sync::Mutex`, prefer `lock_or_poisoned`.
#[allow(dead_code)]
pub fn lock_or_poisoned_arc<T>(mutex: &std::sync::Arc<std::sync::Mutex<T>>) -> crate::error::Result<std::sync::MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|e| AkarError::Transaction(TransactionError::LockPoisoned(e.to_string())))
}
