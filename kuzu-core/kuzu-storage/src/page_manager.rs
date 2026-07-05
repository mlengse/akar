//! Page Manager — allocates and frees database pages.
//!
//! Wraps the `FreeSpaceManager` and `FileHandle` to provide page-level
//! allocation and deallocation. On allocation, the FSM is consulted first;
//! if no free pages are available, the file is extended.
//!
//! Ported from C++ `src/storage/page_manager.cpp`.

use crate::free_space_manager::{FreeSpaceManager, PageRange, INVALID_PAGE_IDX};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Manages page allocation and deallocation for a database file.
pub struct PageManager {
    /// Path to the database file (for extending).
    db_path: PathBuf,
    /// Page size in bytes.
    page_size: usize,
    /// Total number of pages in the file (allocated + free).
    total_pages: AtomicU64,
    /// Free space manager tracks reusable pages.
    fsm: Arc<FreeSpaceManager>,
    /// Last allocated page index (monotonically increasing).
    /// Used for extending the file when FSM has no free pages.
    next_page: AtomicU64,
}

impl PageManager {
    /// Create a new page manager.
    ///
    /// `existing_pages` is the number of pages already present in the file
    /// (determined from file metadata on open).
    pub fn new(
        db_path: PathBuf,
        page_size: usize,
        existing_pages: u64,
        fsm: Arc<FreeSpaceManager>,
    ) -> Self {
        Self {
            db_path,
            page_size,
            total_pages: AtomicU64::new(existing_pages),
            fsm,
            next_page: AtomicU64::new(existing_pages),
        }
    }

    /// Page size in bytes.
    pub fn page_size(&self) -> usize {
        self.page_size
    }

    /// Total number of pages managed (allocated + free).
    pub fn total_pages(&self) -> u64 {
        self.total_pages.load(Ordering::Acquire)
    }

    /// Allocate a new page index.
    ///
    /// Tries the FSM first for a free page; if none available,
    /// extends the file (incrementing `next_page`).
    pub fn allocate_page(&self) -> u64 {
        // Try FSM first
        if let Some(range) = self.fsm.pop_free_pages(1) {
            return range.start_page_idx;
        }

        // No free pages — extend the file
        let page_idx = self.next_page.fetch_add(1, Ordering::SeqCst);
        self.total_pages.fetch_add(1, Ordering::SeqCst);
        page_idx
    }

    /// Free a previously allocated page.
    ///
    /// The page is added back to the FSM for future reuse.
    pub fn free_page(&self, page_idx: u64) {
        if page_idx == INVALID_PAGE_IDX {
            return;
        }
        self.fsm.add_free_pages(PageRange::new(page_idx, 1));
    }

    /// Free a range of pages.
    pub fn free_pages(&self, start_page_idx: u64, num_pages: u64) {
        if num_pages == 0 || start_page_idx == INVALID_PAGE_IDX {
            return;
        }
        self.fsm
            .add_free_pages(PageRange::new(start_page_idx, num_pages));
    }

    /// Access the underlying free space manager.
    pub fn fsm(&self) -> &Arc<FreeSpaceManager> {
        &self.fsm
    }

    /// Get the byte offset for a page index.
    pub fn page_offset(&self, page_idx: u64) -> u64 {
        page_idx * self.page_size as u64
    }

    /// Extend the database file by `num_pages` pages.
    ///
    /// This writes zero-filled pages to the end of the file.
    /// Returns the starting page index of the newly extended region.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn extend_file(&self, num_pages: u64) -> std::io::Result<u64> {
        use std::io::Write;

        let start_page = self.total_pages.load(Ordering::Acquire);
        let extend_bytes = (num_pages as usize) * self.page_size;

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.db_path)?;

        // Write zero-filled pages
        let zeros = vec![0u8; extend_bytes];
        file.write_all(&zeros)?;
        file.flush()?;

        self.total_pages
            .fetch_add(num_pages, Ordering::SeqCst);

        Ok(start_page)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn extend_file(&self, num_pages: u64) -> std::io::Result<u64> {
        let start_page = self.total_pages.load(Ordering::Acquire);
        self.total_pages
            .fetch_add(num_pages, Ordering::SeqCst);
        Ok(start_page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn make_pm() -> (PageManager, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let fsm = Arc::new(FreeSpaceManager::new());
        let pm = PageManager::new(db_path, 4096, 0, fsm);
        (pm, dir)
    }

    #[test]
    fn test_allocate_extends_file() {
        let (pm, _dir) = make_pm();
        assert_eq!(pm.total_pages(), 0);

        let p1 = pm.allocate_page();
        assert_eq!(p1, 0);
        assert_eq!(pm.total_pages(), 1);

        let p2 = pm.allocate_page();
        assert_eq!(p2, 1);
        assert_eq!(pm.total_pages(), 2);
    }

    #[test]
    fn test_free_and_reuse() {
        let (pm, _dir) = make_pm();

        let p1 = pm.allocate_page();
        let _p2 = pm.allocate_page();
        assert_eq!(pm.total_pages(), 2);

        // Free p1 — should be reused on next allocation
        pm.free_page(p1);

        let p3 = pm.allocate_page();
        assert_eq!(p3, p1); // reused
        assert_eq!(pm.total_pages(), 2); // no extension
    }

    #[test]
    fn test_allocate_multiple_no_fsm() {
        let (pm, _dir) = make_pm();

        for i in 0..10 {
            let p = pm.allocate_page();
            assert_eq!(p, i);
        }
        assert_eq!(pm.total_pages(), 10);
    }
}
