//! Connection — used to execute queries against a Database.
//!
//! Manages the full query lifecycle: parse → bind → plan → optimize → execute.
//! DDL statements (CREATE/DROP TABLE) update the catalog directly and return
//! a message result. DML statements (MATCH/RETURN) produce DataChunk results.
//!
//! Supports prepared statements via `prepare()` and `execute()` for
//! parameterized queries.
//!
//! # Concurrent Multi-Writer Support
//!
//! Write transactions use `TransactionManager::begin_write()` / `commit()` /
//! `rollback()` with per-transaction `LocalStorage`, `LocalWAL`, and
//! `ShadowFile` resources held in `txn_resources`. The full commit pipeline
//! flushes: LocalStorage → tables, LocalWAL → global WAL, ShadowFile → BM.

pub mod copy;
pub mod ddl;
pub mod dml;
pub mod query;
pub mod substitute;
pub mod transaction;
pub mod utils;

use crate::database::Database;
use crate::prepared_statement::PreparedStatement;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Per-transaction resources held during an active write transaction.
pub(crate) struct TxnResources {
    pub local_storage: kuzu_storage::LocalStorage,
    pub local_wal: kuzu_storage::LocalWAL,
    pub shadow_file: kuzu_storage::ShadowFile,
}

/// A connection to the database for executing Cypher queries.
///
/// Created via [`Connection::new`] with a reference to a [`Database`].
/// Supports both ad-hoc queries and prepared statements.
///
/// # Examples
///
/// ```no_run
/// use kuzu_main::database::{Database, SystemConfig};
/// use kuzu_main::connection::Connection;
///
/// let db = std::sync::Arc::new(Database::new("./my_db", SystemConfig::default())?);
/// let conn = Connection::new(&db);
///
/// // DDL
/// conn.query("CREATE NODE TABLE Person(name STRING, PRIMARY KEY(name))")?;
///
/// // DML
/// conn.query("CREATE (:Person {name: 'Alice'})")?;
///
/// // Query
/// let result = conn.query("MATCH (p:Person) RETURN p.name")?;
/// assert!(result.success);
/// # Ok::<(), String>(())
/// ```
pub struct Connection {
    pub(crate) database: Arc<Database>,
    /// Cache of prepared statements (query → PreparedStatement).
    pub(crate) statement_cache: Mutex<HashMap<String, PreparedStatement>>,
    /// Per-transaction resources keyed by transaction ID.
    /// Set up when `begin_write()` is called, cleaned up on commit/rollback.
    pub(crate) txn_resources: Mutex<HashMap<u64, TxnResources>>,
}

impl Connection {
    pub fn new(database: &Arc<Database>) -> Self {
        Self {
            database: database.clone(),
            statement_cache: Mutex::new(HashMap::new()),
            txn_resources: Mutex::new(HashMap::new()),
        }
    }

    /// Clear the prepared statement cache.
    pub fn clear_cache(&self) {
        if let Ok(mut cache) = self.statement_cache.lock() {
            cache.clear();
        }
    }

    /// Number of cached prepared statements.
    pub fn cache_size(&self) -> usize {
        self.statement_cache.lock().map(|c| c.len()).unwrap_or(0)
    }
}
