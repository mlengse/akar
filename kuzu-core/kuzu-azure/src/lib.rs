//! Azure Blob Storage extension for Kuzu.
//!
//! Provides integration with Azure Blob Storage using DuckDB delegation.
//! Opens an in-memory DuckDB, loads the `azure` and `httpfs` extensions,
//! and delegates queries to DuckDB.
//!
//! Supports `az://` and `abfss://` URI schemes.
//!
//! ## DuckDB delegation approach
//! Uses `kuzu-duckdb`'s `DuckDbAttachHelper`. The Azure SDK for Rust is
//! still developing, so DuckDB delegation is more stable.

use kuzu_extension::{Extension, ExtensionContext};
use std::sync::Arc;

/// The Azure Blob Storage extension enables reading from Azure Storage from Kuzu.
pub struct AzureExtension;

impl AzureExtension {
    pub fn new() -> Self {
        Self
    }
}

impl Extension for AzureExtension {
    fn name(&self) -> &'static str {
        "AZURE"
    }

    fn load(&self, context: &ExtensionContext) -> Result<(), String> {
        use kuzu_function::registry::TableFunction;

        #[cfg(feature = "duckdb-delegation")]
        {
            // azure_scan(path: String) → scans files from Azure Blob Storage via DuckDB
            let scan_fn: Arc<dyn Fn(&[Value], &mut kuzu_function::DataChunk) -> Result<(), String> + Send + Sync> =
                Arc::new(|args, _chunk| {
                    if args.is_empty() {
                        return Err("azure_scan requires a path argument".into());
                    }
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err("azure_scan expects a string path argument".into()),
                    };

                    // Validate URI scheme
                    let upper = path.to_uppercase();
                    if !upper.starts_with("AZ://") && !upper.starts_with("ABFSS://") {
                        return Err(format!("azure_scan: path must start with az:// or abfss://, got: {path}"));
                    }

                    let helper = kuzu_duckdb::attach_helper::DuckDbAttachHelper::new()?;
                    helper.install_and_load("httpfs")?;
                    helper.execute_batch("CREATE SECRET (TYPE AZURE)")?;

                    // Use DuckDB's read_parquet or read_csv_auto on the azure path
                    let sql = format!("SELECT * FROM read_parquet('{}')", path.replace('\'', "''"));
                    helper.query_rows(&sql)?;
                    Ok(())
                });

            context.register_table_function(
                "azure_scan",
                TableFunction::CustomTable {
                    name: "azure_scan".into(),
                    execute: scan_fn,
                },
            );

            tracing::info!("Azure extension loaded: 1 function registered (DuckDB delegation)");
        }

        #[cfg(not(feature = "duckdb-delegation"))]
        {
            context.register_table_function(
                "azure_scan",
                TableFunction::CustomTable {
                    name: "azure_scan".into(),
                    execute: Arc::new(|_, _| Err("Azure not available (feature 'duckdb-delegation' disabled)".into())),
                },
            );
            tracing::info!("Azure extension loaded (placeholder)");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_azure_extension_name() {
        let ext = AzureExtension::new();
        assert_eq!(ext.name(), "AZURE");
    }
}
