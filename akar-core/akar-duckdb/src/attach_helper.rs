//! Shared DuckDB helper module for extensions that delegate to DuckDB.
//!
//! Provides a common pattern for extensions (Delta, Iceberg, Azure, Unity Catalog)
//! to open an in-memory DuckDB, install/load their extension, and execute queries.

#[cfg(feature = "bundled")]
use crate::connection::DuckDbManager;

/// Helper for extensions that delegate to DuckDB.
pub struct DuckDbAttachHelper {
    #[cfg(feature = "bundled")]
    manager: std::sync::Arc<DuckDbManager>,
}

impl DuckDbAttachHelper {
    /// Create a new helper backed by the process-wide shared in-memory DuckDB.
    #[cfg(feature = "bundled")]
    pub fn new() -> Result<Self, String> {
        let manager = DuckDbManager::shared_in_memory()?;
        Ok(Self { manager })
    }

    /// Install and load a DuckDB extension.
    #[cfg(feature = "bundled")]
    pub fn install_and_load(&self, name: &str) -> Result<(), String> {
        self.manager.install_and_load(name)
    }

    /// Execute a SQL query and return values.
    #[cfg(feature = "bundled")]
    pub fn query_rows(&self, sql: &str) -> Result<Vec<Vec<duckdb::types::Value>>, String> {
        self.manager.query_rows(sql)
    }

    /// Execute a SQL batch statement (no results).
    #[cfg(feature = "bundled")]
    pub fn execute_batch(&self, sql: &str) -> Result<(), String> {
        self.manager.execute_batch(sql)
    }

    /// Execute a SQL statement returning rows affected.
    #[cfg(feature = "bundled")]
    pub fn execute(&self, sql: &str) -> Result<usize, String> {
        self.manager.execute(sql)
    }
}

#[cfg(not(feature = "bundled"))]
impl DuckDbAttachHelper {
    pub fn new() -> Result<Self, String> {
        Err("DuckDB support not enabled".into())
    }

    pub fn install_and_load(&self, _name: &str) -> Result<(), String> {
        Err("DuckDB support not enabled".into())
    }

    pub fn query_rows(&self, _sql: &str) -> Result<Vec<Vec<String>>, String> {
        Err("DuckDB support not enabled".into())
    }

    pub fn execute_batch(&self, _sql: &str) -> Result<(), String> {
        Err("DuckDB support not enabled".into())
    }

    pub fn execute(&self, _sql: &str) -> Result<usize, String> {
        Err("DuckDB support not enabled".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(feature = "bundled"))]
    fn test_helper_new_without_bundled() {
        let result = DuckDbAttachHelper::new();
        assert!(result.is_err());
    }

    #[test]
    #[cfg(feature = "bundled")]
    fn test_helper_new_in_memory() {
        let result = DuckDbAttachHelper::new();
        assert!(result.is_ok());
    }
}
