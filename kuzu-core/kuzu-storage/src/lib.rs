//! Kuzu storage engine.
//!
//! Disk-based columnar storage with buffer management, WAL, compression, and indexing.

pub mod buffer_manager;
pub mod wal;
pub mod compression;
pub mod shadow_file;
pub mod local_storage;
pub mod table;
pub mod index;
pub mod stats;
pub mod checkpoint;

use kuzu_common::memory::MemoryManager;
use std::path::PathBuf;
use std::sync::Arc;

/// The storage manager — root of the storage engine.
#[allow(dead_code)]
pub struct StorageManager {
    db_path: PathBuf,
    buffer_manager: Arc<buffer_manager::BufferManager>,
    memory_manager: Arc<MemoryManager>,
}

impl StorageManager {
    pub fn new(db_path: PathBuf, memory_manager: Arc<MemoryManager>) -> Self {
        let buffer_manager = Arc::new(buffer_manager::BufferManager::new(
            memory_manager.clone(),
        ));
        Self {
            db_path,
            buffer_manager,
            memory_manager,
        }
    }

    pub fn buffer_manager(&self) -> &Arc<buffer_manager::BufferManager> {
        &self.buffer_manager
    }

    pub fn db_path(&self) -> &PathBuf {
        &self.db_path
    }
}
