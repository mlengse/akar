//! Unity Catalog extension for Akar.
//!
//! Provides integration with Databricks Unity Catalog.
//! - **Native mode** (feature `native`): Calls UC REST API directly using `ureq`.
//! - **DuckDB delegation** (feature `duckdb-delegation`): Delegates to DuckDB's uc_catalog extension.

#[cfg(feature = "native")]
mod native_client;

use akar_extension::{Extension, ExtensionContext};
use std::sync::Arc;

#[cfg(any(feature = "native", feature = "duckdb-delegation"))]
use akar_function::Value;

/// The Unity Catalog extension enables querying Unity Catalog from Akar.
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
        use akar_function::registry::TableFunction;

        #[cfg(feature = "native")]
        {
            let scan_fn: Arc<dyn Fn(&[Value], &mut akar_function::DataChunk) -> Result<(), String> + Send + Sync> =
                Arc::new(|args, chunk| {
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

                    if chunk.size > 0 {
                        return Ok(());
                    }

                    let info = native_client::get_table_info(&endpoint, &token, &table)?;

                    let name_arr = std::sync::Arc::new(arrow::array::StringArray::from(vec![info.table_name.as_str()]))
                        as arrow::array::ArrayRef;
                    let type_arr = std::sync::Arc::new(arrow::array::StringArray::from(vec![info.table_type.as_str()]))
                        as arrow::array::ArrayRef;
                    let schema_arr = std::sync::Arc::new(arrow::array::StringArray::from(vec![info.schema.as_str()]))
                        as arrow::array::ArrayRef;
                    let location_arr = std::sync::Arc::new(arrow::array::StringArray::from(vec![
                        info.storage_location.as_deref().unwrap_or("N/A"),
                    ])) as arrow::array::ArrayRef;

                    chunk.fields.clear();
                    chunk.field_types.clear();
                    chunk.field_names.clear();
                    chunk.fields.push(name_arr);
                    chunk.fields.push(type_arr);
                    chunk.fields.push(schema_arr);
                    chunk.fields.push(location_arr);
                    chunk.field_types.push(akar_common::types::PhysicalTypeID::String);
                    chunk.field_types.push(akar_common::types::PhysicalTypeID::String);
                    chunk.field_types.push(akar_common::types::PhysicalTypeID::String);
                    chunk.field_types.push(akar_common::types::PhysicalTypeID::String);
                    chunk.field_names.push("table_name".to_string());
                    chunk.field_names.push("table_type".to_string());
                    chunk.field_names.push("schema".to_string());
                    chunk.field_names.push("storage_location".to_string());
                    chunk.size = 1;
                    Ok(())
                });

            context.register_table_function(
                "uc_scan",
                TableFunction::CustomTable {
                    name: "uc_scan".into(),
                    execute: scan_fn,
                },
            );

            tracing::info!("Unity Catalog extension loaded: 1 function registered (native REST client)");
        }

        #[cfg(not(feature = "native"))]
        {
            #[cfg(feature = "duckdb-delegation")]
            {
                let scan_fn: Arc<dyn Fn(&[Value], &mut akar_function::DataChunk) -> Result<(), String> + Send + Sync> =
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

                        let helper = akar_duckdb::attach_helper::DuckDbAttachHelper::new()?;
                        helper.install_and_load("uc_catalog")?;

                        let create_secret = format!(
                            "CREATE SECRET (TYPE UC, TOKEN '{}', ENDPOINT '{}')",
                            token.replace('\'', "''"),
                            endpoint.replace('\'', "''")
                        );
                        helper.execute_batch(&create_secret)?;

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
                            Err("Unity Catalog not available (enable feature 'native' or 'duckdb-delegation')".into())
                        }),
                    },
                );
                tracing::info!("Unity Catalog extension loaded (placeholder)");
            }
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
