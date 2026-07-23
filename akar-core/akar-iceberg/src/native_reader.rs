//! Native Apache Iceberg table reader for Akar.
//!
//! Reads Iceberg table metadata and data files without DuckDB delegation.
//! Supports reading metadata.json for schema/snapshot info and enumerating
//! data files (Parquet) from the `data/` subdirectory.

use std::fs;
use std::path::Path;

/// Iceberg table metadata parsed from metadata.json.
pub struct IcebergMetadata {
    pub format_version: i64,
    pub current_schema: Option<SchemaField>,
    pub snapshot_count: usize,
    pub snapshots: Vec<IcebergSnapshot>,
}

impl IcebergMetadata {
    /// Load and parse iceberg metadata from a table path.
    /// Looks for `metadata/metadata.json` under the given path.
    pub fn load(table_path: &str) -> Result<Self, String> {
        let meta_path = find_metadata_json(table_path)?;
        let content =
            fs::read_to_string(&meta_path).map_err(|e| format!("Failed to read metadata file {meta_path}: {e}"))?;

        let json: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| format!("Failed to parse Iceberg metadata JSON: {e}"))?;

        let format_version = json["format-version"].as_i64().unwrap_or(1);

        let snapshots = parse_snapshots(&json["snapshots"]);
        let snapshot_count = snapshots.len();

        let current_schema = parse_schema(&json);

        Ok(IcebergMetadata {
            format_version,
            current_schema,
            snapshot_count,
            snapshots,
        })
    }
}

/// A single field in an Iceberg schema.
pub struct SchemaField {
    pub fields: Vec<FieldInfo>,
}

pub struct FieldInfo {
    pub id: i32,
    pub name: String,
    pub type_str: String,
    pub required: bool,
}

/// A single snapshot in an Iceberg table.
pub struct IcebergSnapshot {
    pub snapshot_id: i64,
    pub timestamp_ms: i64,
    pub operation: String,
    pub manifest_list: String,
}

/// Find the latest metadata.json file in the table directory.
fn find_metadata_json(table_path: &str) -> Result<String, String> {
    let meta_dir = Path::new(table_path).join("metadata");

    if !meta_dir.exists() {
        return Err(format!("Iceberg metadata directory not found: {}", meta_dir.display()));
    }

    // Try version-hint.text first
    let hint_path = meta_dir.join("version-hint.text");
    if hint_path.exists() {
        let version = fs::read_to_string(&hint_path)
            .map_err(|e| format!("Failed to read version-hint: {e}"))?
            .trim()
            .to_string();
        let meta_file = meta_dir.join(format!("metadata.{version}.json"));
        if meta_file.exists() {
            return Ok(meta_file.to_string_lossy().to_string());
        }
    }

    // Fall back to metadata.json
    let default_meta = meta_dir.join("metadata.json");
    if default_meta.exists() {
        return Ok(default_meta.to_string_lossy().to_string());
    }

    // Try versioned metadata files
    let mut entries: Vec<_> = fs::read_dir(&meta_dir)
        .map_err(|e| format!("Failed to read metadata directory: {e}"))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name().to_string_lossy().starts_with("metadata.")
                && e.file_name().to_string_lossy().ends_with(".json")
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    if let Some(latest) = entries.last() {
        return Ok(latest.path().to_string_lossy().to_string());
    }

    Err(format!("No metadata.json found in {}", meta_dir.display()))
}

/// Parse the current schema from Iceberg metadata JSON.
fn parse_schema(json: &serde_json::Value) -> Option<SchemaField> {
    let current_id = json["current-schema-id"].as_i64().unwrap_or(0);
    let schemas = json["schemas"].as_array()?;

    let schema_val = schemas
        .iter()
        .find(|s| s.get("schema-id").and_then(|v| v.as_i64()) == Some(current_id))?;

    let fields_arr = schema_val.get("fields")?.as_array()?;
    let fields: Vec<FieldInfo> = fields_arr
        .iter()
        .map(|f| FieldInfo {
            id: f["id"].as_i64().unwrap_or(0) as i32,
            name: f["name"].as_str().unwrap_or("?").to_string(),
            type_str: f["type"].to_string(),
            required: f.get("required").and_then(|v| v.as_bool()).unwrap_or(false),
        })
        .collect();

    Some(SchemaField { fields })
}

/// Parse snapshots array from metadata JSON.
fn parse_snapshots(arr: &serde_json::Value) -> Vec<IcebergSnapshot> {
    match arr.as_array() {
        Some(snapshots) => snapshots
            .iter()
            .map(|s| IcebergSnapshot {
                snapshot_id: s["snapshot-id"].as_i64().unwrap_or(0),
                timestamp_ms: s["timestamp-ms"].as_i64().unwrap_or(0),
                operation: s["summary"]["operation"].as_str().unwrap_or("unknown").to_string(),
                manifest_list: s["manifest-list"].as_str().unwrap_or("").to_string(),
            })
            .collect(),
        None => Vec::new(),
    }
}

/// List all Parquet data files in the data/ subdirectory.
pub fn list_data_files(table_path: &str) -> Result<Vec<String>, String> {
    let data_dir = Path::new(table_path).join("data");
    if !data_dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    collect_parquet_files(&data_dir, &mut files)?;
    Ok(files)
}

fn collect_parquet_files(dir: &Path, files: &mut Vec<String>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("Failed to read directory {}: {e}", dir.display()))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read entry: {e}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_parquet_files(&path, files)?;
        } else if path.extension().map_or(false, |ext| ext == "parquet") {
            files.push(path.to_string_lossy().to_string());
        }
    }
    Ok(())
}

/// Format schema fields as a human-readable string.
pub fn format_schema(schema: &SchemaField) -> String {
    schema
        .fields
        .iter()
        .map(|f| format!("  {}: {} (id={}, required={})", f.name, f.type_str, f.id, f.required))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_test_metadata(dir: &str) -> String {
        let meta_dir = Path::new(dir).join("metadata");
        fs::create_dir_all(&meta_dir).unwrap();

        let metadata = serde_json::json!({
            "format-version": 2,
            "table-uuid": "test-uuid-1234",
            "location": dir,
            "current-schema-id": 0,
            "schemas": [
                {
                    "schema-id": 0,
                    "type": "struct",
                    "fields": [
                        {"id": 1, "name": "id", "type": "int", "required": true},
                        {"id": 2, "name": "name", "type": "string", "required": false}
                    ]
                }
            ],
            "current-snapshot-id": 1001,
            "snapshots": [
                {
                    "snapshot-id": 1001,
                    "timestamp-ms": 1710000000000i64,
                    "summary": {"operation": "append"},
                    "manifest-list": "snap-1001-manifest-list.avro"
                }
            ],
            "partition-specs": [],
            "properties": {}
        });

        let meta_path = meta_dir.join("metadata.json");
        fs::write(&meta_path, serde_json::to_string_pretty(&metadata).unwrap()).unwrap();
        meta_path.to_string_lossy().to_string()
    }

    #[test]
    fn test_load_metadata() {
        let dir = std::env::temp_dir().join("iceberg_test_load");
        let _ = fs::remove_dir_all(&dir);
        create_test_metadata(&dir.to_string_lossy());

        let meta = IcebergMetadata::load(&dir.to_string_lossy()).unwrap();
        assert_eq!(meta.format_version, 2);
        assert_eq!(meta.snapshot_count, 1);
        let schema = meta.current_schema.unwrap();
        assert_eq!(schema.fields.len(), 2);
        assert_eq!(schema.fields[0].name, "id");
        assert_eq!(schema.fields[1].name, "name");
    }

    #[test]
    fn test_list_data_files() {
        let dir = std::env::temp_dir().join("iceberg_test_data");
        let _ = fs::remove_dir_all(&dir);
        let data_dir = dir.join("data");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("file1.parquet"), "fake data").unwrap();
        fs::write(data_dir.join("file2.parquet"), "fake data").unwrap();

        let files = list_data_files(&dir.to_string_lossy()).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_format_schema() {
        let schema = SchemaField {
            fields: vec![
                FieldInfo {
                    id: 1,
                    name: "id".into(),
                    type_str: "int".into(),
                    required: true,
                },
                FieldInfo {
                    id: 2,
                    name: "name".into(),
                    type_str: "string".into(),
                    required: false,
                },
            ],
        };
        let formatted = format_schema(&schema);
        assert!(formatted.contains("id"));
        assert!(formatted.contains("name"));
    }
}
