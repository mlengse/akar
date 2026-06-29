//! Kuzu storage engine.
//!
//! Disk-based columnar storage with buffer management, WAL, compression, and indexing.

pub mod page;
pub mod buffer_manager;
pub mod column;
pub mod column_chunk;
pub mod node_group;
pub mod wal;
pub mod compression;
pub mod shadow_file;
pub mod local_storage;
pub mod table;
pub mod index;
pub mod stats;
pub mod checkpoint;

use buffer_manager::{BufferManager, BufferManagerConfig};
use checkpoint::checkpoint;
use kuzu_common::memory::MemoryManager;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use wal::WAL;

pub use table::{TableCatalog, NodeTable, RelTable, ColumnDefinition};
pub use column_chunk::{ColumnChunk, NODE_GROUP_SIZE};
pub use node_group::NodeGroup;

/// The storage manager — root of the storage engine.
#[allow(dead_code)]
pub struct StorageManager {
    db_path: PathBuf,
    buffer_manager: Arc<Mutex<BufferManager>>,
    wal: Arc<Mutex<WAL>>,
    memory_manager: Arc<MemoryManager>,
    /// In-memory table catalog holding actual data for all tables.
    pub(crate) table_catalog: Arc<Mutex<TableCatalog>>,
}

impl StorageManager {
    pub fn new(db_path: PathBuf, memory_manager: Arc<MemoryManager>) -> Self {
        let config = BufferManagerConfig::default();
        let bm = BufferManager::new(db_path.clone(), memory_manager.clone(), config);
        let wal_path = db_path.join("wal.log");
        // If a WAL file exists from a previous session, recover from it.
        // For now, start with a fresh WAL.
        if wal_path.exists() {
            let _ = std::fs::remove_file(&wal_path);
        }
        let wal = WAL::new(wal_path);
        Self {
            db_path,
            buffer_manager: Arc::new(Mutex::new(bm)),
            wal: Arc::new(Mutex::new(wal)),
            memory_manager,
            table_catalog: Arc::new(Mutex::new(TableCatalog::new())),
        }
    }

    pub fn buffer_manager(&self) -> &Arc<Mutex<BufferManager>> {
        &self.buffer_manager
    }

    pub fn wal(&self) -> &Arc<Mutex<WAL>> {
        &self.wal
    }

    pub fn db_path(&self) -> &PathBuf {
        &self.db_path
    }

    /// Get a reference to the table catalog for reading/writing table data.
    pub fn table_catalog(&self) -> Arc<Mutex<TableCatalog>> {
        self.table_catalog.clone()
    }

    /// Log a column write to the WAL before applying it to the BufferManager.
    pub fn log_column_write(&self, table_id: u64, col_id: u32, page_id: u64, data: &[u8]) {
        let mut wal = self.wal.lock().unwrap();
        wal.log_column_write(table_id, col_id, page_id, data);
    }

    /// Create a node table in the catalog and return its ID.
    pub fn create_node_table(
        &self,
        name: String,
        columns: Vec<ColumnDefinition>,
    ) -> NodeTable {
        let mut catalog = self.table_catalog.lock().unwrap();
        catalog.create_node_table(name, columns)
    }

    /// Create a rel table in the catalog.
    pub fn create_rel_table(
        &self,
        name: String,
        src_table_id: u64,
        dst_table_id: u64,
        columns: Vec<ColumnDefinition>,
    ) -> RelTable {
        let mut catalog = self.table_catalog.lock().unwrap();
        catalog.create_rel_table(name, src_table_id, dst_table_id, columns)
    }

    /// Perform a checkpoint: flush WAL + dirty pages to disk.
    pub fn checkpoint(&self) -> std::io::Result<checkpoint::CheckpointResult> {
        let mut wal = self.wal.lock().unwrap();
        checkpoint(&mut wal, &self.buffer_manager)
    }
}
