//! Unity Catalog extension for Kuzu.
//!
//! Provides integration with Databricks Unity Catalog using DuckDB delegation.
//! Opens an in-memory DuckDB, loads the `uc_catalog` extension from the
//! DuckDB nightly build, and delegates queries to DuckDB.
//!
//! ## DuckDB delegation approach
//! Uses `kuzu-duckdb`'s `DuckDbAttachHelper`. There is no native Rust crate
//! for Unity Catalog; DuckDB's `uc_catalog` extension handles the REST API.

use kuzu_extension::{Extension, ExtensionContext};
use std::sync::Arc;

/// The Unity Catalog extension enables querying Unity Catalog from Kuzu.
pub struct UnityCatalogExtension;

impl Default for UnityCatalogExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl UnityCatalogExtension {
    pub fn new() -> Self {
        Self
    }
}

impl Extension for UnityCatalogExtension {
    fn name(&self) -> &'static str {
        "UNITY_CATALOG"
    }

    fn load(&self, context: &ExtensionContext) -> Result<(), String> {
        use kuzu_function::registry::TableFunction;

        #[cfg(feature = "duckdb-delegation")]
        {
            // uc_scan(endpoint: String, token: String, table: String) → scans a UC table
            let scan_fn: Arc<dyn Fn(&[Value], &mut kuzu_function::DataChunk) -> Result<(), String> + Send + Sync> =
                Arc::new(|args, _chunk| {
                    if args.len() < 3 {
                        return Err("uc_scan requires (endpoint, token, table) arguments".into());
                    }
                    let endpoint = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err("uc_scan: first argument must be endpoint string".into()),
                    };
                    let token = match &args[1] {
                        Value::String(s) => s.clone(),
                        _ => return Err("uc_scan: second argument must be token string".into()),
                    };
                    let table = match &args[2] {
                        Value::String(s) => s.clone(),
                        _ => return Err("uc_scan: third argument must be table name".into()),
                    };

                    let helper = kuzu_duckdb::attach_helper::DuckDbAttachHelper::new()?;
                    helper.install_and_load("uc_catalog")?;

                    let create_secret = format!(
                        "CREATE SECRET (TYPE UC, TOKEN '{}', ENDPOINT '{}')",
                        token.replace('\'', "''"),
                        endpoint.replace('\'', "''")
                    );
                    helper.execute_batch(&create_secret)?;

                    // Use DuckDB's UC catalog to read the table
                    let sql = format!("SELECT * FROM {} LIMIT 1000", table);
                    helper.query_rows(&sql)?;
                    Ok(())
                });

            context.register_table_function(
                "uc_scan",
                TableFunction::CustomTable {
                    name: "uc_scan".into(),
                    execute: scan_fn,
                },
            );

            tracing::info!("Unity Catalog extension loaded: 1 function registered (DuckDB delegation)");
        }

        #[cfg(not(feature = "duckdb-delegation"))]
        {
            context.register_table_function(
                "uc_scan",
                TableFunction::CustomTable {
                    name: "uc_scan".into(),
                    execute: Arc::new(|_, _| {
                        Err("Unity Catalog not available (feature 'duckdb-delegation' disabled)".into())
                    }),
                },
            );
            tracing::info!("Unity Catalog extension loaded (placeholder)");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uc_extension_name() {
        let ext = UnityCatalogExtension::new();
        assert_eq!(ext.name(), "UNITY_CATALOG");
    }
}
