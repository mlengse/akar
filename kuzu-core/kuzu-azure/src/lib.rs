//! Azure Blob Storage extension for Kuzu.
//!
//! Provides integration with Azure Blob Storage.
//! - **Native mode** (feature `native`): Downloads blobs via Azure REST API using `ureq`.
//! - **DuckDB delegation** (feature `duckdb-delegation`): Delegates to DuckDB's httpfs extension.
//!
//! Supports `az://` and `abfss://` URI schemes.

use kuzu_extension::{Extension, ExtensionContext};
use std::sync::Arc;

#[cfg(feature = "native")]
mod azure_storage;

#[cfg(feature = "native")]
use azure_storage::download_blob;

#[cfg(any(feature = "native", feature = "duckdb-delegation"))]
use kuzu_function::Value;

/// The Azure Blob Storage extension enables reading from Azure Storage from Kuzu.
pub struct AzureExtension;

impl Default for AzureExtension {
    fn default() -> Self {
        Self::new()
    }
}

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

        #[cfg(feature = "native")]
        {
            let scan_fn: Arc<dyn Fn(&[Value], &mut kuzu_function::DataChunk) -> Result<(), String> + Send + Sync> =
                Arc::new(|args, chunk| {
                    if args.is_empty() {
                        return Err("azure_scan requires a path argument".into());
                    }
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err("azure_scan expects a string path argument".into()),
                    };

                    let upper = path.to_uppercase();
                    if !upper.starts_with("AZ://") && !upper.starts_with("ABFSS://") {
                        return Err(format!(
                            "azure_scan: path must start with az:// or abfss://, got: {path}"
                        ));
                    }

                    if chunk.size > 0 {
                        return Ok(());
                    }

                    let local_path = download_blob(&path)?;

                    let array = std::sync::Arc::new(
                        arrow::array::StringArray::from(vec![local_path.as_str()])
                    ) as arrow::array::ArrayRef;

                    chunk.fields.clear();
                    chunk.field_types.clear();
                    chunk.field_names.clear();
                    chunk.fields.push(array);
                    chunk.field_types.push(kuzu_common::types::PhysicalTypeID::String);
                    chunk.field_names.push("path".to_string());
                    chunk.resize(1);
                    Ok(())
                });

            context.register_table_function(
                "azure_scan",
                TableFunction::CustomTable {
                    name: "azure_scan".into(),
                    execute: scan_fn,
                },
            );

            tracing::info!("Azure extension loaded: 1 function registered (native reader)");
        }

        #[cfg(not(feature = "native"))]
        {
            #[cfg(feature = "duckdb-delegation")]
            {
                let scan_fn: Arc<dyn Fn(&[Value], &mut kuzu_function::DataChunk) -> Result<(), String> + Send + Sync> =
                    Arc::new(|args, _chunk| {
                        if args.is_empty() {
                            return Err("azure_scan requires a path argument".into());
                        }
                        let path = match &args[0] {
                            Value::String(s) => s.clone(),
                            _ => return Err("azure_scan expects a string path argument".into()),
                        };

                        let upper = path.to_uppercase();
                        if !upper.starts_with("AZ://") && !upper.starts_with("ABFSS://") {
                            return Err(format!(
                                "azure_scan: path must start with az:// or abfss://, got: {path}"
                            ));
                        }

                        let helper = kuzu_duckdb::attach_helper::DuckDbAttachHelper::new()?;
                        helper.install_and_load("httpfs")?;
                        helper.execute_batch("CREATE SECRET (TYPE AZURE)")?;

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
                        execute: Arc::new(|_, _| {
                            Err("Azure not available (enable feature 'native' or 'duckdb-delegation')".into())
                        }),
                    },
                );
                tracing::info!("Azure extension loaded (placeholder)");
            }
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
