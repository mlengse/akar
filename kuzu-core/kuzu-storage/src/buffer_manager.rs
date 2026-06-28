//! Buffer manager — manages in-memory page cache with eviction.

use kuzu_common::memory::MemoryManager;
use std::collections::HashMap;
use std::sync::Arc;

/// A handle to a page in the buffer pool.
#[derive(Debug, Clone)]
pub struct PageHandle {
    pub page_id: u64,
    pub data: Vec<u8>,
}

/// The buffer manager manages a pool of pinned/unpinned pages in memory.
#[allow(dead_code)]
pub struct BufferManager {
    memory_manager: Arc<MemoryManager>,
    page_size: usize,
    /// Simple page table: page_id → PageHandle
    pages: HashMap<u64, PageHandle>,
    max_pages: usize,
}

impl BufferManager {
    pub fn new(memory_manager: Arc<MemoryManager>) -> Self {
        let page_size = 8192; // 8KB default
        let max_memory = memory_manager.max_memory();
        let max_pages = if max_memory == u64::MAX {
            10000
        } else {
            (max_memory as usize) / page_size
        };

        Self {
            memory_manager,
            page_size,
            pages: HashMap::new(),
            max_pages,
        }
    }

    pub fn page_size(&self) -> usize {
        self.page_size
    }

    pub fn num_pages(&self) -> usize {
        self.pages.len()
    }

    /// Get a page from the buffer (or returns None if not loaded).
    pub fn get_page(&self, page_id: u64) -> Option<&PageHandle> {
        self.pages.get(&page_id)
    }

    /// Pin a page into the buffer pool.
    pub fn pin_page(&mut self, page_id: u64, data: Vec<u8>) {
        let handle = PageHandle { page_id, data };
        self.memory_manager
            .allocate(self.page_size as u64);
        self.pages.insert(page_id, handle);
    }

    /// Unpin (remove) a page from the buffer pool.
    pub fn unpin_page(&mut self, page_id: u64) {
        if self.pages.remove(&page_id).is_some() {
            self.memory_manager
                .deallocate(self.page_size as u64);
        }
    }
}
