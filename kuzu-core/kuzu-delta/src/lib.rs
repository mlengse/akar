//! Delta Lake extension for Kuzu.
//!
//! Provides integration with Delta Lake tables.
//! - **Native mode** (feature `native`): Reads Delta transaction log and enumerates data files.
//! - **DuckDB delegation** (feature `duckdb-delegation`): Delegates to DuckDB's delta extension.

#[cfg(feature = "native")]
mod native_reader;

use kuzu_extension::{Extension, ExtensionContext};
use std::sync::Arc;

#[cfg(any(feature = "native", feature = "duckdb-delegation"))]
use kuzu_function::Value;

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

        #[cfg(feature = "native")]
        {
            let scan_fn: Arc<dyn Fn(&[Value], &mut kuzu_function::DataChunk) -> Result<(), String> + Send + Sync> =
                Arc::new(|args, chunk| {
                    if args.is_empty() {
                        return Err("delta_scan requires a path argument".into());
                    }
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err("delta_scan expects a string path argument".into()),
                    };

                    if chunk.size > 0 {
                        return Ok(());
                    }

                    let table_info = native_reader::load_delta_table(&path)?;
                    let file_refs: Vec<&str> = table_info.data_files.iter().map(|s| s.as_str()).collect();

                    let array = std::sync::Arc::new(arrow::array::StringArray::from(file_refs)) as arrow::array::ArrayRef;

                    chunk.fields.clear();
                    chunk.field_types.clear();
                    chunk.field_names.clear();
                    chunk.fields.push(array);
                    chunk.field_types.push(kuzu_common::types::PhysicalTypeID::String);
                    chunk.field_names.push("file_path".to_string());
                    chunk.size = table_info.data_files.len();
                    Ok(())
                });

            context.register_table_function(
                "delta_scan",
                TableFunction::CustomTable {
                    name: "delta_scan".into(),
                    execute: scan_fn,
                },
            );

            tracing::info!("Delta extension loaded: 1 function registered (native reader)");
        }

        #[cfg(not(feature = "native"))]
        {
            #[cfg(feature = "duckdb-delegation")]
            {
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
                        execute: Arc::new(|_, _| {
                            Err("Delta not available (enable feature 'native' or 'duckdb-delegation')".into())
                        }),
                    },
                );
                tracing::info!("Delta extension loaded (placeholder)");
            }
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
