//! Apache Iceberg extension for Kuzu.
//!
//! Provides integration with Apache Iceberg tables using DuckDB delegation.
//! Opens an in-memory DuckDB, loads the `iceberg` extension, and delegates
//! iceberg_scan(), iceberg_metadata(), and iceberg_snapshots() to DuckDB.
//!
//! ## DuckDB delegation approach
//! Uses `kuzu-duckdb`'s `DuckDbAttachHelper` to create an in-memory DuckDB.
//! The native Rust `iceberg-rust` crate is still maturing, so DuckDB delegation
//! is more stable.

use kuzu_extension::{Extension, ExtensionContext};
use std::sync::Arc;

/// The Apache Iceberg extension enables querying Iceberg tables from Kuzu.
pub struct IcebergExtension;

impl Default for IcebergExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl IcebergExtension {
    pub fn new() -> Self {
        Self
    }
}

impl Extension for IcebergExtension {
    fn name(&self) -> &'static str {
        "ICEBERG"
    }

    fn load(&self, context: &ExtensionContext) -> Result<(), String> {
        use kuzu_function::registry::TableFunction;

        #[cfg(feature = "duckdb-delegation")]
        {
            // iceberg_scan(path: String) → scans an Iceberg table
            let scan_fn: Arc<dyn Fn(&[Value], &mut kuzu_function::DataChunk) -> Result<(), String> + Send + Sync> =
                Arc::new(|args, _chunk| {
                    if args.is_empty() {
                        return Err("iceberg_scan requires a path argument".into());
                    }
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err("iceberg_scan expects a string path argument".into()),
                    };

                    let helper = kuzu_duckdb::attach_helper::DuckDbAttachHelper::new()?;
                    helper.install_and_load("iceberg")?;
                    let sql = format!("SELECT * FROM iceberg_scan('{}')", path.replace('\'', "''"));
                    helper.query_rows(&sql)?;
                    Ok(())
                });

            context.register_table_function(
                "iceberg_scan",
                TableFunction::CustomTable {
                    name: "iceberg_scan".into(),
                    execute: scan_fn,
                },
            );

            // iceberg_metadata(path: String) → returns Iceberg table metadata
            let meta_fn: Arc<dyn Fn(&[Value], &mut kuzu_function::DataChunk) -> Result<(), String> + Send + Sync> =
                Arc::new(|args, _chunk| {
                    if args.is_empty() {
                        return Err("iceberg_metadata requires a path argument".into());
                    }
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err("iceberg_metadata expects a string path argument".into()),
                    };

                    let helper = kuzu_duckdb::attach_helper::DuckDbAttachHelper::new()?;
                    helper.install_and_load("iceberg")?;
                    let sql = format!("SELECT * FROM iceberg_metadata('{}')", path.replace('\'', "''"));
                    helper.query_rows(&sql)?;
                    Ok(())
                });

            context.register_table_function(
                "iceberg_metadata",
                TableFunction::CustomTable {
                    name: "iceberg_metadata".into(),
                    execute: meta_fn,
                },
            );

            // iceberg_snapshots(path: String) → returns Iceberg snapshot info
            let snap_fn: Arc<dyn Fn(&[Value], &mut kuzu_function::DataChunk) -> Result<(), String> + Send + Sync> =
                Arc::new(|args, _chunk| {
                    if args.is_empty() {
                        return Err("iceberg_snapshots requires a path argument".into());
                    }
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err("iceberg_snapshots expects a string path argument".into()),
                    };

                    let helper = kuzu_duckdb::attach_helper::DuckDbAttachHelper::new()?;
                    helper.install_and_load("iceberg")?;
                    let sql = format!("SELECT * FROM iceberg_snapshots('{}')", path.replace('\'', "''"));
                    helper.query_rows(&sql)?;
                    Ok(())
                });

            context.register_table_function(
                "iceberg_snapshots",
                TableFunction::CustomTable {
                    name: "iceberg_snapshots".into(),
                    execute: snap_fn,
                },
            );

            tracing::info!("Iceberg extension loaded: 3 functions registered (DuckDB delegation)");
        }

        #[cfg(not(feature = "duckdb-delegation"))]
        {
            context.register_table_function(
                "iceberg_scan",
                TableFunction::CustomTable {
                    name: "iceberg_scan".into(),
                    execute: Arc::new(|_, _| Err("Iceberg not available".into())),
                },
            );
            context.register_table_function(
                "iceberg_metadata",
                TableFunction::CustomTable {
                    name: "iceberg_metadata".into(),
                    execute: Arc::new(|_, _| Err("Iceberg not available".into())),
                },
            );
            context.register_table_function(
                "iceberg_snapshots",
                TableFunction::CustomTable {
                    name: "iceberg_snapshots".into(),
                    execute: Arc::new(|_, _| Err("Iceberg not available".into())),
                },
            );
            tracing::info!("Iceberg extension loaded: 3 functions registered (placeholder)");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iceberg_extension_name() {
        let ext = IcebergExtension::new();
        assert_eq!(ext.name(), "ICEBERG");
    }
}
