//! Delta Lake extension for Kuzu.
//!
//! Provides integration with Delta Lake tables using DuckDB delegation.
//! Opens an in-memory DuckDB, loads the `delta` extension, and delegates
//! delta_scan() queries to DuckDB.
//!
//! ## DuckDB delegation approach
//! Uses `kuzu-duckdb`'s `DuckDbAttachHelper` to create an in-memory DuckDB,
//! install the `delta` extension, and execute queries. The native Rust
//! `deltalake` crate API is still maturing, so DuckDB delegation is more stable.

use kuzu_extension::{Extension, ExtensionContext};
use std::sync::Arc;
use kuzu_common::types::Value;

/// The Delta Lake extension enables querying Delta tables from Kuzu.
pub struct DeltaExtension;

impl Default for DeltaExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl DeltaExtension {
    pub fn new() -> Self {
        Self
    }
}

impl Extension for DeltaExtension {
    fn name(&self) -> &'static str {
        "DELTA"
    }

    fn load(&self, context: &ExtensionContext) -> Result<(), String> {
        use kuzu_function::registry::TableFunction;

        #[cfg(feature = "duckdb-delegation")]
        {
            // delta_scan(path: String) → scans a Delta table via DuckDB
            let scan_fn: Arc<dyn Fn(&[Value], &mut kuzu_function::DataChunk) -> Result<(), String> + Send + Sync> =
                Arc::new(|args, _chunk| {
                    if args.is_empty() {
                        return Err("delta_scan requires a path argument".into());
                    }
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err("delta_scan expects a string path argument".into()),
                    };

                    let helper = kuzu_duckdb::attach_helper::DuckDbAttachHelper::new()?;
                    helper.install_and_load("delta")?;

                    let sql = format!("SELECT * FROM delta_scan('{}')", path.replace('\'', "''"));
                    helper.query_rows(&sql)?;

                    Ok(())
                });

            context.register_table_function(
                "delta_scan",
                TableFunction::CustomTable {
                    name: "delta_scan".into(),
                    execute: scan_fn,
                },
            );

            tracing::info!("Delta extension loaded: 1 function registered (DuckDB delegation)");
        }

        #[cfg(not(feature = "duckdb-delegation"))]
        {
            context.register_table_function(
                "delta_scan",
                TableFunction::CustomTable {
                    name: "delta_scan".into(),
                    execute: Arc::new(|_, _| Err("Delta not available (feature 'duckdb-delegation' disabled)".into())),
                },
            );
            tracing::info!("Delta extension loaded (placeholder)");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delta_extension_name() {
        let ext = DeltaExtension::new();
        assert_eq!(ext.name(), "DELTA");
    }
}
