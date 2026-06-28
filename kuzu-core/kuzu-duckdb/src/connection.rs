//! DuckDB connection manager.
//!
//! Manages a `duckdb::Database` and `duckdb::Connection` with
//! support for local file, in-memory, and remote HTTP/S3 modes.

#[cfg(feature = "bundled")]
use duckdb::{AccessMode, Config, Database, Connection as DuckDbRawConnection};

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

/// Manager for a DuckDB database connection.
pub struct DuckDbManager {
    #[cfg(feature = "bundled")]
    database: Database,
    #[cfg(feature = "bundled")]
    connection: DuckDbRawConnection,
    config: DuckDbConfig,
}

impl DuckDbManager {
    /// Open a DuckDB database with the given configuration.
    #[cfg(feature = "bundled")]
    pub fn open(config: DuckDbConfig) -> Result<Self, String> {
        let access_mode = if config.read_only {
            AccessMode::ReadOnly
        } else {
            AccessMode::ReadWrite
        };

        let mut duck_config = Config::default();
        duck_config
            .access_mode(access_mode)
            .map_err(|e| format!("Failed to set access mode: {e}"))?;

        if let Some(threads) = config.thread_count {
            duck_config
                .set_maximum_threads(threads as i64)
                .map_err(|e| format!("Failed to set threads: {e}"))?;
        }

        if let Some(ref max_mem) = config.max_memory {
            duck_config
                .set_max_memory(max_mem)
                .map_err(|e| format!("Failed to set max memory: {e}"))?;
        }

        let database = Database::open_with_config(&config.database_path, duck_config)
            .map_err(|e| format!("Failed to open DuckDB database: {e}"))?;

        let connection = database
            .connect()
            .map_err(|e| format!("Failed to connect to DuckDB: {e}"))?;

        Ok(Self {
            database,
            connection,
            config,
        })
    }

    /// Open an in-memory DuckDB database.
    #[cfg(feature = "bundled")]
    pub fn in_memory() -> Result<Self, String> {
        Self::open(DuckDbConfig::in_memory())
    }

    /// Execute a SQL query and return the number of rows affected.
    #[cfg(feature = "bundled")]
    pub fn execute(&self, sql: &str) -> Result<u64, String> {
        self.connection
            .execute(sql, [])
            .map_err(|e| format!("DuckDB execute error: {e}"))
    }

    /// Execute a SQL query and return the result set.
    #[cfg(feature = "bundled")]
    pub fn query(&self, sql: &str) -> Result<duckdb::ResultSet<duckdb::DataRow>, String> {
        self.connection
            .query(sql, [])
            .map_err(|e| format!("DuckDB query error: {e}"))
    }

    /// Execute a SQL query and return a DuckDB prepared statement for chunked access.
    #[cfg(feature = "bundled")]
    pub fn prepare(&self, sql: &str) -> Result<duckdb::Statement<'_>, String> {
        self.connection
            .prepare(sql)
            .map_err(|e| format!("DuckDB prepare error: {e}"))
    }

    /// Install and load a DuckDB extension.
    #[cfg(feature = "bundled")]
    pub fn install_and_load(&self, extension_name: &str) -> Result<(), String> {
        self.execute(&format!("INSTALL {}; LOAD {};", extension_name, extension_name))?;
        Ok(())
    }

    /// Get the connection (for advanced operations).
    #[cfg(feature = "bundled")]
    pub fn connection(&self) -> &DuckDbRawConnection {
        &self.connection
    }

    /// Get the config.
    pub fn config(&self) -> &DuckDbConfig {
        &self.config
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
        let count: i64 = manager
            .query("SELECT 1 + 1")
            .unwrap()
            .into_iter()
            .next()
            .and_then(|row| row.get::<_, i64>(0).ok())
            .unwrap_or(0);
        assert_eq!(count, 2);
    }

    #[test]
    #[cfg(feature = "bundled")]
    fn test_execute() {
        let manager = DuckDbManager::in_memory().unwrap();
        manager.execute("CREATE TABLE test (id INTEGER, name VARCHAR)").unwrap();
        let affected = manager.execute("INSERT INTO test VALUES (1, 'hello'), (2, 'world')").unwrap();
        assert_eq!(affected, 2);
    }
}
