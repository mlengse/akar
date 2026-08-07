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
//! ├── Binder(BinderError)
//! ├── Planner(PlannerError)
//! ├── Processor(ProcessorError)
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
    Binder(BinderError),
    /// Logical planner failure
    Planner(PlannerError),
    /// Query processor / execution failure
    Processor(ProcessorError),
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
            Self::Binder(e) => write!(f, "binder: {e}"),
            Self::Planner(e) => write!(f, "planner: {e}"),
            Self::Processor(e) => write!(f, "processor: {e}"),
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
            Self::Binder(e) => Some(e),
            Self::Planner(e) => Some(e),
            Self::Processor(e) => Some(e),
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

impl From<BinderError> for AkarError {
    fn from(e: BinderError) -> Self {
        Self::Binder(e)
    }
}

impl From<PlannerError> for AkarError {
    fn from(e: PlannerError) -> Self {
        Self::Planner(e)
    }
}

impl From<ProcessorError> for AkarError {
    fn from(e: ProcessorError) -> Self {
        Self::Processor(e)
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
    /// Row-level write conflict: another active transaction modified the same row
    WriteConflict {
        table_id: u64,
        row_id: u64,
        conflicting_txn: u64,
    },
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
            Self::WriteConflict {
                table_id,
                row_id,
                conflicting_txn,
            } => {
                write!(
                    f,
                    "write conflict on table {table_id} row {row_id}: txn#{conflicting_txn} also modified this row"
                )
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
// Binder errors
// ---------------------------------------------------------------------------

/// Errors originating from the binder (semantic analysis).
#[derive(Debug)]
pub enum BinderError {
    /// Table not found in catalog
    TableNotFound(String),
    /// Column not found in table
    ColumnNotFound { table: String, column: String },
    /// Variable not in scope
    VariableNotInScope(String),
    /// Type not recognized
    UnknownType(String),
    /// Validation error (general)
    Validation(String),
    /// I/O error during import
    Io(String),
}

impl fmt::Display for BinderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TableNotFound(s) => write!(f, "table not found: {s}"),
            Self::ColumnNotFound { table, column } => {
                write!(f, "column '{column}' not found in table '{table}'")
            }
            Self::VariableNotInScope(s) => write!(f, "variable not in scope: {s}"),
            Self::UnknownType(s) => write!(f, "unknown type: {s}"),
            Self::Validation(s) => write!(f, "{s}"),
            Self::Io(s) => write!(f, "I/O error: {s}"),
        }
    }
}

impl std::error::Error for BinderError {}

/// Allow `?` in functions still returning `Result<T, String>`.
impl From<BinderError> for String {
    fn from(e: BinderError) -> String {
        format!("binder: {e}")
    }
}

/// Allow `Err("message".into())` patterns during incremental migration.
impl From<String> for BinderError {
    fn from(s: String) -> Self {
        BinderError::Validation(s)
    }
}

/// Allow `Err("literal".into())` patterns.
impl From<&str> for BinderError {
    fn from(s: &str) -> Self {
        BinderError::Validation(s.to_string())
    }
}

/// Allow catalog errors to propagate through the binder.
impl From<CatalogError> for BinderError {
    fn from(e: CatalogError) -> Self {
        match e {
            CatalogError::AlreadyExists(s) => BinderError::Validation(format!("already exists: {s}")),
            CatalogError::NotFound(s) => BinderError::TableNotFound(s),
            CatalogError::ColumnAlreadyExists { table, column } => {
                BinderError::Validation(format!("column '{column}' already exists on table '{table}'"))
            }
            CatalogError::ColumnNotFound { table, column } => BinderError::ColumnNotFound { table, column },
            CatalogError::InvalidOperation(s) => BinderError::Validation(s),
        }
    }
}

// ---------------------------------------------------------------------------
// Planner errors
// ---------------------------------------------------------------------------

/// Errors originating from the logical planner.
#[derive(Debug)]
pub enum PlannerError {
    /// Planning failure (general)
    Planning(String),
}

impl fmt::Display for PlannerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Planning(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for PlannerError {}

/// Allow `?` in functions still returning `Result<T, String>`.
impl From<PlannerError> for String {
    fn from(e: PlannerError) -> String {
        format!("planner: {e}")
    }
}

/// Allow `Err("message".into())` patterns during incremental migration.
impl From<String> for PlannerError {
    fn from(s: String) -> Self {
        PlannerError::Planning(s)
    }
}

/// Allow `Err("literal".into())` patterns.
impl From<&str> for PlannerError {
    fn from(s: &str) -> Self {
        PlannerError::Planning(s.to_string())
    }
}

// ---------------------------------------------------------------------------
// Processor errors
// ---------------------------------------------------------------------------

/// Errors originating from the query processor / execution engine.
#[derive(Debug)]
pub enum ProcessorError {
    /// Expression evaluation failure
    Expression(String),
    /// Execution failure (general)
    Execution(String),
    /// I/O error during execution
    Io(String),
}

impl fmt::Display for ProcessorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expression(s) => write!(f, "expression: {s}"),
            Self::Execution(s) => write!(f, "{s}"),
            Self::Io(s) => write!(f, "I/O error: {s}"),
        }
    }
}

impl std::error::Error for ProcessorError {}

/// Allow `?` in functions still returning `Result<T, String>`.
impl From<ProcessorError> for String {
    fn from(e: ProcessorError) -> String {
        format!("processor: {e}")
    }
}

/// Allow `Err("message".into())` patterns during incremental migration.
impl From<String> for ProcessorError {
    fn from(s: String) -> Self {
        ProcessorError::Execution(s)
    }
}

/// Allow `Err("literal".into())` patterns.
impl From<&str> for ProcessorError {
    fn from(s: &str) -> Self {
        ProcessorError::Execution(s.to_string())
    }
}

/// Allow `?` on StorageError in functions returning `Result<T, ProcessorError>`.
impl From<StorageError> for ProcessorError {
    fn from(e: StorageError) -> Self {
        ProcessorError::Execution(format!("storage: {e}"))
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
