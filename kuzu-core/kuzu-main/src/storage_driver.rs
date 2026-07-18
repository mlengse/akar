use kuzu_catalog::Catalog;
use kuzu_common::file_system::VirtualFileSystemRegistry;
use kuzu_storage::{BufferInfo, FileInfo, FsmInfo, StorageInfo, StorageManager};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// High-level storage access API.
///
/// Provides programmatic access to storage-level metadata (page counts,
/// buffer manager stats, file sizes, FSM state, table layout) without
/// going through Cypher queries.
///
/// Obtain via [`crate::Database::storage_driver()`].
pub struct StorageDriver {
    storage_manager: Arc<StorageManager>,
    catalog: Arc<Mutex<Catalog>>,
    vfs: Arc<VirtualFileSystemRegistry>,
}

impl StorageDriver {
    pub(crate) fn new(
        storage_manager: Arc<StorageManager>,
        catalog: Arc<Mutex<Catalog>>,
        vfs: Arc<VirtualFileSystemRegistry>,
    ) -> Self {
        Self {
            storage_manager,
            catalog,
            vfs,
        }
    }

    /// Database path as reported by the storage engine.
    pub fn db_path(&self) -> &Path {
        self.storage_manager.db_path()
    }

    /// Aggregate storage info (page count, page size, free pages).
    pub fn storage_info(&self) -> StorageInfo {
        self.storage_manager.storage_info()
    }

    /// Buffer manager memory and pin statistics.
    pub fn buffer_info(&self) -> BufferInfo {
        self.storage_manager.buffer_info()
    }

    /// File-level size statistics (data pages + WAL).
    pub fn file_info(&self) -> FileInfo {
        self.storage_manager.file_info()
    }

    /// Free space manager summary.
    pub fn fsm_info(&self) -> FsmInfo {
        self.storage_manager.fsm_info()
    }

    /// Current WAL size in bytes.
    pub fn wal_size(&self) -> usize {
        self.storage_manager.wal_size()
    }

    /// Access the table catalog for reading table/column metadata.
    pub fn table_catalog(&self) -> Arc<kuzu_storage::TableCatalog> {
        self.storage_manager.table_catalog()
    }

    /// Access the node/rel catalog.
    pub fn catalog(&self) -> &Arc<Mutex<Catalog>> {
        &self.catalog
    }

    /// Access the virtual file system registry.
    pub fn vfs(&self) -> &Arc<VirtualFileSystemRegistry> {
        &self.vfs
    }

    /// Number of tables (node + rel) in the catalog.
    pub fn num_tables(&self) -> usize {
        self.catalog.lock().unwrap().all_entries().count()
    }

    /// Number of node tables.
    pub fn num_node_tables(&self) -> usize {
        self.catalog
            .lock()
            .unwrap()
            .all_entries()
            .filter(|e| e.is_node_table())
            .count()
    }

    /// Number of rel tables.
    pub fn num_rel_tables(&self) -> usize {
        self.catalog
            .lock()
            .unwrap()
            .all_entries()
            .filter(|e| !e.is_node_table())
            .count()
    }

    /// Total number of pages allocated across all data files.
    pub fn total_pages(&self) -> u64 {
        self.storage_info().total_pages
    }

    /// Total data file size on disk in bytes.
    pub fn total_file_size(&self) -> u64 {
        self.file_info().total_file_size
    }

    /// Number of pinned buffer frames.
    pub fn pinned_frames(&self) -> usize {
        self.buffer_info().num_pinned
    }
}
