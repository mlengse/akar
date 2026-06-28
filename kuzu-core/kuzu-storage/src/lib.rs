//! Kuzu storage engine.
//!
//! Disk-based columnar storage with buffer management, WAL, compression, and indexing.

pub mod page;
pub mod buffer_manager;
pub mod wal;
pub mod compression;
pub mod shadow_file;
pub mod local_storage;
pub mod table;
pub mod index;
pub mod stats;
pub mod checkpoint;

use buffer_manager::{BufferManager, BufferManagerConfig};
use kuzu_common::memory::MemoryManager;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub use table::{TableCatalog, NodeTable, RelTable, ColumnDefinition};

/// The storage manager — root of the storage engine.
#[allow(dead_code)]
pub struct StorageManager {
    db_path: PathBuf,
    buffer_manager: Arc<Mutex<BufferManager>>,
    memory_manager: Arc<MemoryManager>,
    /// In-memory table catalog holding actual data for all tables.
    pub(crate) table_catalog: Arc<Mutex<TableCatalog>>,
}

impl StorageManager {
    pub fn new(db_path: PathBuf, memory_manager: Arc<MemoryManager>) -> Self {
        let config = BufferManagerConfig::default();
        let bm = BufferManager::new(db_path.clone(), memory_manager.clone(), config);
        Self {
            db_path,
            buffer_manager: Arc::new(Mutex::new(bm)),
            memory_manager,
            table_catalog: Arc::new(Mutex::new(TableCatalog::new())),
        }
    }

    pub fn buffer_manager(&self) -> &Arc<Mutex<BufferManager>> {
        &self.buffer_manager
    }

    pub fn db_path(&self) -> &PathBuf {
        &self.db_path
    }

    /// Get a reference to the table catalog for reading/writing table data.
    pub fn table_catalog(&self) -> Arc<Mutex<TableCatalog>> {
        self.table_catalog.clone()
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
}
