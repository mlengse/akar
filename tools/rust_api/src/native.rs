//! Native Rust backend for Kuzu (using kuzu-core).
//!
//! This module provides the same public API as the C++ FFI backend,
//! but implemented entirely in Rust via `kuzu-main` and `kuzu-common`.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

/// Re-export core types from kuzu-core.
pub use kuzu_main::{
    Connection as RawConnection, Database as RawDatabase, PreparedStatement, QueryResult,
};
pub use kuzu_main::SystemConfig;

/// Re-export common value types.
pub use kuzu_common::types::{InternalID, LogicalTypeID, Value};

/// The version of the Kuzu library.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Get the storage version number.
pub fn get_storage_version() -> u64 {
    0 // Native Rust storage version
}

// ==================== Backward-compatible Error type ====================

/// Error type for Kuzu operations.
///
/// Wraps a string message for backward compatibility with the C++ FFI API
/// where operations returned `Result<_, Error>` instead of `Result<_, String>`.
#[derive(Debug, Clone)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Error {}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error(s)
    }
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error(s.to_string())
    }
}

// ==================== Database wrapper ====================

/// The main database instance.
///
/// Wraps [`kuzu_main::Database`] for backward-compatible API.
pub struct Database {
    pub(crate) inner: Arc<RawDatabase>,
}

impl Database {
    /// Create a new database at the given path.
    ///
    /// # Arguments
    /// * `db_path` - Path to the database directory.
    /// * `config` - Configuration options.
    pub fn new(db_path: impl Into<PathBuf>, config: SystemConfig) -> Result<Self, Error> {
        let db = RawDatabase::new(db_path, config).map_err(Error)?;
        Ok(Database {
            inner: Arc::new(db),
        })
    }
}

// ==================== Connection wrapper ====================

/// A connection to the database for executing queries.
///
/// Wraps [`kuzu_main::Connection`] for backward-compatible API
/// (returns `Result<Self, Error>` instead of `Self`).
pub struct Connection {
    inner: kuzu_main::Connection,
}

impl Connection {
    /// Creates a connection to the database.
    ///
    /// # Arguments
    /// * `database` - A reference to the database instance.
    pub fn new(database: &Database) -> Result<Self, Error> {
        Ok(Connection {
            inner: kuzu_main::Connection::new(&database.inner),
        })
    }

    /// Execute a Cypher query and return the result.
    pub fn query(&self, query_str: &str) -> Result<QueryResult, Error> {
        self.inner.query(query_str).map_err(Error)
    }

    /// Prepare a query for parameterized execution.
    pub fn prepare(&self, query_str: &str) -> Result<PreparedStatement, Error> {
        self.inner.prepare(query_str).map_err(Error)
    }

    /// Execute a prepared statement with the given parameter values.
    pub fn execute(
        &self,
        prepared: &PreparedStatement,
        params: Vec<(&str, Value)>,
    ) -> Result<QueryResult, Error> {
        self.inner.execute(prepared, params).map_err(Error)
    }
}
