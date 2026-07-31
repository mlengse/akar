//! Durable column mirror for in-memory tables (P45.4).
//!
//! Node/rel table rows are held in memory (`NodeGroup`s for node tables,
//! edge/adjacency lists for rel tables). This module mirrors committed rows
//! into persistent `Column` files (`col_{table_id}_{col_idx}` with a `.meta`
//! sidecar) so that committed data survives process restarts.
//!
//! Lifecycle:
//! - `sync_*` is called at commit/checkpoint. Newly inserted rows are appended
//!   incrementally; when an UPDATE/DELETE touched a table, the mirror is
//!   rewritten from scratch.
//! - `load_*` is called at startup (`Database::new`) after tables are restored
//!   from the persisted catalog, rebuilding the in-memory state from the mirror.
//! - `remove` is called on DROP TABLE to delete the mirror files.

use crate::buffer_manager::BufferManager;
use crate::column::Column;
use crate::table::{ColumnDefinition, NodeTable, RelTable, TableCatalog};
use akar_common::enums::CompressionType;
use akar_common::error::StorageError;
use akar_common::types::{LogicalTypeID, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Per-table durable mirror state.
#[derive(Debug, Default)]
pub struct TablePersistenceState {
    /// Durable mirror columns, one per persisted table column.
    pub columns: Vec<Column>,
    /// Number of table rows already flushed into `columns`.
    pub flushed_rows: u64,
    /// Values larger than a single mirror page: `oversized[col][row]` holds the
    /// serialised value bytes (the column keeps a `Null` placeholder so row
    /// indices stay aligned). Persisted to the `col_{tid}.ovf` sidecar.
    pub oversized: Vec<HashMap<u64, Vec<u8>>>,
}

/// Registry of durable column mirrors keyed by table id.
#[derive(Debug, Default)]
pub struct TablePersistence {
    inner: Mutex<HashMap<u64, TablePersistenceState>>,
}

impl TablePersistence {
    pub fn new() -> Self {
        Self::default()
    }

    fn io_err(e: std::io::Error) -> StorageError {
        StorageError::Page(format!("persistence I/O error: {e}"))
    }

    fn ovf_file_name(table_id: u64) -> String {
        format!("col_{table_id}.ovf")
    }

    /// Append `value` to mirror column `col`. Values that are larger than a
    /// single mirror page cannot be stored inline: they are recorded in
    /// `oversized` (with a `Null` placeholder in the column so row indices stay
    /// aligned) and persisted to the `.ovf` sidecar.
    fn append_mirror_value(
        col: &mut Column,
        oversized: &mut HashMap<u64, Vec<u8>>,
        row: u64,
        value: &Value,
    ) -> Result<(), StorageError> {
        match col.append_value(value) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::OutOfMemory => {
                oversized.insert(row, Column::serialize_value(value));
                col.append_value(&Value::Null).map_err(Self::io_err)
            }
            Err(e) => Err(Self::io_err(e)),
        }
    }

    /// Persist the oversized-value map to the `col_{tid}.ovf` sidecar.
    fn save_overflow(
        table_id: u64,
        oversized: &[HashMap<u64, Vec<u8>>],
        db_path: &Path,
    ) -> Result<(), StorageError> {
        let path = db_path.join(Self::ovf_file_name(table_id));
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(&(oversized.len() as u32).to_le_bytes());
        for col in oversized {
            let mut entries: Vec<(&u64, &Vec<u8>)> = col.iter().collect();
            entries.sort_by_key(|(row, _)| **row);
            buf.extend_from_slice(&(entries.len() as u32).to_le_bytes());
            for (row, bytes) in entries {
                buf.extend_from_slice(&row.to_le_bytes());
                buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(bytes);
            }
        }
        std::fs::write(&path, &buf).map_err(Self::io_err)
    }

    /// Load the oversized-value map from the `col_{tid}.ovf` sidecar.
    fn load_overflow(
        table_id: u64,
        num_cols: usize,
        db_path: &Path,
    ) -> Vec<HashMap<u64, Vec<u8>>> {
        let mut result: Vec<HashMap<u64, Vec<u8>>> = (0..num_cols).map(|_| HashMap::new()).collect();
        let path = db_path.join(Self::ovf_file_name(table_id));
        let buf = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => return result,
        };
        let mut pos = 0usize;
        if buf.len() < 4 {
            return result;
        }
        let file_cols = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        for ci in 0..file_cols.min(num_cols) {
            if pos + 4 > buf.len() {
                break;
            }
            let count = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;
            for _ in 0..count {
                if pos + 12 > buf.len() {
                    break;
                }
                let row = u64::from_le_bytes(buf[pos..pos + 8].try_into().unwrap());
                pos += 8;
                let len = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
                pos += 4;
                if pos + len > buf.len() {
                    break;
                }
                result[ci].insert(row, buf[pos..pos + len].to_vec());
                pos += len;
            }
        }
        result
    }

    fn build_column(
        def: &ColumnDefinition,
        table_id: u64,
        col_idx: u32,
        db_path: &Path,
        bm: &Arc<Mutex<BufferManager>>,
        page_size: usize,
    ) -> Column {
        Column::with_compression(def.logical_type, table_id, col_idx, db_path, bm.clone(), page_size, def.compression)
    }

    /// Delete the mirror files for a dropped table.
    pub fn remove(&self, table_id: u64, db_path: &Path, bm: &Arc<Mutex<BufferManager>>) {
        if let Some(state) = self.inner.lock().unwrap().remove(&table_id) {
            Self::drop_mirror_files(table_id, state.columns.len(), db_path, bm);
        }
    }

    /// Remove the mirror files for columns `0..num_cols` of `table_id`.
    ///
    /// Drops the buffer-manager state (frames/mmap/registration) first so the
    /// files can be deleted even while cached or memory-mapped, then removes
    /// the data and `.meta` files from disk.
    fn drop_mirror_files(table_id: u64, num_cols: usize, db_path: &Path, bm: &Arc<Mutex<BufferManager>>) {
        {
            let mut guard = bm.lock().unwrap();
            for ci in 0..num_cols {
                let fname = format!("col_{}_{}", table_id, ci);
                guard.drop_file(&fname);
            }
        }
        for ci in 0..num_cols {
            let path = db_path.join(format!("col_{}_{}", table_id, ci));
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(path.with_extension("meta"));
        }
        let _ = std::fs::remove_file(db_path.join(Self::ovf_file_name(table_id)));
    }

    // ---------------------------------------------------------------------
    // Node tables
    // ---------------------------------------------------------------------

    /// Persist `table`'s rows into its durable column mirror.
    pub fn sync_node_table(
        &self,
        table: &mut NodeTable,
        db_path: &Path,
        bm: &Arc<Mutex<BufferManager>>,
        page_size: usize,
    ) -> Result<(), StorageError> {
        let num_cols = table.columns.len();
        let mut state = self.inner.lock().unwrap();
        let entry = state.entry(table.table_id).or_default();

        if !table.persistence_dirty && table.num_rows == entry.flushed_rows {
            return Ok(());
        }

        if table.persistence_dirty || table.num_rows < entry.flushed_rows {
            // Full rewrite: an UPDATE/DELETE touched the table, so rebuild the
            // mirror from scratch (the in-memory state is the source of truth).
            Self::drop_mirror_files(table.table_id, num_cols, db_path, bm);
            let mut columns = Vec::with_capacity(num_cols);
            let mut oversized: Vec<HashMap<u64, Vec<u8>>> = (0..num_cols).map(|_| HashMap::new()).collect();
            for (ci, def) in table.columns.iter().enumerate() {
                let mut col = Self::build_column(def, table.table_id, ci as u32, db_path, bm, page_size);
                for row in 0..table.num_rows as usize {
                    let value = table.get_value(row, ci).cloned().unwrap_or(Value::Null);
                    Self::append_mirror_value(&mut col, &mut oversized[ci], row as u64, &value)?;
                }
                col.flush().map_err(Self::io_err)?;
                col.save_metadata().map_err(Self::io_err)?;
                columns.push(col);
            }
            entry.columns = columns;
            entry.oversized = oversized;
            entry.flushed_rows = table.num_rows;
            table.persistence_dirty = false;
        } else {
            // Incremental append of newly inserted rows.
            if entry.columns.is_empty() {
                for (ci, def) in table.columns.iter().enumerate() {
                    entry
                        .columns
                        .push(Self::build_column(def, table.table_id, ci as u32, db_path, bm, page_size));
                }
                entry.oversized = (0..num_cols).map(|_| HashMap::new()).collect();
            }
            for row in entry.flushed_rows as usize..table.num_rows as usize {
                for (ci, col) in entry.columns.iter_mut().enumerate() {
                    let value = table.get_value(row, ci).cloned().unwrap_or(Value::Null);
                    Self::append_mirror_value(col, &mut entry.oversized[ci], row as u64, &value)?;
                }
            }
            for col in entry.columns.iter_mut() {
                col.flush().map_err(Self::io_err)?;
                col.save_metadata().map_err(Self::io_err)?;
            }
            entry.flushed_rows = table.num_rows;
        }
        Self::save_overflow(table.table_id, &entry.oversized, db_path)?;
        Ok(())
    }

    /// Load a node table from its durable column mirror.
    ///
    /// Returns `Ok(true)` when data was loaded, `Ok(false)` when no mirror
    /// exists yet (fresh table).
    pub fn load_node_table(
        &self,
        table: &mut NodeTable,
        db_path: &Path,
        bm: &Arc<Mutex<BufferManager>>,
        page_size: usize,
    ) -> Result<bool, StorageError> {
        let num_cols = table.columns.len();
        let mut columns = Vec::with_capacity(num_cols);
        for (ci, def) in table.columns.iter().enumerate() {
            let mut col = Self::build_column(def, table.table_id, ci as u32, db_path, bm, page_size);
            if !col.load_metadata().map_err(Self::io_err)? {
                return Ok(false);
            }
            columns.push(col);
        }

        let num_rows = columns[0].num_values as usize;
        let oversized = Self::load_overflow(table.table_id, num_cols, db_path);
        let mut rows = Vec::with_capacity(num_rows);
        for row in 0..num_rows {
            let mut values = Vec::with_capacity(num_cols);
            for ci in 0..num_cols {
                let value = if let Some(bytes) = oversized[ci].get(&(row as u64)) {
                    Column::deserialize_value_bytes(bytes).unwrap_or(Value::Null)
                } else {
                    columns[ci].get_value(row as u64).unwrap_or(Value::Null)
                };
                values.push(value);
            }
            rows.push(values);
        }

        table.load_persisted_rows(rows)?;

        let mut state = self.inner.lock().unwrap();
        state.insert(
            table.table_id,
            TablePersistenceState {
                columns,
                flushed_rows: table.num_rows,
                oversized,
            },
        );
        Ok(num_rows > 0)
    }

    // ---------------------------------------------------------------------
    // Rel tables
    // ---------------------------------------------------------------------

    /// Mirror column definition for the structural (src/dst) columns.
    fn rel_structural_def(col_idx: usize) -> ColumnDefinition {
        ColumnDefinition {
            name: format!("__structural_{col_idx}"),
            logical_type: LogicalTypeID::UInt64,
            is_primary_key: false,
            compression: CompressionType::Uncompressed,
        }
    }

    /// Persist `table`'s edges + properties into its durable column mirror.
    ///
    /// Mirror layout: `[src: UInt64][dst: UInt64][prop_0..prop_n]`. Deleted
    /// edges are stored as `(u64::MAX, u64::MAX)` tombstones so edge indices
    /// stay stable across restarts.
    pub fn sync_rel_table(
        &self,
        table: &mut RelTable,
        db_path: &Path,
        bm: &Arc<Mutex<BufferManager>>,
        page_size: usize,
    ) -> Result<(), StorageError> {
        let num_prop_cols = table.columns.len();
        let num_cols = num_prop_cols + 2;
        let mut state = self.inner.lock().unwrap();
        let entry = state.entry(table.table_id).or_default();

        if !table.persistence_dirty && table.num_rows == entry.flushed_rows {
            return Ok(());
        }

        let value_at = |table: &RelTable, ci: usize, e: usize| -> Value {
            if ci == 0 {
                Value::UInt64(table.edges[e].0)
            } else if ci == 1 {
                Value::UInt64(table.edges[e].1)
            } else {
                table.properties[ci - 2].get(e).cloned().unwrap_or(Value::Null)
            }
        };

        if table.persistence_dirty || table.num_rows < entry.flushed_rows {
            Self::drop_mirror_files(table.table_id, num_cols, db_path, bm);
            let mut columns = Vec::with_capacity(num_cols);
            let mut oversized: Vec<HashMap<u64, Vec<u8>>> = (0..num_cols).map(|_| HashMap::new()).collect();
            for ci in 0..num_cols {
                let def = if ci < 2 {
                    Self::rel_structural_def(ci)
                } else {
                    table.columns[ci - 2].clone()
                };
                let mut col = Self::build_column(&def, table.table_id, ci as u32, db_path, bm, page_size);
                for e in 0..table.num_rows as usize {
                    Self::append_mirror_value(&mut col, &mut oversized[ci], e as u64, &value_at(table, ci, e))?;
                }
                col.flush().map_err(Self::io_err)?;
                col.save_metadata().map_err(Self::io_err)?;
                columns.push(col);
            }
            entry.columns = columns;
            entry.oversized = oversized;
            entry.flushed_rows = table.num_rows;
            table.persistence_dirty = false;
        } else {
            if entry.columns.is_empty() {
                for ci in 0..num_cols {
                    let def = if ci < 2 {
                        Self::rel_structural_def(ci)
                    } else {
                        table.columns[ci - 2].clone()
                    };
                    entry
                        .columns
                        .push(Self::build_column(&def, table.table_id, ci as u32, db_path, bm, page_size));
                }
                entry.oversized = (0..num_cols).map(|_| HashMap::new()).collect();
            }
            for e in entry.flushed_rows as usize..table.num_rows as usize {
                for (ci, col) in entry.columns.iter_mut().enumerate() {
                    Self::append_mirror_value(col, &mut entry.oversized[ci], e as u64, &value_at(table, ci, e))?;
                }
            }
            for col in entry.columns.iter_mut() {
                col.flush().map_err(Self::io_err)?;
                col.save_metadata().map_err(Self::io_err)?;
            }
            entry.flushed_rows = table.num_rows;
        }
        Self::save_overflow(table.table_id, &entry.oversized, db_path)?;
        Ok(())
    }

    /// Load a rel table from its durable column mirror.
    pub fn load_rel_table(
        &self,
        table: &mut RelTable,
        db_path: &Path,
        bm: &Arc<Mutex<BufferManager>>,
        page_size: usize,
    ) -> Result<bool, StorageError> {
        let num_prop_cols = table.columns.len();
        let num_cols = num_prop_cols + 2;
        let mut columns = Vec::with_capacity(num_cols);
        for ci in 0..num_cols {
            let def = if ci < 2 {
                Self::rel_structural_def(ci)
            } else {
                table.columns[ci - 2].clone()
            };
            let mut col = Self::build_column(&def, table.table_id, ci as u32, db_path, bm, page_size);
            if !col.load_metadata().map_err(Self::io_err)? {
                return Ok(false);
            }
            columns.push(col);
        }

        let num_rows = columns[0].num_values as usize;
        let oversized = Self::load_overflow(table.table_id, num_cols, db_path);
        let mut edges = Vec::with_capacity(num_rows);
        let mut properties = vec![Vec::with_capacity(num_rows); num_prop_cols];
        let mut fwd_adj: HashMap<u64, Vec<(u64, usize)>> = HashMap::new();
        let mut rev_adj: HashMap<u64, Vec<(u64, usize)>> = HashMap::new();
        let structural = |ci: usize, e: u64| -> Value {
            if let Some(bytes) = oversized[ci].get(&e) {
                Column::deserialize_value_bytes(bytes).unwrap_or(Value::Null)
            } else {
                columns[ci].get_value(e).unwrap_or(Value::Null)
            }
        };
        for e in 0..num_rows {
            let src = match structural(0, e as u64) {
                Value::UInt64(v) => v,
                _ => u64::MAX,
            };
            let dst = match structural(1, e as u64) {
                Value::UInt64(v) => v,
                _ => u64::MAX,
            };
            edges.push((src, dst));
            if src != u64::MAX {
                fwd_adj.entry(src).or_default().push((dst, e));
                rev_adj.entry(dst).or_default().push((src, e));
            }
            for ci in 0..num_prop_cols {
                let prop = if let Some(bytes) = oversized[ci + 2].get(&(e as u64)) {
                    Column::deserialize_value_bytes(bytes).unwrap_or(Value::Null)
                } else {
                    columns[ci + 2].get_value(e as u64).unwrap_or(Value::Null)
                };
                properties[ci].push(prop);
            }
        }

        table.edges = edges;
        table.fwd_adj = fwd_adj;
        table.rev_adj = rev_adj;
        table.properties = properties;
        table.num_rows = num_rows as u64;
        table.csr_index = None;

        let mut state = self.inner.lock().unwrap();
        state.insert(
            table.table_id,
            TablePersistenceState {
                columns,
                flushed_rows: table.num_rows,
                oversized,
            },
        );
        Ok(num_rows > 0)
    }

    // ---------------------------------------------------------------------
    // All-tables helpers
    // ---------------------------------------------------------------------

    /// Sync all node + rel tables into their durable mirrors.
    pub fn persist_all(
        &self,
        catalog: &Arc<TableCatalog>,
        db_path: &Path,
        bm: &Arc<Mutex<BufferManager>>,
        page_size: usize,
    ) -> Result<(), StorageError> {
        let node_ids: Vec<u64> = catalog.all_node_tables().iter().map(|r| *r.key()).collect();
        for tid in node_ids {
            if let Some(mut table) = catalog.get_node_table_mut(tid) {
                self.sync_node_table(&mut table, db_path, bm, page_size)?;
            }
        }
        let rel_ids: Vec<u64> = catalog.all_rel_tables().iter().map(|r| *r.key()).collect();
        for tid in rel_ids {
            if let Some(mut table) = catalog.get_rel_table_mut(tid) {
                self.sync_rel_table(&mut table, db_path, bm, page_size)?;
            }
        }
        Ok(())
    }

    /// Load all persisted tables from their durable mirrors.
    ///
    /// Returns the number of tables that had persisted data.
    pub fn load_all(
        &self,
        catalog: &Arc<TableCatalog>,
        db_path: &Path,
        bm: &Arc<Mutex<BufferManager>>,
        page_size: usize,
    ) -> Result<usize, StorageError> {
        let mut loaded = 0usize;
        let node_ids: Vec<u64> = catalog.all_node_tables().iter().map(|r| *r.key()).collect();
        for tid in node_ids {
            if let Some(mut table) = catalog.get_node_table_mut(tid) {
                if self.load_node_table(&mut table, db_path, bm, page_size)? {
                    loaded += 1;
                }
            }
        }
        let rel_ids: Vec<u64> = catalog.all_rel_tables().iter().map(|r| *r.key()).collect();
        for tid in rel_ids {
            if let Some(mut table) = catalog.get_rel_table_mut(tid) {
                if self.load_rel_table(&mut table, db_path, bm, page_size)? {
                    loaded += 1;
                }
            }
        }
        Ok(loaded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer_manager::{BufferManager, BufferManagerConfig};
    use crate::table::{NodeTable, RelTable};
    use akar_common::memory::MemoryManager;
    use tempfile::TempDir;

    fn test_dir() -> TempDir {
        TempDir::new().expect("Failed to create temp dir")
    }

    fn buffer_manager(db_path: &Path) -> Arc<Mutex<BufferManager>> {
        std::fs::create_dir_all(db_path).expect("Failed to create db dir");
        let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
        Arc::new(Mutex::new(BufferManager::new(
            db_path.to_path_buf(),
            mm,
            BufferManagerConfig::default(),
        )))
    }

    fn node_defs() -> Vec<ColumnDefinition> {
        vec![
            ColumnDefinition {
                name: "name".into(),
                logical_type: LogicalTypeID::String,
                is_primary_key: true,
                compression: CompressionType::Uncompressed,
            },
            ColumnDefinition {
                name: "age".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: false,
                compression: CompressionType::Uncompressed,
            },
        ]
    }

    fn rel_defs() -> Vec<ColumnDefinition> {
        vec![ColumnDefinition {
            name: "since".into(),
            logical_type: LogicalTypeID::Int64,
            is_primary_key: false,
            compression: CompressionType::Uncompressed,
        }]
    }

    #[test]
    fn test_node_table_mirror_roundtrip() {
        let dir = test_dir();
        let db_path = dir.path().join("db");
        let bm = buffer_manager(&db_path);
        let page_size = bm.lock().unwrap().page_size();
        let persistence = TablePersistence::new();

        let mut table = NodeTable::new(1, "Person".into(), node_defs());
        table
            .insert_row(vec![Value::String("alice".into()), Value::Int64(30)])
            .unwrap();
        table
            .insert_row(vec![Value::String("bob".into()), Value::Int64(25)])
            .unwrap();
        table
            .insert_row(vec![Value::String("carol".into()), Value::Int64(40)])
            .unwrap();

        persistence.sync_node_table(&mut table, &db_path, &bm, page_size).unwrap();
        assert!(!table.persistence_dirty, "sync should clear the dirty flag");

        // Fresh table with the same schema must reload the mirrored rows.
        let mut restored = NodeTable::new(1, "Person".into(), node_defs());
        let loaded = persistence
            .load_node_table(&mut restored, &db_path, &bm, page_size)
            .unwrap();
        assert!(loaded, "mirror should exist and be loadable");
        assert_eq!(restored.num_rows, 3);
        assert_eq!(restored.get_value(0, 0), Some(&Value::String("alice".into())));
        assert_eq!(restored.get_value(1, 1), Some(&Value::Int64(25)));
        assert_eq!(restored.get_value(2, 0), Some(&Value::String("carol".into())));
    }

    #[test]
    fn test_node_table_mirror_incremental_append() {
        let dir = test_dir();
        let db_path = dir.path().join("db");
        let bm = buffer_manager(&db_path);
        let page_size = bm.lock().unwrap().page_size();
        let persistence = TablePersistence::new();

        let mut table = NodeTable::new(1, "Person".into(), node_defs());
        table
            .insert_row(vec![Value::String("alice".into()), Value::Int64(30)])
            .unwrap();
        persistence.sync_node_table(&mut table, &db_path, &bm, page_size).unwrap();

        // Insert more rows — the mirror must append incrementally.
        table
            .insert_row(vec![Value::String("bob".into()), Value::Int64(25)])
            .unwrap();
        persistence.sync_node_table(&mut table, &db_path, &bm, page_size).unwrap();

        let mut restored = NodeTable::new(1, "Person".into(), node_defs());
        persistence
            .load_node_table(&mut restored, &db_path, &bm, page_size)
            .unwrap();
        assert_eq!(restored.num_rows, 2);
        assert_eq!(restored.get_value(1, 0), Some(&Value::String("bob".into())));
    }

    #[test]
    fn test_node_table_mirror_update_triggers_rewrite() {
        let dir = test_dir();
        let db_path = dir.path().join("db");
        let bm = buffer_manager(&db_path);
        let page_size = bm.lock().unwrap().page_size();
        let persistence = TablePersistence::new();

        let mut table = NodeTable::new(1, "Person".into(), node_defs());
        table
            .insert_row(vec![Value::String("alice".into()), Value::Int64(30)])
            .unwrap();
        persistence.sync_node_table(&mut table, &db_path, &bm, page_size).unwrap();

        // UPDATE marks the table dirty → the mirror is rewritten from scratch.
        table.update_cell(0, 1, Value::Int64(31)).unwrap();
        assert!(table.persistence_dirty);
        persistence.sync_node_table(&mut table, &db_path, &bm, page_size).unwrap();
        assert!(!table.persistence_dirty);

        let mut restored = NodeTable::new(1, "Person".into(), node_defs());
        persistence
            .load_node_table(&mut restored, &db_path, &bm, page_size)
            .unwrap();
        assert_eq!(restored.num_rows, 1);
        assert_eq!(restored.get_value(0, 1), Some(&Value::Int64(31)));
    }

    #[test]
    fn test_rel_table_mirror_roundtrip() {
        let dir = test_dir();
        let db_path = dir.path().join("db");
        let bm = buffer_manager(&db_path);
        let page_size = bm.lock().unwrap().page_size();
        let persistence = TablePersistence::new();

        let mut table = RelTable::new(2, "LivesIn".into(), 1, 1, rel_defs());
        table.insert_rel(0, 1, vec![Value::Int64(2010)]).unwrap();
        table.insert_rel(0, 2, vec![Value::Int64(2015)]).unwrap();
        table.insert_rel(3, 1, vec![Value::Int64(2020)]).unwrap();

        persistence.sync_rel_table(&mut table, &db_path, &bm, page_size).unwrap();
        assert!(!table.persistence_dirty);

        let mut restored = RelTable::new(2, "LivesIn".into(), 1, 1, rel_defs());
        let loaded = persistence
            .load_rel_table(&mut restored, &db_path, &bm, page_size)
            .unwrap();
        assert!(loaded, "rel mirror should exist and be loadable");
        assert_eq!(restored.num_rows, 3);
        assert_eq!(restored.edges, vec![(0, 1), (0, 2), (3, 1)]);
        assert_eq!(
            restored.fwd_adj.get(&0).cloned(),
            Some(vec![(1, 0), (2, 1)])
        );
        assert_eq!(restored.rev_adj.get(&1).cloned(), Some(vec![(0, 0), (3, 2)]));
        assert_eq!(restored.properties[0], vec![Value::Int64(2010), Value::Int64(2015), Value::Int64(2020)]);
    }

    #[test]
    fn test_node_table_mirror_oversized_value() {
        let dir = test_dir();
        let db_path = dir.path().join("db");
        let bm = buffer_manager(&db_path);
        let page_size = bm.lock().unwrap().page_size();
        let persistence = TablePersistence::new();

        let long = "A".repeat(page_size * 4);
        let mut table = NodeTable::new(1, "Person".into(), node_defs());
        table
            .insert_row(vec![Value::String("alice".into()), Value::Int64(30)])
            .unwrap();
        table
            .insert_row(vec![Value::String(long.clone()), Value::Int64(25)])
            .unwrap();
        table
            .insert_row(vec![Value::String("carol".into()), Value::Int64(40)])
            .unwrap();

        persistence.sync_node_table(&mut table, &db_path, &bm, page_size).unwrap();
        assert!(!table.persistence_dirty);

        // The oversized string is stored in the overflow sidecar.
        assert!(db_path.join("col_1.ovf").exists(), "overflow sidecar should be written");

        let mut restored = NodeTable::new(1, "Person".into(), node_defs());
        persistence
            .load_node_table(&mut restored, &db_path, &bm, page_size)
            .unwrap();
        assert_eq!(restored.num_rows, 3);
        assert_eq!(restored.get_value(0, 0), Some(&Value::String("alice".into())));
        assert_eq!(restored.get_value(1, 0), Some(&Value::String(long.clone())));
        assert_eq!(restored.get_value(1, 1), Some(&Value::Int64(25)));
        assert_eq!(restored.get_value(2, 0), Some(&Value::String("carol".into())));

        // A subsequent incremental append must keep the oversized row intact.
        table
            .insert_row(vec![Value::String("dave".into()), Value::Int64(50)])
            .unwrap();
        persistence.sync_node_table(&mut table, &db_path, &bm, page_size).unwrap();
        let mut restored2 = NodeTable::new(1, "Person".into(), node_defs());
        persistence
            .load_node_table(&mut restored2, &db_path, &bm, page_size)
            .unwrap();
        assert_eq!(restored2.num_rows, 4);
        assert_eq!(restored2.get_value(1, 0), Some(&Value::String(long.clone())));
        assert_eq!(restored2.get_value(3, 0), Some(&Value::String("dave".into())));
    }

    #[test]
    fn test_remove_deletes_mirror_files() {
        let dir = test_dir();
        let db_path = dir.path().join("db");
        let bm = buffer_manager(&db_path);
        let page_size = bm.lock().unwrap().page_size();
        let persistence = TablePersistence::new();

        let mut table = NodeTable::new(1, "Person".into(), node_defs());
        table
            .insert_row(vec![Value::String("alice".into()), Value::Int64(30)])
            .unwrap();
        persistence.sync_node_table(&mut table, &db_path, &bm, page_size).unwrap();

        let col_file = db_path.join("col_1_0");
        assert!(col_file.exists(), "column data file should exist after sync");

        persistence.remove(1, &db_path, &bm);
        assert!(!col_file.exists(), "column data file should be removed on drop");
        assert!(!db_path.join("col_1_0.meta").exists());
    }
}
