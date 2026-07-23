//! Native Delta Lake table reader for Akar.
//!
//! Reads Delta Lake tables by parsing the transaction log (_delta_log/*.json).
//! Supports listing data files, extracting schema, and reading table metadata.

use std::fs;
use std::path::Path;

/// Parsed Delta table metadata.
#[allow(dead_code)]
pub struct DeltaTableInfo {
    pub version: i64,
    pub data_files: Vec<String>,
    pub schema: Option<String>,
    pub table_id: Option<String>,
    pub min_reader_version: i32,
    pub min_writer_version: i32,
}

/// Find the latest delta log version by listing _delta_log directory.
fn latest_delta_log_version(table_path: &str) -> Result<i64, String> {
    let log_dir = Path::new(table_path).join("_delta_log");
    if !log_dir.exists() {
        return Err(format!("Delta log directory not found: {}", log_dir.display()));
    }

    let entries = fs::read_dir(&log_dir).map_err(|e| format!("Failed to read delta log directory: {e}"))?;

    let max_version = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            // Match "00000000000000000000.json" pattern
            if name.ends_with(".json") && name.len() == 25 {
                name[..20].parse::<i64>().ok()
            } else {
                None
            }
        })
        .max();

    match max_version {
        Some(v) => Ok(v),
        None => Err(format!("No delta log files found in {}", log_dir.display())),
    }
}

/// Read the latest delta log JSON and parse actions.
fn read_latest_delta_log(table_path: &str) -> Result<Vec<serde_json::Value>, String> {
    let version = latest_delta_log_version(table_path)?;
    let log_path = Path::new(table_path)
        .join("_delta_log")
        .join(format!("{:020}.json", version));

    let content =
        fs::read_to_string(&log_path).map_err(|e| format!("Failed to read delta log {}: {e}", log_path.display()))?;

    let actions: Vec<serde_json::Value> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    Ok(actions)
}

/// Load Delta table info from the given path.
pub fn load_delta_table(table_path: &str) -> Result<DeltaTableInfo, String> {
    let version = latest_delta_log_version(table_path)?;
    let actions = read_latest_delta_log(table_path)?;

    let mut data_files = Vec::new();
    let mut schema: Option<String> = None;
    let mut table_id: Option<String> = None;
    let mut min_reader_version: i32 = 1;
    let mut min_writer_version: i32 = 2;

    // Track files to remove
    let mut removed_files: Vec<String> = Vec::new();

    for action in &actions {
        // Add file
        if let Some(add) = action.get("add") {
            if let Some(path_str) = add.get("path").and_then(|v| v.as_str()) {
                let full_path = Path::new(table_path).join(path_str);
                data_files.push(full_path.to_string_lossy().to_string());
            }
        }

        // Remove file
        if let Some(remove) = action.get("remove") {
            if let Some(path_str) = remove.get("path").and_then(|v| v.as_str()) {
                removed_files.push(path_str.to_string());
            }
        }

        // Metadata action
        if let Some(meta) = action.get("metaData") {
            if let Some(id) = meta.get("id").and_then(|v| v.as_str()) {
                table_id = Some(id.to_string());
            }
            if let Some(schema_str) = meta.get("schemaString").and_then(|v| v.as_str()) {
                schema = Some(format_schema_string(schema_str));
            }
        }

        // Protocol action
        if let Some(proto) = action.get("protocol") {
            min_reader_version = proto.get("minReaderVersion").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
            min_writer_version = proto.get("minWriterVersion").and_then(|v| v.as_i64()).unwrap_or(2) as i32;
        }
    }

    // Filter out removed files
    data_files.retain(|f| {
        let name = Path::new(f).file_name().and_then(|n| n.to_str()).unwrap_or("");
        !removed_files.iter().any(|r| r.ends_with(name))
    });

    Ok(DeltaTableInfo {
        version,
        data_files,
        schema,
        table_id,
        min_reader_version,
        min_writer_version,
    })
}

/// Format a Delta schema JSON string into human-readable form.
#[allow(dead_code)]
fn format_schema_string(schema_str: &str) -> String {
    let json: serde_json::Value = match serde_json::from_str(schema_str) {
        Ok(v) => v,
        Err(_) => return schema_str.to_string(),
    };

    let fields = match json.get("fields").and_then(|v| v.as_array()) {
        Some(f) => f,
        None => return schema_str.to_string(),
    };

    fields
        .iter()
        .filter_map(|f| {
            let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let type_str = f.get("type").and_then(|v| v.as_str()).unwrap_or("?");
            let nullable = f.get("nullable").and_then(|v| v.as_bool()).unwrap_or(true);
            Some(format!("  {name}: {type_str} (nullable={nullable})"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// List all Parquet data files in the table directory (non-_delta_log files).
#[allow(dead_code)]
pub fn list_parquet_files(table_path: &str) -> Result<Vec<String>, String> {
    let dir = Path::new(table_path);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    collect_table_parquet_files(dir, &mut files)?;
    Ok(files)
}

#[allow(dead_code)]
fn collect_table_parquet_files(dir: &Path, files: &mut Vec<String>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("Failed to read directory {}: {e}", dir.display()))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read entry: {e}"))?;
        let path = entry.path();
        if path.is_dir() {
            // Skip _delta_log directory
            if path.file_name().map_or(false, |n| n != "_delta_log") {
                collect_table_parquet_files(&path, files)?;
            }
        } else if path.extension().map_or(false, |ext| ext == "parquet") {
            files.push(path.to_string_lossy().to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_delta_log_dir(table_path: &str) -> String {
        let log_dir = Path::new(table_path).join("_delta_log");
        fs::create_dir_all(&log_dir).unwrap();

        // Create version 0 log
        let log_entries = vec![
            r#"{"protocol":{"minReaderVersion":1,"minWriterVersion":2}}"#,
            r#"{"metaData":{"id":"test-table-id","format":{"provider":"parquet"},"schemaString":"{\"type\":\"struct\",\"fields\":[{\"name\":\"id\",\"type\":\"integer\",\"nullable\":true,\"metadata\":{}},{\"name\":\"name\",\"type\":\"string\",\"nullable\":true,\"metadata\":{}}]}","partitionColumns":[]}}"#,
            r#"{"add":{"path":"part-00001-xxx.parquet","size":100,"partitionValues":{},"modificationTime":1710000000000,"dataChange":true}}"#,
            r#"{"add":{"path":"part-00002-xxx.parquet","size":200,"partitionValues":{},"modificationTime":1710000000001,"dataChange":true}}"#,
        ];
        let log_path = log_dir.join("00000000000000000000.json");
        fs::write(&log_path, log_entries.join("\n")).unwrap();

        log_path.to_string_lossy().to_string()
    }

    #[test]
    fn test_load_delta_table() {
        let dir = std::env::temp_dir().join("delta_test_load");
        let _ = fs::remove_dir_all(&dir);
        create_delta_log_dir(&dir.to_string_lossy());

        let info = load_delta_table(&dir.to_string_lossy()).unwrap();
        assert_eq!(info.version, 0);
        assert_eq!(info.data_files.len(), 2);
        assert!(info.schema.is_some());
        assert_eq!(info.table_id.unwrap(), "test-table-id");
        assert_eq!(info.min_reader_version, 1);
    }

    #[test]
    fn test_list_parquet_files() {
        let dir = std::env::temp_dir().join("delta_test_data");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("file1.parquet"), "fake data").unwrap();
        // Create _delta_log directory that should be skipped
        fs::create_dir_all(dir.join("_delta_log")).unwrap();
        fs::write(dir.join("_delta_log/00000000000000000000.json"), "{}").unwrap();

        let files = list_parquet_files(&dir.to_string_lossy()).unwrap();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn test_format_schema_string() {
        let schema = r#"{"type":"struct","fields":[{"name":"id","type":"integer","nullable":true,"metadata":{}},{"name":"name","type":"string","nullable":false,"metadata":{}}]}"#;
        let formatted = format_schema_string(schema);
        assert!(formatted.contains("id"));
        assert!(formatted.contains("string"));
    }
}
