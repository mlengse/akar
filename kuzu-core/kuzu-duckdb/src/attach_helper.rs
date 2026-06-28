//! Shared DuckDB helper module for extensions that delegate to DuckDB.
//!
//! Provides a common pattern for extensions (Delta, Iceberg, Azure, Unity Catalog)
//! to open an in-memory DuckDB, install/load their extension, and execute queries.
//!
//! Usage:
//! ```ignore
//! let mut helper = DuckDbAttachHelper::new();
//! helper.install_and_load("delta")?;
//! let rows = helper.query("SELECT * FROM delta_scan('/path/to/table')")?;
//! ```

use crate::connection::{DuckDbConfig, DuckDbManager};

/// Helper for extensions that delegate to DuckDB.
///
/// Wraps a `DuckDbManager` with in-memory configuration and provides
/// convenience methods for installing DuckDB extensions and running queries.
pub struct DuckDbAttachHelper {
    #[cfg(feature = "bundled")]
    manager: DuckDbManager,
}

impl DuckDbAttachHelper {
    /// Create a new helper with an in-memory DuckDB.
    #[cfg(feature = "bundled")]
    pub fn new() -> Result<Self, String> {
        let manager = DuckDbManager::in_memory()?;
        Ok(Self { manager })
    }

    /// Install and load a DuckDB extension.
    #[cfg(feature = "bundled")]
    pub fn install_and_load(&self, name: &str) -> Result<(), String> {
        self.manager.install_and_load(name)
    }

    /// Execute an install + load + query sequence.
    #[cfg(feature = "bundled")]
    pub fn query_with_extension(&self, extension_name: &str, sql: &str) -> Result<Vec<std::collections::HashMap<String, String>>, String> {
        self.install_and_load(extension_name)?;
        let rows = self.manager.query(sql)?;
        let mut results = Vec::new();
        for row in rows {
            let mut map = std::collections::HashMap::new();
            // DuckDB's DataRow doesn't have a column_count method directly.
            // We iterate through known columns by index.
            for i in 0..10 {
                if let Ok(val) = row.get::<_, String>(i) {
                    map.insert(format!("col_{}", i), val);
                } else if let Ok(val) = row.get::<_, i64>(i) {
                    map.insert(format!("col_{}", i), val.to_string());
                } else if let Ok(val) = row.get::<_, f64>(i) {
                    map.insert(format!("col_{}", i), val.to_string());
                } else if let Ok(val) = row.get::<_, bool>(i) {
                    map.insert(format!("col_{}", i), val.to_string());
                } else {
                    break;
                }
            }
            if !map.is_empty() {
                results.push(map);
            }
        }
        Ok(results)
    }

    /// Execute a raw SQL query on the in-memory DuckDB.
    #[cfg(feature = "bundled")]
    pub fn query(&self, sql: &str) -> Result<duckdb::ResultSet<duckdb::DataRow>, String> {
        self.manager.query(sql)
    }

    /// Execute a SQL statement (no results).
    #[cfg(feature = "bundled")]
    pub fn execute(&self, sql: &str) -> Result<u64, String> {
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

    pub fn query_with_extension(&self, _name: &str, _sql: &str) -> Result<Vec<std::collections::HashMap<String, String>>, String> {
        Err("DuckDB support not enabled".into())
    }

    pub fn query(&self, _sql: &str) -> Result<(), String> {
        Err("DuckDB support not enabled".into())
    }

    pub fn execute(&self, _sql: &str) -> Result<u64, String> {
        Err("DuckDB support not enabled".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_helper_new_without_bundled() {
        // This test only verifies the struct exists
        // Without bundled feature, new() returns Err
        let result = DuckDbAttachHelper::new();
        #[cfg(not(feature = "bundled"))]
        assert!(result.is_err());
    }
}
