//! Shared helpers for connector extensions (httpfs, azure, delta, iceberg,
//! unity-catalog, sqlite, duckdb).
//!
//! Centralizes the scan-closure copy-paste that used to live in each connector:
//! filling a single-string-column chunk, quoting SQL identifiers, and managing
//! the lifecycle of temp files downloaded by `*_scan` table functions.

use crate::data_chunk::DataChunk;
use crate::types::PhysicalTypeID;
use arrow::array::{ArrayRef, StringArray};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Fill a `DataChunk` with a single String column, replacing any existing schema.
///
/// The chunk `size` is set to `values.len()`. This mirrors the repeated
/// clear -> push StringArray -> set names/types -> size sequence that every
/// connector scan closure used to hand-roll.
pub fn fill_chunk_with_strings(chunk: &mut DataChunk, field_name: &str, values: &[String]) {
    let array: ArrayRef = Arc::new(StringArray::from_iter_values(values.iter().map(String::as_str)));
    chunk.fields.clear();
    chunk.field_types.clear();
    chunk.field_names.clear();
    chunk.fields.push(array);
    chunk.field_types.push(PhysicalTypeID::String);
    chunk.field_names.push(field_name.to_string());
    chunk.size = values.len();
}

/// Quote a single SQL identifier using double quotes (escaping embedded quotes).
pub fn quote_sql_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Quote a possibly-qualified SQL table name (`catalog.schema.table`) part-by-part.
///
/// Each dot-separated component is quoted independently so a user-supplied table
/// name cannot break out of the query (prevents SQL injection in DuckDB-delegated
/// scans).
pub fn quote_sql_table_name(name: &str) -> String {
    name.split('.').map(quote_sql_identifier).collect::<Vec<_>>().join(".")
}

/// Maximum number of retained connector temp files before the oldest are evicted.
const MAX_RETAINED_TEMP_FILES: usize = 64;

/// Registry of temp files downloaded by connector scans and kept past the scan
/// closure so downstream reads can open them by path.
static RETAINED_TEMP_FILES: Mutex<VecDeque<PathBuf>> = Mutex::new(VecDeque::new());

/// Track a kept temp file so it is eventually removed.
///
/// Connector scans previously called `NamedTempFile::keep()` and leaked the file
/// permanently (unbounded accumulation). The registry bounds the number of
/// retained files: when the cap is exceeded the oldest file is deleted from disk.
/// Returns the path as a string for the scan result row.
pub fn retain_temp_file(path: PathBuf) -> String {
    let mut queue = RETAINED_TEMP_FILES.lock().unwrap_or_else(|p| p.into_inner());
    queue.push_back(path.clone());
    while queue.len() > MAX_RETAINED_TEMP_FILES {
        if let Some(oldest) = queue.pop_front() {
            let _ = std::fs::remove_file(&oldest);
        }
    }
    path.to_string_lossy().to_string()
}

/// Remove all currently retained temp files (used by tests and clean shutdown).
pub fn clear_retained_temp_files() {
    let mut queue = RETAINED_TEMP_FILES.lock().unwrap_or_else(|p| p.into_inner());
    for path in queue.drain(..) {
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::DataChunk;

    #[test]
    fn test_fill_chunk_with_strings() {
        let mut chunk = DataChunk::new(Vec::new(), Vec::new());
        fill_chunk_with_strings(&mut chunk, "path", &["a.parquet".into(), "b.parquet".into()]);
        assert_eq!(chunk.size, 2);
        assert_eq!(chunk.num_fields(), 1);
        assert_eq!(chunk.field_names, vec!["path".to_string()]);
        assert_eq!(chunk.field_types, vec![PhysicalTypeID::String]);
    }

    #[test]
    fn test_fill_chunk_with_strings_replaces_schema() {
        let mut chunk = DataChunk::new(Vec::new(), Vec::new());
        fill_chunk_with_strings(&mut chunk, "first", &["a".into()]);
        fill_chunk_with_strings(&mut chunk, "second", &["x".into(), "y".into(), "z".into()]);
        assert_eq!(chunk.size, 3);
        assert_eq!(chunk.num_fields(), 1);
        assert_eq!(chunk.field_names, vec!["second".to_string()]);
    }

    #[test]
    fn test_quote_sql_identifier() {
        assert_eq!(quote_sql_identifier("tbl"), "\"tbl\"");
        assert_eq!(quote_sql_identifier("we\"ird"), "\"we\"\"ird\"");
    }

    #[test]
    fn test_quote_sql_table_name() {
        assert_eq!(
            quote_sql_table_name("main.default.people"),
            "\"main\".\"default\".\"people\""
        );
        assert_eq!(
            quote_sql_table_name("x.\"y\"; DROP TABLE t"),
            "\"x\".\"\"\"y\"\"; DROP TABLE t\""
        );
    }

    #[test]
    fn test_retain_temp_file_evicts_oldest() {
        clear_retained_temp_files();
        let dir = std::env::temp_dir();
        for i in 0..(MAX_RETAINED_TEMP_FILES + 5) {
            let p = dir.join(format!("akar_retain_test_{i}.tmp"));
            std::fs::write(&p, b"x").unwrap();
            retain_temp_file(p);
        }
        // Oldest 5 files must have been evicted (deleted).
        for i in 0..5 {
            let p = dir.join(format!("akar_retain_test_{i}.tmp"));
            assert!(!p.exists(), "file {i} should have been evicted");
        }
        // Newest MAX files still present.
        for i in 5..(MAX_RETAINED_TEMP_FILES + 5) {
            let p = dir.join(format!("akar_retain_test_{i}.tmp"));
            assert!(p.exists(), "file {i} should still be retained");
        }
        clear_retained_temp_files();
    }
}
