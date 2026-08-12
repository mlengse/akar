//! Apache Iceberg extension for Akar.
//!
//! Provides integration with Apache Iceberg tables.
//! - **Native mode** (feature `native`): Reads Iceberg metadata and enumerates data files.
//! - **DuckDB delegation** (feature `duckdb-delegation`): Delegates to DuckDB's iceberg extension.

#[cfg(feature = "native")]
mod native_reader;

#[cfg(feature = "native")]
mod avro;

use akar_extension::{Extension, ExtensionContext};
use std::sync::Arc;

#[cfg(any(feature = "native", feature = "duckdb-delegation"))]
use akar_function::Value;

/// The Apache Iceberg extension enables querying Iceberg tables from Akar.
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
        use akar_function::registry::TableFunction;

        #[cfg(feature = "native")]
        {
            // iceberg_scan(path: String) → lists data files and returns their paths
            let scan_fn: Arc<dyn Fn(&[Value], &mut akar_function::DataChunk) -> Result<(), String> + Send + Sync> =
                Arc::new(|args, chunk| {
                    if args.is_empty() {
                        return Err("iceberg_scan requires a path argument".into());
                    }
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err("iceberg_scan expects a string path argument".into()),
                    };

                    if chunk.size > 0 {
                        return Ok(());
                    }

                    // Load metadata and list the data files referenced by the
                    // current snapshot (excludes compacted/deleted files).
                    let _meta = native_reader::IcebergMetadata::load(&path)?;
                    let data_files = native_reader::list_active_data_files(&path)?;

                    akar_common::extension_utils::fill_chunk_with_strings(chunk, "file_path", &data_files);
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
            let meta_fn: Arc<dyn Fn(&[Value], &mut akar_function::DataChunk) -> Result<(), String> + Send + Sync> =
                Arc::new(|args, chunk| {
                    if args.is_empty() {
                        return Err("iceberg_metadata requires a path argument".into());
                    }
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err("iceberg_metadata expects a string path argument".into()),
                    };

                    if chunk.size > 0 {
                        return Ok(());
                    }

                    let meta = native_reader::IcebergMetadata::load(&path)?;

                    let format_version = format!("v{}", meta.format_version);
                    let schema_str = match &meta.current_schema {
                        Some(s) => native_reader::format_schema(s),
                        None => "N/A".to_string(),
                    };

                    let format_arr = std::sync::Arc::new(arrow::array::StringArray::from(vec![format_version.as_str()]))
                        as arrow::array::ArrayRef;
                    let snap_count_arr =
                        std::sync::Arc::new(arrow::array::Int64Array::from(vec![meta.snapshot_count as i64]))
                            as arrow::array::ArrayRef;
                    let schema_arr = std::sync::Arc::new(arrow::array::StringArray::from(vec![schema_str.as_str()]))
                        as arrow::array::ArrayRef;

                    chunk.fields.clear();
                    chunk.field_types.clear();
                    chunk.field_names.clear();
                    chunk.fields.push(format_arr);
                    chunk.fields.push(snap_count_arr);
                    chunk.fields.push(schema_arr);
                    chunk.field_types.push(akar_common::types::PhysicalTypeID::String);
                    chunk.field_types.push(akar_common::types::PhysicalTypeID::Int64);
                    chunk.field_types.push(akar_common::types::PhysicalTypeID::String);
                    chunk.field_names.push("format_version".to_string());
                    chunk.field_names.push("snapshot_count".to_string());
                    chunk.field_names.push("schema".to_string());
                    chunk.size = 1;
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
            let snap_fn: Arc<dyn Fn(&[Value], &mut akar_function::DataChunk) -> Result<(), String> + Send + Sync> =
                Arc::new(|args, chunk| {
                    if args.is_empty() {
                        return Err("iceberg_snapshots requires a path argument".into());
                    }
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err("iceberg_snapshots expects a string path argument".into()),
                    };

                    if chunk.size > 0 {
                        return Ok(());
                    }

                    let meta = native_reader::IcebergMetadata::load(&path)?;
                    let n = meta.snapshots.len();

                    let ids: Vec<i64> = meta.snapshots.iter().map(|s| s.snapshot_id).collect();
                    let timestamps: Vec<i64> = meta.snapshots.iter().map(|s| s.timestamp_ms).collect();
                    let ops: Vec<&str> = meta.snapshots.iter().map(|s| s.operation.as_str()).collect();
                    let manifests: Vec<&str> = meta.snapshots.iter().map(|s| s.manifest_list.as_str()).collect();

                    chunk.fields.clear();
                    chunk.field_types.clear();
                    chunk.field_names.clear();

                    chunk
                        .fields
                        .push(std::sync::Arc::new(arrow::array::Int64Array::from(ids)) as arrow::array::ArrayRef);
                    chunk.fields.push(
                        std::sync::Arc::new(arrow::array::Int64Array::from(timestamps)) as arrow::array::ArrayRef
                    );
                    chunk
                        .fields
                        .push(std::sync::Arc::new(arrow::array::StringArray::from(ops)) as arrow::array::ArrayRef);
                    chunk.fields.push(
                        std::sync::Arc::new(arrow::array::StringArray::from(manifests)) as arrow::array::ArrayRef
                    );

                    chunk.field_types.push(akar_common::types::PhysicalTypeID::Int64);
                    chunk.field_types.push(akar_common::types::PhysicalTypeID::Int64);
                    chunk.field_types.push(akar_common::types::PhysicalTypeID::String);
                    chunk.field_types.push(akar_common::types::PhysicalTypeID::String);

                    chunk.field_names.push("snapshot_id".to_string());
                    chunk.field_names.push("timestamp_ms".to_string());
                    chunk.field_names.push("operation".to_string());
                    chunk.field_names.push("manifest_list".to_string());

                    chunk.size = n;
                    Ok(())
                });

            context.register_table_function(
                "iceberg_snapshots",
                TableFunction::CustomTable {
                    name: "iceberg_snapshots".into(),
                    execute: snap_fn,
                },
            );

            tracing::info!("Iceberg extension loaded: 3 functions registered (native reader)");
        }

        #[cfg(not(feature = "native"))]
        {
            #[cfg(feature = "duckdb-delegation")]
            {
                let scan_fn: Arc<dyn Fn(&[Value], &mut akar_function::DataChunk) -> Result<(), String> + Send + Sync> =
                    Arc::new(|args, _chunk| {
                        if args.is_empty() {
                            return Err("iceberg_scan requires a path argument".into());
                        }
                        let path = match &args[0] {
                            Value::String(s) => s.clone(),
                            _ => return Err("iceberg_scan expects a string path argument".into()),
                        };

                        let helper = akar_duckdb::attach_helper::DuckDbAttachHelper::new()?;
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

                let meta_fn: Arc<dyn Fn(&[Value], &mut akar_function::DataChunk) -> Result<(), String> + Send + Sync> =
                    Arc::new(|args, _chunk| {
                        if args.is_empty() {
                            return Err("iceberg_metadata requires a path argument".into());
                        }
                        let path = match &args[0] {
                            Value::String(s) => s.clone(),
                            _ => return Err("iceberg_metadata expects a string path argument".into()),
                        };

                        let helper = akar_duckdb::attach_helper::DuckDbAttachHelper::new()?;
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

                let snap_fn: Arc<dyn Fn(&[Value], &mut akar_function::DataChunk) -> Result<(), String> + Send + Sync> =
                    Arc::new(|args, _chunk| {
                        if args.is_empty() {
                            return Err("iceberg_snapshots requires a path argument".into());
                        }
                        let path = match &args[0] {
                            Value::String(s) => s.clone(),
                            _ => return Err("iceberg_snapshots expects a string path argument".into()),
                        };

                        let helper = akar_duckdb::attach_helper::DuckDbAttachHelper::new()?;
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
                        execute: Arc::new(|_, _| {
                            Err("Iceberg not available (enable feature 'native' or 'duckdb-delegation')".into())
                        }),
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
