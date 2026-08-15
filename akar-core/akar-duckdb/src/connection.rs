//! DuckDB connection manager.
//!
//! Manages a DuckDB connection with support for local file,
//! in-memory, and remote HTTP/S3 modes.

#[cfg(feature = "bundled")]
use std::sync::{Mutex, OnceLock};

#[cfg(feature = "bundled")]
use duckdb::{AccessMode, Config, Connection as DuckDbConn};

/// The three modes of DuckDB operation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DuckDbMode {
    /// Local file-based database.
    Local,
    /// In-memory database (no persistence).
    InMemory,
    /// Remote HTTP/S3 database (requires httpfs extension).
    Remote,
}

/// Configuration for opening a DuckDB database.
#[derive(Debug, Clone)]
pub struct DuckDbConfig {
    pub database_path: String,
    pub mode: DuckDbMode,
    pub read_only: bool,
    pub thread_count: Option<u32>,
    pub max_memory: Option<String>,
}

impl Default for DuckDbConfig {
    fn default() -> Self {
        Self {
            database_path: ":memory:".into(),
            mode: DuckDbMode::InMemory,
            read_only: false,
            thread_count: None,
            max_memory: None,
        }
    }
}

impl DuckDbConfig {
    pub fn new(path: &str) -> Self {
        Self {
            database_path: path.to_string(),
            ..Default::default()
        }
    }

    pub fn in_memory() -> Self {
        Self {
            database_path: ":memory:".into(),
            mode: DuckDbMode::InMemory,
            read_only: false,
            thread_count: None,
            max_memory: None,
        }
    }

    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    pub fn with_threads(mut self, threads: u32) -> Self {
        self.thread_count = Some(threads);
        self
    }

    pub fn with_max_memory(mut self, max_memory: &str) -> Self {
        self.max_memory = Some(max_memory.to_string());
        self
    }

    pub fn with_mode(mut self, mode: DuckDbMode) -> Self {
        self.mode = mode;
        self
    }
}

/// Process-wide shared in-memory DuckDB instance.
#[cfg(feature = "bundled")]
static SHARED_IN_MEMORY: OnceLock<Result<std::sync::Arc<DuckDbManager>, String>> = OnceLock::new();

/// Manager for a DuckDB connection.
pub struct DuckDbManager {
    #[cfg(feature = "bundled")]
    connection: Mutex<DuckDbConn>,
}

impl DuckDbManager {
    /// Return the process-wide shared in-memory DuckDB instance.
    ///
    /// Reuses a single DuckDB instance (catalog + thread pool) across every
    /// `duckdb_query`/`duckdb_scan` and DuckDB-delegated extension call instead
    /// of booting a fresh instance per call (P52.32).
    #[cfg(feature = "bundled")]
    pub fn shared_in_memory() -> Result<std::sync::Arc<DuckDbManager>, String> {
        SHARED_IN_MEMORY
            .get_or_init(|| Self::in_memory().map(std::sync::Arc::new))
            .clone()
    }

    /// Open a DuckDB database with the given configuration.
    #[cfg(feature = "bundled")]
    pub fn open(config: DuckDbConfig) -> Result<Self, String> {
        let access_mode = if config.read_only {
            AccessMode::ReadOnly
        } else {
            AccessMode::ReadWrite
        };

        let mut duck_config = Config::default();
        duck_config = duck_config
            .access_mode(access_mode)
            .map_err(|e| format!("Failed to set access mode: {e}"))?;

        if let Some(threads) = config.thread_count {
            duck_config = duck_config
                .threads(threads as i64)
                .map_err(|e| format!("Failed to set threads: {e}"))?;
        }

        if let Some(ref max_mem) = config.max_memory {
            duck_config = duck_config
                .max_memory(max_mem)
                .map_err(|e| format!("Failed to set max memory: {e}"))?;
        }

        let connection = if config.database_path == ":memory:" {
            DuckDbConn::open_in_memory_with_flags(duck_config)
        } else {
            DuckDbConn::open_with_flags(&config.database_path, duck_config)
        }
        .map_err(|e| format!("Failed to open DuckDB: {e}"))?;

        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Open an in-memory DuckDB database.
    #[cfg(feature = "bundled")]
    pub fn in_memory() -> Result<Self, String> {
        let connection = DuckDbConn::open_in_memory().map_err(|e| format!("Failed to open in-memory DuckDB: {e}"))?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Execute a SQL statement and return the number of rows affected.
    #[cfg(feature = "bundled")]
    pub fn execute(&self, sql: &str) -> Result<usize, String> {
        let conn = self.connection.lock().map_err(|e| format!("Lock poisoned: {e}"))?;
        conn.execute(sql, []).map_err(|e| format!("DuckDB execute error: {e}"))
    }

    /// Execute a SQL statement (batch, no return).
    #[cfg(feature = "bundled")]
    pub fn execute_batch(&self, sql: &str) -> Result<(), String> {
        let conn = self.connection.lock().map_err(|e| format!("Lock poisoned: {e}"))?;
        conn.execute_batch(sql)
            .map_err(|e| format!("DuckDB execute batch error: {e}"))
    }

    /// Install and load a DuckDB extension.
    #[cfg(feature = "bundled")]
    pub fn install_and_load(&self, name: &str) -> Result<(), String> {
        let sql = format!("INSTALL '{}'; LOAD '{}';", name, name);
        let conn = self.connection.lock().map_err(|e| format!("Lock poisoned: {e}"))?;
        conn.execute_batch(&sql)
            .map_err(|e| format!("Failed to install/load DuckDB extension '{name}': {e}"))
    }

    /// Query rows using a prepared statement.
    #[cfg(feature = "bundled")]
    pub fn query_rows(&self, sql: &str) -> Result<Vec<Vec<duckdb::types::Value>>, String> {
        let conn = self.connection.lock().map_err(|e| format!("Lock poisoned: {e}"))?;
        let mut stmt = match conn.prepare(sql) {
            Ok(s) => s,
            Err(e) => return Err(format!("DuckDB prepare error: {e}")),
        };
        let col_count = stmt.column_count();
        let mut rows = match stmt.query([]) {
            Ok(r) => r,
            Err(e) => return Err(format!("DuckDB query error: {e}")),
        };

        let mut results = Vec::new();
        while let Some(row) = rows.next().map_err(|e| format!("DuckDB row error: {e}"))? {
            let mut values = Vec::with_capacity(col_count);
            for i in 0..col_count {
                let val: duckdb::types::Value = row
                    .get::<_, duckdb::types::Value>(i)
                    .unwrap_or(duckdb::types::Value::Null);
                values.push(val);
            }
            results.push(values);
        }
        Ok(results)
    }
}

#[cfg(not(feature = "bundled"))]
impl DuckDbManager {
    pub fn open(_config: DuckDbConfig) -> Result<Self, String> {
        Err("DuckDB support not enabled (feature 'bundled' not active)".into())
    }

    pub fn in_memory() -> Result<Self, String> {
        Err("DuckDB support not enabled (feature 'bundled' not active)".into())
    }

    pub fn execute(&self, _sql: &str) -> Result<usize, String> {
        Err("DuckDB support not enabled".into())
    }

    pub fn execute_batch(&self, _sql: &str) -> Result<(), String> {
        Err("DuckDB support not enabled".into())
    }

    pub fn query_rows(&self, _sql: &str) -> Result<Vec<Vec<String>>, String> {
        Err("DuckDB support not enabled".into())
    }

    pub fn install_and_load(&self, _name: &str) -> Result<(), String> {
        Err("DuckDB support not enabled".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let cfg = DuckDbConfig::default();
        assert_eq!(cfg.database_path, ":memory:");
        assert_eq!(cfg.mode, DuckDbMode::InMemory);
        assert!(!cfg.read_only);
    }

    #[test]
    fn test_config_custom() {
        let cfg = DuckDbConfig::new("/data/mydb.duckdb")
            .with_read_only(false)
            .with_threads(4);
        assert_eq!(cfg.database_path, "/data/mydb.duckdb");
        assert!(!cfg.read_only);
        assert_eq!(cfg.thread_count, Some(4));
    }

    #[test]
    #[cfg(feature = "bundled")]
    fn test_open_in_memory() {
        let manager = DuckDbManager::in_memory().unwrap();
        let rows = manager.query_rows("SELECT 1 + 1").unwrap();
        assert!(!rows.is_empty());
        if let Some(first) = rows.first() {
            if let Some(val) = first.first() {
                match val {
                    duckdb::types::Value::Int(i) => assert_eq!(*i, 2),
                    _ => panic!("Expected Int(2), got {:?}", val),
                }
            }
        }
    }

    #[test]
    #[cfg(feature = "bundled")]
    fn test_execute() {
        let manager = DuckDbManager::in_memory().unwrap();
        manager.execute("CREATE TABLE test (id INTEGER, name VARCHAR)").unwrap();
        let affected = manager
            .execute("INSERT INTO test VALUES (1, 'hello'), (2, 'world')")
            .unwrap();
        assert_eq!(affected, 2);
    }

    #[test]
    #[cfg(feature = "bundled")]
    fn test_shared_in_memory_is_singleton() {
        let a = DuckDbManager::shared_in_memory().unwrap();
        let b = DuckDbManager::shared_in_memory().unwrap();
        assert!(
            std::sync::Arc::ptr_eq(&a, &b),
            "shared instance must be reused, not recreated"
        );
        a.execute("CREATE TABLE IF NOT EXISTS shared_t (id INTEGER)").unwrap();
        b.execute("INSERT INTO shared_t VALUES (42)").unwrap();
        let rows = b.query_rows("SELECT COUNT(*) FROM shared_t").unwrap();
        assert_eq!(rows.len(), 1);
    }
}
