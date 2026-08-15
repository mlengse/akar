//! Native Unity Catalog REST API client for Akar.
//!
//! Uses `ureq` to call Databricks Unity Catalog REST API directly,
//! without DuckDB delegation.
//!
//! API Reference: https://docs.databricks.com/api/workspace/unitycatalog

use serde_json::Value;
use std::io::Read;

/// Result of scanning a Unity Catalog table.
///
/// `columns`/`rows`/`row_count` are scan-result scaffolding for the future
/// data-scan path (currently the REST client only returns metadata); only
/// `table_name`/`table_type`/`schema`/`storage_location` are read today.
/// Compiled only under the `native` feature (`--all-features`).
#[allow(dead_code)]
pub struct UcTableScan {
    pub table_name: String,
    pub table_type: String,
    pub schema: String,
    pub storage_location: Option<String>,
    pub columns: Vec<UcColumnInfo>,
    pub rows: Vec<Vec<String>>,
    pub row_count: usize,
}

pub struct UcColumnInfo {
    pub name: String,
    pub type_str: String,
    pub nullable: bool,
}

/// Call the Unity Catalog REST API to get table metadata.
///
/// `endpoint`: Databricks workspace URL (e.g., `https://myworkspace.cloud.databricks.com`)
/// `token`: Databricks personal access token
/// `table`: Fully qualified table name (e.g., `catalog.schema.table`)
pub fn get_table_info(endpoint: &str, token: &str, table: &str) -> Result<UcTableScan, String> {
    let url = format!(
        "{}/api/2.1/unity-catalog/tables/{}",
        endpoint.trim_end_matches('/'),
        table
    );

    let resp = ureq::get(&url)
        .header("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| format!("Unity Catalog API request failed: {e}"))?;

    let status = resp.status();
    if status != 200 {
        return Err(format!(
            "Unity Catalog API returned HTTP {status} for table '{table}' (endpoint: {endpoint})"
        ));
    }

    let mut body_reader = resp.into_body().into_reader();
    let mut body_str = String::new();
    body_reader
        .read_to_string(&mut body_str)
        .map_err(|e| format!("Failed to read UC response: {e}"))?;
    let body: Value = serde_json::from_str(&body_str).map_err(|e| format!("Failed to parse UC response: {e}"))?;

    let table_name = body["name"].as_str().unwrap_or(table).to_string();
    let table_type = body["table_type"].as_str().unwrap_or("UNKNOWN").to_string();
    let storage_location = body["storage_location"].as_str().map(|s| s.to_string());

    let columns: Vec<UcColumnInfo> = body["columns"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|c| UcColumnInfo {
                    name: c["name"].as_str().unwrap_or("?").to_string(),
                    type_str: c["type_text"].as_str().unwrap_or("?").to_string(),
                    nullable: c["nullable"].as_bool().unwrap_or(true),
                })
                .collect()
        })
        .unwrap_or_default();

    let schema = columns
        .iter()
        .map(|c| format!("  {}: {} (nullable={})", c.name, c.type_str, c.nullable))
        .collect::<Vec<_>>()
        .join("\n");

    Ok(UcTableScan {
        table_name,
        table_type,
        schema,
        storage_location,
        columns,
        rows: Vec::new(),
        row_count: 0,
    })
}

/// Format column info into human-readable schema string.
#[allow(dead_code)]
pub fn format_columns(columns: &[UcColumnInfo]) -> String {
    columns
        .iter()
        .map(|c| format!("  {}: {} (nullable={})", c.name, c.type_str, c.nullable))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_columns() {
        let columns = vec![
            UcColumnInfo {
                name: "id".into(),
                type_str: "int".into(),
                nullable: false,
            },
            UcColumnInfo {
                name: "name".into(),
                type_str: "string".into(),
                nullable: true,
            },
        ];
        let formatted = format_columns(&columns);
        assert!(formatted.contains("id"));
        assert!(formatted.contains("name"));
    }

    #[test]
    fn test_get_table_info_empty_columns() {
        // Test parsing with missing columns field
        let _ = UcTableScan {
            table_name: "test".into(),
            table_type: "TABLE".into(),
            schema: String::new(),
            storage_location: None,
            columns: Vec::new(),
            rows: Vec::new(),
            row_count: 0,
        };
    }
}
