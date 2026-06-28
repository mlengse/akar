//! Buffer manager — manages in-memory page cache with Clock eviction policy.

use crate::page::{Frame, PageNum, DEFAULT_PAGE_SIZE};
use kuzu_common::memory::MemoryManager;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Configuration for the buffer manager.
#[derive(Debug, Clone)]
pub struct BufferManagerConfig {
    /// Maximum memory to use for the buffer pool (in bytes).
    pub max_memory: u64,
    /// Page size in bytes.
    pub page_size: usize,
}

impl Default for BufferManagerConfig {
    fn default() -> Self {
        Self {
            max_memory: 64 * 1024 * 1024, // 64MB default
            page_size: DEFAULT_PAGE_SIZE,
        }
    }
}

/// Statistics for the buffer manager.
#[derive(Debug, Default, Clone, Copy)]
pub struct BufferManagerStats {
    /// Total number of page faults (disk reads).
    pub page_faults: u64,
    /// Total number of page writes to disk.
    pub page_writes: u64,
    /// Current number of frames in the pool.
    pub num_frames: usize,
    /// Number of dirty frames.
    pub dirty_frames: usize,
    /// Number of pinned frames.
    pub pinned_frames: usize,
}

/// The buffer manager manages a pool of frames with a Clock eviction policy.
#[allow(dead_code)]
pub struct BufferManager {
    /// Path to the database directory.
    db_path: PathBuf,
    /// Page size in bytes.
    page_size: usize,
    /// Maximum number of frames allowed.
    max_frames: usize,
    /// Frame table: page_num → Frame.
    frames: HashMap<PageNum, Frame>,
    /// File handles for each database file (path → FileHandle data).
    files: HashMap<String, FileHandleInfo>,
    /// Clock hand index for eviction.
    clock_hand: usize,
    /// Ordered list of page numbers for Clock algorithm.
    clock_order: Vec<PageNum>,
    /// Memory manager for tracking allocation.
    memory_manager: Arc<MemoryManager>,
    /// Statistics.
    stats: BufferManagerStats,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct FileHandleInfo {
    path: PathBuf,
    num_pages: u64,
}

impl BufferManager {
    pub fn new(
        db_path: PathBuf,
        memory_manager: Arc<MemoryManager>,
        config: BufferManagerConfig,
    ) -> Self {
        let max_frames = if config.max_memory > 0 {
            (config.max_memory / config.page_size as u64) as usize
        } else {
            1000
        };

        Self {
            db_path,
            page_size: config.page_size,
            max_frames,
            frames: HashMap::new(),
            files: HashMap::new(),
            clock_hand: 0,
            clock_order: Vec::new(),
            memory_manager,
            stats: BufferManagerStats::default(),
        }
    }

    pub fn stats(&self) -> &BufferManagerStats {
        &self.stats
    }

    pub fn page_size(&self) -> usize {
        self.page_size
    }

    pub fn num_frames(&self) -> usize {
        self.frames.len()
    }

    /// Register a database file with the buffer manager.
    pub fn register_file(&mut self, name: &str, path: PathBuf) {
        let num_pages = if path.exists() {
            let len = std::fs::metadata(&path)
                .map(|m| m.len())
                .unwrap_or(0);
            len / self.page_size as u64
        } else {
            0
        };
        self.files.insert(
            name.to_string(),
            FileHandleInfo { path, num_pages },
        );
    }

    /// Pin a page: bring it into the buffer pool if not already present.
    pub fn pin(&mut self, file_name: &str, page_num: PageNum) -> std::io::Result<&Frame> {
        if let Some(frame) = self.frames.get_mut(&page_num) {
            frame.pin();
            frame.clock_ref = true;
            return Ok(unsafe { &*(frame as *const Frame) });
        }

        // Page fault
        self.stats.page_faults += 1;

        if self.frames.len() >= self.max_frames {
            self.evict()?;
        }

        let data = self.read_from_disk(file_name, page_num)?;
        let mut frame = Frame::new(page_num, data);
        frame.pin();
        self.clock_order.push(page_num);
        self.frames.insert(page_num, frame);
        self.memory_manager.allocate(self.page_size as u64);

        self.update_stats();
        Ok(self.frames.get(&page_num).unwrap())
    }

    /// Pin a page and get mutable access.
    pub fn pin_mut(&mut self, file_name: &str, page_num: PageNum) -> std::io::Result<&mut Frame> {
        if !self.frames.contains_key(&page_num) {
            self.pin(file_name, page_num)?;
        }
        if let Some(frame) = self.frames.get_mut(&page_num) {
            frame.pin();
            frame.clock_ref = true;
            return Ok(frame);
        }
        unreachable!()
    }

    /// Unpin a page.
    pub fn unpin(&mut self, page_num: PageNum) {
        if let Some(frame) = self.frames.get_mut(&page_num) {
            frame.unpin();
        }
        self.update_stats();
    }

    /// Flush a dirty page to disk.
    pub fn flush(&mut self, file_name: &str, page_num: PageNum) -> std::io::Result<()> {
        if let Some(frame) = self.frames.get(&page_num) {
            if frame.is_dirty {
                self.write_to_disk(file_name, page_num, &frame.data)?;
                if let Some(f) = self.frames.get_mut(&page_num) {
                    f.is_dirty = false;
                }
                self.stats.page_writes += 1;
            }
        }
        self.update_stats();
        Ok(())
    }

    /// Flush all dirty pages to disk.
    pub fn flush_all(&mut self) -> std::io::Result<()> {
        let dirty: Vec<PageNum> = self
            .frames
            .iter()
            .filter(|(_, f)| f.is_dirty)
            .map(|(k, _)| *k)
            .collect();

        for page_num in dirty {
            for name in self.files.keys().cloned().collect::<Vec<_>>() {
                self.flush(&name, page_num)?;
            }
        }
        Ok(())
    }

    // --- Clock eviction ---

    fn evict(&mut self) -> std::io::Result<()> {
        let n = self.clock_order.len();
        if n == 0 {
            return Ok(());
        }

        for _ in 0..=n {
            if self.clock_hand >= n {
                self.clock_hand = 0;
            }
            let page_num = self.clock_order[self.clock_hand];

            if let Some(frame) = self.frames.get(&page_num) {
                if frame.is_pinned() {
                    self.clock_hand += 1;
                    continue;
                }
                if frame.clock_ref {
                    self.frames.get_mut(&page_num).unwrap().clock_ref = false;
                    self.clock_hand += 1;
                    continue;
                }
                // Victim found
                if frame.is_dirty {
                    for name in self.files.keys().cloned().collect::<Vec<_>>() {
                        self.write_to_disk(&name, page_num, &frame.data)?;
                        self.stats.page_writes += 1;
                    }
                }
                self.frames.remove(&page_num);
                self.clock_order.remove(self.clock_hand);
                self.memory_manager.deallocate(self.page_size as u64);
                return Ok(());
            }
        }
        Ok(())
    }

    // --- Disk I/O ---

    fn read_from_disk(&self, file_name: &str, page_num: PageNum) -> std::io::Result<Vec<u8>> {
        if let Some(fh) = self.files.get(file_name) {
            use std::io::{Read, Seek, SeekFrom};
            let mut file = std::fs::File::open(&fh.path)?;
            file.seek(SeekFrom::Start(page_num * self.page_size as u64))?;
            let mut buf = vec![0u8; self.page_size];
            file.read_exact(&mut buf)?;
            Ok(buf)
        } else {
            Ok(vec![0u8; self.page_size])
        }
    }

    fn write_to_disk(&self, file_name: &str, page_num: PageNum, data: &[u8]) -> std::io::Result<()> {
        if let Some(fh) = self.files.get(file_name) {
            use std::io::{Seek, SeekFrom, Write};
            use std::fs::OpenOptions;
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .open(&fh.path)?;
            file.seek(SeekFrom::Start(page_num * self.page_size as u64))?;
            file.write_all(data)?;
        }
        Ok(())
    }

    fn update_stats(&mut self) {
        self.stats.num_frames = self.frames.len();
        self.stats.dirty_frames = self.frames.values().filter(|f| f.is_dirty).count();
        self.stats.pinned_frames = self.frames.values().filter(|f| f.is_pinned()).count();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_bm() -> (BufferManager, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mm = Arc::new(MemoryManager::new(1024 * 1024));
        let config = BufferManagerConfig {
            max_memory: 256 * 1024,
            page_size: DEFAULT_PAGE_SIZE,
        };
        let mut bm = BufferManager::new(dir.path().to_path_buf(), mm, config);
        let db_path = dir.path().join("test.db");
        std::fs::write(&db_path, vec![0u8; 8192 * 10]).unwrap();
        bm.register_file("main", db_path);
        (bm, dir)
    }

    #[test]
    fn test_pin_unpin() {
        let (mut bm, _dir) = create_test_bm();
        let frame = bm.pin("main", 0).unwrap();
        assert_eq!(frame.page_num, 0);
        assert!(frame.is_pinned());
        bm.unpin(0);
        assert_eq!(bm.stats().page_faults, 1);
    }

    #[test]
    fn test_cached_page_no_fault() {
        let (mut bm, _dir) = create_test_bm();
        bm.pin("main", 0).unwrap();
        bm.unpin(0);
        bm.pin("main", 0).unwrap();
        bm.unpin(0);
        assert_eq!(bm.stats().page_faults, 1);
    }

    #[test]
    fn test_multiple_pages() {
        let (mut bm, _dir) = create_test_bm();
        for i in 0..5 {
            bm.pin("main", i).unwrap();
            bm.unpin(i);
        }
        assert_eq!(bm.stats().page_faults, 5);
        assert_eq!(bm.num_frames(), 5);
    }

    #[test]
    fn test_dirty_and_flush() {
        let (mut bm, _dir) = create_test_bm();
        let frame = bm.pin_mut("main", 1).unwrap();
        frame.mark_dirty();
        bm.unpin(1);
        assert_eq!(bm.stats().dirty_frames, 1);
        bm.flush("main", 1).unwrap();
        assert_eq!(bm.stats().dirty_frames, 0);
        assert_eq!(bm.stats().page_writes, 1);
    }

    #[test]
    fn test_clock_eviction() {
        let dir = tempfile::tempdir().unwrap();
        let mm = Arc::new(MemoryManager::new(3 * DEFAULT_PAGE_SIZE as u64));
        let config = BufferManagerConfig {
            max_memory: 3 * DEFAULT_PAGE_SIZE as u64,
            page_size: DEFAULT_PAGE_SIZE,
        };
        let mut bm = BufferManager::new(dir.path().to_path_buf(), mm, config);
        let db_path = dir.path().join("test.db");
        std::fs::write(&db_path, vec![0u8; 8192 * 20]).unwrap();
        bm.register_file("main", db_path);

        for i in 0..3 {
            bm.pin("main", i).unwrap();
            bm.unpin(i);
        }
        assert_eq!(bm.num_frames(), 3);

        bm.pin("main", 3).unwrap();
        bm.unpin(3);
        assert_eq!(bm.num_frames(), 3);
    }

    #[test]
    fn test_flush_all() {
        let (mut bm, _dir) = create_test_bm();
        for i in 0..3 {
            let frame = bm.pin_mut("main", i).unwrap();
            frame.mark_dirty();
            bm.unpin(i);
        }
        bm.flush_all().unwrap();
        assert_eq!(bm.stats().dirty_frames, 0);
    }
}

