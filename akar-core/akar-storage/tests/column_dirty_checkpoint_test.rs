//! Column-granular checkpoint tests (T034, US5 — FR-008).
//!
//! A single-column `UPDATE` on a wide table must rewrite only that column's
//! durable mirror file, keep every other column byte-identical, and still
//! round-trip through `load_node_table`.

use akar_common::enums::CompressionType;
use akar_common::memory::MemoryManager;
use akar_common::types::{LogicalTypeID, Value};
use akar_storage::buffer_manager::{BufferManager, BufferManagerConfig};
use akar_storage::persistence::TablePersistence;
use akar_storage::table::{ColumnDefinition, NodeTable};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

const TABLE_ID: u64 = 7;
const NUM_COLUMNS: usize = 16;
const NUM_ROWS: usize = 200;
const UPDATED_COLUMN: usize = 5;
const UPDATED_ROW: usize = 37;

fn columns() -> Vec<ColumnDefinition> {
    (0..NUM_COLUMNS)
        .map(|ci| ColumnDefinition {
            name: if ci == 0 {
                "id".into()
            } else {
                format!("col_{ci}")
            },
            logical_type: LogicalTypeID::Int64,
            is_primary_key: ci == 0,
            compression: CompressionType::Uncompressed,
        })
        .collect()
}

fn buffer_manager(db_path: &Path) -> Arc<Mutex<BufferManager>> {
    std::fs::create_dir_all(db_path).expect("create db dir");
    Arc::new(Mutex::new(BufferManager::new(
        db_path.to_path_buf(),
        Arc::new(MemoryManager::new(64 * 1024 * 1024)),
        BufferManagerConfig::default(),
    )))
}

fn row(id: i64) -> Vec<Value> {
    (0..NUM_COLUMNS)
        .map(|ci| Value::Int64(id * 100 + ci as i64))
        .collect()
}

/// FNV-1a over file bytes — enough to detect an unchanged mirror file.
fn file_hash(path: &Path) -> Option<u64> {
    let bytes = std::fs::read(path).ok()?;
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Some(hash)
}

/// Every mirror file belonging to `TABLE_ID` (column data + metadata).
fn mirror_files(db_path: &Path) -> Vec<PathBuf> {
    let prefix = format!("col_{TABLE_ID}_");
    let mut files: Vec<PathBuf> = std::fs::read_dir(db_path)
        .expect("read db dir")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix))
        })
        .collect();
    files.sort();
    files
}

fn snapshot(db_path: &Path) -> HashMap<PathBuf, (u64, u64)> {
    mirror_files(db_path)
        .into_iter()
        .filter_map(|path| {
            let hash = file_hash(&path)?;
            let size = std::fs::metadata(&path).ok()?.len();
            Some((path, (hash, size)))
        })
        .collect()
}

#[test]
fn test_single_column_dirty_flush() {
    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("db");
    let bm = buffer_manager(&db_path);
    let page_size = bm.lock().expect("bm lock").page_size();
    let persistence = TablePersistence::new();

    let mut table = NodeTable::new(TABLE_ID, "Wide".into(), columns());
    for id in 0..NUM_ROWS as i64 {
        table.insert_row(row(id)).expect("insert row");
    }

    // Baseline: every column mirror is written.
    persistence
        .sync_node_table(&mut table, &db_path, &bm, page_size)
        .expect("initial sync");
    assert!(table.dirty_columns.is_empty(), "initial sync clears dirty columns");
    let before = snapshot(&db_path);
    assert!(
        before.len() >= NUM_COLUMNS,
        "expected at least one mirror file per column, got {}",
        before.len()
    );

    // A single-cell UPDATE must dirty exactly one column.
    table
        .update_cell(UPDATED_ROW as u64, UPDATED_COLUMN, Value::Int64(999_999))
        .expect("update cell");
    assert_eq!(
        table.dirty_columns.iter().copied().collect::<Vec<_>>(),
        vec![UPDATED_COLUMN],
        "only the updated column may be marked dirty"
    );

    persistence
        .sync_node_table(&mut table, &db_path, &bm, page_size)
        .expect("dirty sync");
    assert!(table.dirty_columns.is_empty(), "dirty sync clears the dirty set");
    assert!(!table.persistence_dirty, "dirty sync clears the dirty flag");

    // Only the updated column's mirror bytes may have changed.
    let after = snapshot(&db_path);
    let mut changed_bytes = 0u64;
    let mut total_bytes = 0u64;
    let mut changed_columns: Vec<String> = Vec::new();
    for (path, (hash, size)) in &after {
        total_bytes += size;
        let was = before.get(path).map(|(hash, _)| *hash);
        if was != Some(*hash) {
            changed_bytes += size;
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            // `col_7_5` and `col_7_5.meta` both belong to column 5.
            let column = name
                .strip_prefix(&format!("col_{TABLE_ID}_"))
                .and_then(|rest| rest.split('.').next())
                .and_then(|ci| ci.parse::<usize>().ok());
            assert_eq!(
                column,
                Some(UPDATED_COLUMN),
                "unexpected mirror file rewritten: {name}"
            );
            changed_columns.push(name.to_string());
        }
    }
    assert!(
        !changed_columns.is_empty(),
        "the updated column's mirror must be rewritten"
    );
    assert!(
        changed_bytes * 100 < total_bytes * 10,
        "dirty flush wrote {changed_bytes} of {total_bytes} bytes ({}%), expected < 10%",
        changed_bytes * 100 / total_bytes.max(1)
    );

    // The mirror must still restore the exact post-update table.
    let mut restored = NodeTable::new(TABLE_ID, "Wide".into(), columns());
    let loaded = persistence
        .load_node_table(&mut restored, &db_path, &bm, page_size)
        .expect("load mirror");
    assert!(loaded, "mirror must exist");
    assert_eq!(restored.num_rows, NUM_ROWS as u64);
    assert_eq!(
        restored.get_value(UPDATED_ROW, UPDATED_COLUMN),
        Some(&Value::Int64(999_999)),
        "updated cell must survive the column-granular flush"
    );
    for ci in 0..NUM_COLUMNS {
        if ci == UPDATED_COLUMN {
            continue;
        }
        assert_eq!(
            restored.get_value(UPDATED_ROW, ci),
            Some(&Value::Int64(UPDATED_ROW as i64 * 100 + ci as i64)),
            "column {ci} must be unchanged"
        );
    }
    assert_eq!(restored.get_value(0, 1), Some(&Value::Int64(1)));
    assert_eq!(
        restored.get_value(NUM_ROWS - 1, 2),
        Some(&Value::Int64((NUM_ROWS as i64 - 1) * 100 + 2))
    );
}
