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
use std::sync::Arc;

/// The storage manager — root of the storage engine.
#[allow(dead_code)]
pub struct StorageManager {
    db_path: PathBuf,
    buffer_manager: Arc<std::sync::Mutex<BufferManager>>,
    memory_manager: Arc<MemoryManager>,
}

impl StorageManager {
    pub fn new(db_path: PathBuf, memory_manager: Arc<MemoryManager>) -> Self {
        let config = BufferManagerConfig::default();
        let bm = BufferManager::new(db_path.clone(), memory_manager.clone(), config);
        Self {
            db_path,
            buffer_manager: Arc::new(std::sync::Mutex::new(bm)),
            memory_manager,
        }
    }

    pub fn buffer_manager(&self) -> &Arc<std::sync::Mutex<BufferManager>> {
        &self.buffer_manager
    }

    pub fn db_path(&self) -> &PathBuf {
        &self.db_path
    }
}
