//! Buffer manager — manages in-memory page cache with Clock eviction policy.

#![allow(clippy::trivial_regex, clippy::collapsible_if)]

use crate::page::{DEFAULT_PAGE_SIZE, Frame, PageNum};
use akar_common::memory::MemoryManager;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// NUMA topology information.
#[derive(Debug, Clone)]
pub struct NumaInfo {
    /// Number of NUMA nodes detected on the system.
    pub num_nodes: u32,
}

impl NumaInfo {
    /// Detect the NUMA topology of the current system.
    ///
    /// - On Windows: returns 1 (Windows NUMA API is complex; simple default).
    /// - On Linux: parses `/sys/devices/system/node/node*` directories.
    /// - On other platforms: returns 1.
    pub fn detect() -> Self {
        let num_nodes = Self::detect_num_nodes();
        Self { num_nodes }
    }

    #[cfg(target_os = "linux")]
    fn detect_num_nodes() -> u32 {
        use std::fs;
        let node_dir = "/sys/devices/system/node";
        if let Ok(entries) = fs::read_dir(node_dir) {
            let count = entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .map(|s| s.starts_with("node") && s != "node")
                        .unwrap_or(false)
                })
                .count();
            if count > 0 {
                return count as u32;
            }
        }
        1
    }

    #[cfg(not(target_os = "linux"))]
    fn detect_num_nodes() -> u32 {
        1
    }
}

/// Readahead configuration for sequential access detection.
#[derive(Debug, Clone)]
pub struct ReadaheadPolicy {
    /// Whether readahead prefetching is enabled.
    pub enabled: bool,
    /// How many pages ahead to prefetch when a sequential pattern is detected.
    pub window: usize,
}

impl Default for ReadaheadPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            window: 4,
        }
    }
}

/// Configuration for the buffer manager.
#[derive(Debug, Clone)]
pub struct BufferManagerConfig {
    /// Maximum memory to use for the buffer pool (in bytes).
    pub max_memory: u64,
    /// Page size in bytes.
    pub page_size: usize,
    /// Use memory-mapped I/O instead of read syscalls when available.
    pub use_mmap: bool,
    /// Enable NUMA-aware frame tracking.
    pub numa_aware: bool,
    /// Sequential readahead policy.
    pub readahead: ReadaheadPolicy,
}

impl Default for BufferManagerConfig {
    fn default() -> Self {
        Self {
            max_memory: 64 * 1024 * 1024, // 64MB default
            page_size: DEFAULT_PAGE_SIZE,
            use_mmap: false,
            numa_aware: false,
            readahead: ReadaheadPolicy::default(),
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
#[derive(Debug)]
#[allow(dead_code)]
pub struct BufferManager {
    /// Path to the database directory.
    db_path: PathBuf,
    /// Page size in bytes.
    page_size: usize,
    /// Maximum number of frames allowed.
    max_frames: usize,
    /// Frame table: (file_name, page_num) → Frame.
    /// Composite key so that different files don't collide on page 0.
    frames: HashMap<(String, PageNum), Frame>,
    /// File handles for each database file (path → FileHandle data).
    files: HashMap<String, FileHandleInfo>,
    /// Clock hand index for eviction.
    clock_hand: usize,
    /// Ordered list of (file_name, page_num) for Clock algorithm.
    clock_order: Vec<(String, PageNum)>,
    /// Memory manager for tracking allocation.
    memory_manager: Arc<MemoryManager>,
    /// Statistics.
    stats: BufferManagerStats,
    /// Whether mmap reads are enabled.
    use_mmap: bool,
    /// Memory-mapped regions keyed by file path.
    #[cfg(not(target_arch = "wasm32"))]
    mmap_regions: HashMap<String, memmap2::Mmap>,
    /// NUMA topology info.
    numa_info: NumaInfo,
    /// Sequential readahead policy.
    readahead: ReadaheadPolicy,
    /// Track last page accessed per file for sequential detection.
    last_accessed: HashMap<String, PageNum>,
    /// Track the previous last-accessed page per file (for sequential pattern check).
    prev_last_accessed: HashMap<String, PageNum>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct FileHandleInfo {
    path: PathBuf,
    num_pages: u64,
}

impl BufferManager {
    pub fn new(db_path: PathBuf, memory_manager: Arc<MemoryManager>, config: BufferManagerConfig) -> Self {
        let max_frames = if config.max_memory > 0 {
            (config.max_memory / config.page_size as u64) as usize
        } else {
            1000
        };

        let numa_info = if config.numa_aware {
            NumaInfo::detect()
        } else {
            NumaInfo { num_nodes: 1 }
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
            use_mmap: config.use_mmap,
            #[cfg(not(target_arch = "wasm32"))]
            mmap_regions: HashMap::new(),
            numa_info,
            readahead: config.readahead,
            last_accessed: HashMap::new(),
            prev_last_accessed: HashMap::new(),
        }
    }

    /// Build a composite key for the frames map.
    fn key(file_name: &str, page_num: PageNum) -> (String, PageNum) {
        (file_name.to_string(), page_num)
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

    /// Check if a file is already registered with the buffer manager.
    pub fn is_file_registered(&self, name: &str) -> bool {
        self.files.contains_key(name)
    }

    /// Register a database file with the buffer manager.
    pub fn register_file(&mut self, name: &str, path: PathBuf) {
        let num_pages = if path.exists() {
            let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            len / self.page_size as u64
        } else {
            0
        };
        self.files.insert(name.to_string(), FileHandleInfo { path, num_pages });
    }

    /// Pin a page: bring it into the buffer pool if not already present.
    pub fn pin(&mut self, file_name: &str, page_num: PageNum) -> std::io::Result<Frame> {
        let k = Self::key(file_name, page_num);
        if let Some(frame) = self.frames.get_mut(&k) {
            frame.pin();
            frame.clock_ref = true;

            // Update sequential tracking (cache hit path)
            let prev = self.last_accessed.remove(file_name);
            if let Some(p) = prev {
                self.prev_last_accessed.insert(file_name.to_string(), p);
            }
            self.last_accessed.insert(file_name.to_string(), page_num);

            return Ok(frame.clone());
        }

        // Page fault
        self.stats.page_faults += 1;

        if self.frames.len() >= self.max_frames {
            self.evict()?;
        }

        let data = self.read_from_disk(file_name, page_num)?;
        let mut frame = Frame::new(page_num, data);
        frame.pin();

        if self.numa_info.num_nodes > 1 {
            frame.numa_node = self.current_numa_node();
        }

        let k = Self::key(file_name, page_num);
        self.clock_order.push(k.clone());
        self.frames.insert(k.clone(), frame.clone());
        self.memory_manager.allocate(self.page_size as u64);

        self.update_stats();

        // Update sequential tracking (fault path)
        let prev = self.last_accessed.remove(file_name);
        if let Some(p) = prev {
            self.prev_last_accessed.insert(file_name.to_string(), p);
        }
        self.last_accessed.insert(file_name.to_string(), page_num);

        // Check readahead AFTER updating tracking so prev_last_accessed is correct
        self.maybe_readahead(file_name, page_num);

        Ok(frame)
    }

    /// Pin a page and get mutable access.
    pub fn pin_mut(&mut self, file_name: &str, page_num: PageNum) -> std::io::Result<&mut Frame> {
        let k = Self::key(file_name, page_num);
        if !self.frames.contains_key(&k) {
            // Pin to bring it into the pool (we ignore the returned Frame)
            let _ = self.pin(file_name, page_num)?;
        }
        if let Some(frame) = self.frames.get_mut(&k) {
            frame.pin();
            frame.clock_ref = true;
            return Ok(frame);
        }
        unreachable!()
    }

    /// Unpin a page (requires file_name for composite key lookup).
    pub fn unpin(&mut self, file_name: &str, page_num: PageNum) {
        let k = Self::key(file_name, page_num);
        if let Some(frame) = self.frames.get_mut(&k) {
            frame.unpin();
        }
        self.update_stats();
    }

    /// Flush a dirty page to disk.
    pub fn flush(&mut self, file_name: &str, page_num: PageNum) -> std::io::Result<()> {
        let k = Self::key(file_name, page_num);
        if let Some(frame) = self.frames.get(&k) {
            if frame.is_dirty {
                self.write_to_disk(file_name, page_num, &frame.data)?;
                if let Some(f) = self.frames.get_mut(&k) {
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
        let dirty: Vec<(String, PageNum)> = self
            .frames
            .iter()
            .filter(|(_, f)| f.is_dirty)
            .map(|(k, _)| k.clone())
            .collect();

        for (file_name, page_num) in dirty {
            self.flush(&file_name, page_num)?;
        }
        Ok(())
    }

    /// Return all dirty page numbers belonging to a specific file.
    pub fn dirty_page_nums_for_file(&self, file_name: &str) -> Vec<PageNum> {
        self.frames
            .iter()
            .filter(|(k, f)| k.0 == file_name && f.is_dirty)
            .map(|(k, _)| k.1)
            .collect()
    }

    /// Drop all state for a file: cached frames, registration, mmap region,
    /// and sequential-access tracking. Used when a file is deleted or rebuilt
    /// from scratch (e.g. a full persistence-mirror rewrite) so stale frames
    /// are not re-read under the same file name.
    pub fn drop_file(&mut self, file_name: &str) {
        let path = self.files.get(file_name).map(|f| f.path.clone());
        self.frames.retain(|(f, _), _| f != file_name);
        self.clock_order.retain(|(f, _)| f != file_name);
        self.files.remove(file_name);
        self.last_accessed.remove(file_name);
        self.prev_last_accessed.remove(file_name);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(p) = path {
            let pstr = p.to_string_lossy().to_string();
            self.mmap_regions.remove(&pstr);
        }
        self.update_stats();
    }

    /// Access the NUMA topology info.
    pub fn numa_info(&self) -> &NumaInfo {
        &self.numa_info
    }

    // --- Sequential access tracking & readahead ---

    /// If the access pattern is sequential (page N follows page N-1 for the same
    /// file), prefetch the next `window` pages into the cache (unpinned, just warm).
    fn maybe_readahead(&mut self, file_name: &str, page_num: PageNum) {
        if !self.readahead.enabled || self.readahead.window == 0 {
            return;
        }

        if let Some(&prev) = self.prev_last_accessed.get(file_name) {
            if page_num > 0 && prev == page_num - 1 {
                // Sequential pattern detected — prefetch next `window` pages.
                for offset in 1..=self.readahead.window as u64 {
                    let prefetch_page = page_num + offset;
                    let pk = Self::key(file_name, prefetch_page);
                    if self.frames.contains_key(&pk) {
                        continue; // Already in cache
                    }
                    if self.frames.len() >= self.max_frames {
                        break; // Don't evict to prefetch
                    }
                    if let Ok(data) = self.read_from_disk(file_name, prefetch_page) {
                        self.stats.page_faults += 1;
                        let mut frame = Frame::new(prefetch_page, data);
                        if self.numa_info.num_nodes > 1 {
                            frame.numa_node = self.current_numa_node();
                        }
                        self.clock_order.push(pk.clone());
                        self.frames.insert(pk, frame);
                        self.memory_manager.allocate(self.page_size as u64);
                    }
                }
            }
        }
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
            let (ref file_name, ref page_num) = self.clock_order[self.clock_hand];
            let k = Self::key(file_name, *page_num);

            if let Some(frame) = self.frames.get(&k) {
                if frame.is_pinned() {
                    self.clock_hand += 1;
                    continue;
                }
                if frame.clock_ref {
                    self.frames.get_mut(&k).unwrap().clock_ref = false;
                    self.clock_hand += 1;
                    continue;
                }
                // Victim found
                if frame.is_dirty {
                    self.write_to_disk(file_name, *page_num, &frame.data)?;
                    self.stats.page_writes += 1;
                }
                self.frames.remove(&k);
                self.clock_order.remove(self.clock_hand);
                self.memory_manager.deallocate(self.page_size as u64);
                return Ok(());
            }
        }
        Ok(())
    }

    // --- Disk I/O ---

    /// Read a page from disk, using mmap if enabled.
    fn read_from_disk(&mut self, file_name: &str, page_num: PageNum) -> std::io::Result<Vec<u8>> {
        if self.use_mmap {
            #[cfg(not(target_arch = "wasm32"))]
            {
                return self.read_mmap(file_name, page_num);
            }
            #[cfg(target_arch = "wasm32")]
            {
                return self.read_syscall(file_name, page_num);
            }
        }
        self.read_syscall(file_name, page_num)
    }

    /// Read a page via standard syscalls.
    fn read_syscall(&self, file_name: &str, page_num: PageNum) -> std::io::Result<Vec<u8>> {
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

    /// Read a page via memory-mapped I/O (zero-copy).
    #[cfg(not(target_arch = "wasm32"))]
    fn read_mmap(&mut self, file_name: &str, page_num: PageNum) -> std::io::Result<Vec<u8>> {
        if let Some(fh) = self.files.get(file_name) {
            let path_str = fh.path.to_string_lossy().to_string();
            let page_size = self.page_size;

            if !self.mmap_regions.contains_key(&path_str) {
                let file = std::fs::File::open(&fh.path)?;
                let mmap = unsafe { memmap2::Mmap::map(&file)? };
                self.mmap_regions.insert(path_str.clone(), mmap);
            }

            let mmap = &self.mmap_regions[&path_str];
            let offset = page_num as usize * page_size;
            let end = offset + page_size;

            if end > mmap.len() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    format!(
                        "mmap read out of bounds: page {} (offset {offset}, end {end}) exceeds file length {}",
                        page_num,
                        mmap.len()
                    ),
                ));
            }

            // SAFETY: We checked that offset..end is within bounds of the mmap region.
            // The mmap region is valid for the lifetime of `self`, and we copy the
            // data into a Vec so there is no dangling pointer risk.
            let slice = unsafe { std::slice::from_raw_parts(mmap.as_ptr().add(offset), page_size) };
            Ok(slice.to_vec())
        } else {
            Ok(vec![0u8; self.page_size])
        }
    }

    fn write_to_disk(&self, file_name: &str, page_num: PageNum, data: &[u8]) -> std::io::Result<()> {
        if let Some(fh) = self.files.get(file_name) {
            use std::fs::OpenOptions;
            use std::io::{Seek, SeekFrom, Write};
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(&fh.path)?;
            file.seek(SeekFrom::Start(page_num * self.page_size as u64))?;
            file.write_all(data)?;
        }
        Ok(())
    }

    /// Return the current NUMA node for the calling thread.
    fn current_numa_node(&self) -> u32 {
        // Simple heuristic: always return node 0.
        // A real implementation would call getcpu() or use libnuma.
        0
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

    const TEST_FILE: &str = "main";

    fn create_test_bm() -> (BufferManager, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mm = Arc::new(MemoryManager::new(1024 * 1024));
        let config = BufferManagerConfig {
            max_memory: 256 * 1024,
            page_size: DEFAULT_PAGE_SIZE,
            readahead: ReadaheadPolicy {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bm = BufferManager::new(dir.path().to_path_buf(), mm, config);
        let db_path = dir.path().join("test.db");
        std::fs::write(&db_path, vec![0u8; 8192 * 10]).unwrap();
        bm.register_file(TEST_FILE, db_path);
        (bm, dir)
    }

    #[test]
    fn test_pin_unpin() {
        let (mut bm, _dir) = create_test_bm();
        let frame = bm.pin(TEST_FILE, 0).unwrap();
        assert_eq!(frame.page_num, 0);
        assert!(frame.is_pinned());
        bm.unpin(TEST_FILE, 0);
        assert_eq!(bm.stats().page_faults, 1);
    }

    #[test]
    fn test_cached_page_no_fault() {
        let (mut bm, _dir) = create_test_bm();
        bm.pin(TEST_FILE, 0).unwrap();
        bm.unpin(TEST_FILE, 0);
        bm.pin(TEST_FILE, 0).unwrap();
        bm.unpin(TEST_FILE, 0);
        assert_eq!(bm.stats().page_faults, 1);
    }

    #[test]
    fn test_multiple_pages() {
        let (mut bm, _dir) = create_test_bm();
        for i in 0..5 {
            bm.pin(TEST_FILE, i).unwrap();
            bm.unpin(TEST_FILE, i);
        }
        assert_eq!(bm.stats().page_faults, 5);
        assert_eq!(bm.num_frames(), 5);
    }

    #[test]
    fn test_dirty_and_flush() {
        let (mut bm, _dir) = create_test_bm();
        let frame = bm.pin_mut(TEST_FILE, 1).unwrap();
        frame.mark_dirty();
        bm.unpin(TEST_FILE, 1);
        assert_eq!(bm.stats().dirty_frames, 1);
        bm.flush(TEST_FILE, 1).unwrap();
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
            ..Default::default()
        };
        let mut bm = BufferManager::new(dir.path().to_path_buf(), mm, config);
        let db_path = dir.path().join("test.db");
        std::fs::write(&db_path, vec![0u8; 8192 * 20]).unwrap();
        bm.register_file(TEST_FILE, db_path);

        for i in 0..3 {
            bm.pin(TEST_FILE, i).unwrap();
            bm.unpin(TEST_FILE, i);
        }
        assert_eq!(bm.num_frames(), 3);

        bm.pin(TEST_FILE, 3).unwrap();
        bm.unpin(TEST_FILE, 3);
        assert_eq!(bm.num_frames(), 3);
    }

    #[test]
    fn test_flush_all() {
        let (mut bm, _dir) = create_test_bm();
        for i in 0..3 {
            let frame = bm.pin_mut(TEST_FILE, i).unwrap();
            frame.mark_dirty();
            bm.unpin(TEST_FILE, i);
        }
        bm.flush_all().unwrap();
        assert_eq!(bm.stats().dirty_frames, 0);
    }

    // --- New tests for the three features ---

    #[test]
    fn test_mmap_read() {
        let dir = tempfile::tempdir().unwrap();
        let mm = Arc::new(MemoryManager::new(1024 * 1024));
        let config = BufferManagerConfig {
            max_memory: 256 * 1024,
            page_size: DEFAULT_PAGE_SIZE,
            use_mmap: true,
            ..Default::default()
        };
        let mut bm = BufferManager::new(dir.path().to_path_buf(), mm, config);

        // Write known data
        let db_path = dir.path().join("mmap_test.db");
        let mut data = vec![0u8; DEFAULT_PAGE_SIZE * 4];
        for i in 0..DEFAULT_PAGE_SIZE {
            data[i] = (i % 256) as u8;
        }
        std::fs::write(&db_path, &data).unwrap();
        bm.register_file(TEST_FILE, db_path);

        // Read page 0 via mmap
        let frame = bm.pin(TEST_FILE, 0).unwrap();
        assert_eq!(&frame.data[..], &data[..DEFAULT_PAGE_SIZE]);
        bm.unpin(TEST_FILE, 0);

        // Read page 1 via mmap
        let frame = bm.pin(TEST_FILE, 1).unwrap();
        assert_eq!(&frame.data[..], &data[DEFAULT_PAGE_SIZE..DEFAULT_PAGE_SIZE * 2]);
        bm.unpin(TEST_FILE, 1);
    }

    #[test]
    fn test_readahead_sequential() {
        let dir = tempfile::tempdir().unwrap();
        let mm = Arc::new(MemoryManager::new(1024 * 1024));
        let config = BufferManagerConfig {
            max_memory: 512 * 1024,
            page_size: DEFAULT_PAGE_SIZE,
            readahead: ReadaheadPolicy {
                enabled: true,
                window: 4,
            },
            ..Default::default()
        };
        let mut bm = BufferManager::new(dir.path().to_path_buf(), mm, config);

        let db_path = dir.path().join("seq_test.db");
        std::fs::write(&db_path, vec![42u8; DEFAULT_PAGE_SIZE * 20]).unwrap();
        bm.register_file(TEST_FILE, db_path);

        // Access page 0 — first access, no readahead
        bm.pin(TEST_FILE, 0).unwrap();
        bm.unpin(TEST_FILE, 0);
        let faults_after_0 = bm.stats().page_faults;

        // Access page 1 — sequential from 0, should prefetch pages 2..5
        bm.pin(TEST_FILE, 1).unwrap();
        bm.unpin(TEST_FILE, 1);
        let faults_after_1 = bm.stats().page_faults;

        // readahead of window=4 means pages 2,3,4,5 prefetched: total >= 6 faults
        assert!(
            faults_after_1 > faults_after_0 + 1,
            "Expected readahead faults: after_page_0={}, after_page_1={}",
            faults_after_0,
            faults_after_1
        );

        // Pages 2-5 should now be in cache
        for p in 2..=5 {
            let k = BufferManager::key(TEST_FILE, p);
            assert!(
                bm.frames.contains_key(&k),
                "Page {} should be in cache after readahead",
                p
            );
        }

        // Pinning page 2 again should NOT cause a new fault
        let faults_before = bm.stats().page_faults;
        bm.pin(TEST_FILE, 2).unwrap();
        bm.unpin(TEST_FILE, 2);
        assert_eq!(bm.stats().page_faults, faults_before);
    }

    #[test]
    fn test_readahead_random() {
        let dir = tempfile::tempdir().unwrap();
        let mm = Arc::new(MemoryManager::new(1024 * 1024));
        let config = BufferManagerConfig {
            max_memory: 512 * 1024,
            page_size: DEFAULT_PAGE_SIZE,
            readahead: ReadaheadPolicy {
                enabled: true,
                window: 4,
            },
            ..Default::default()
        };
        let mut bm = BufferManager::new(dir.path().to_path_buf(), mm, config);

        let db_path = dir.path().join("rand_test.db");
        std::fs::write(&db_path, vec![0u8; DEFAULT_PAGE_SIZE * 20]).unwrap();
        bm.register_file(TEST_FILE, db_path);

        // Access page 0
        bm.pin(TEST_FILE, 0).unwrap();
        bm.unpin(TEST_FILE, 0);

        // Access page 5 (random — NOT sequential from 0)
        bm.pin(TEST_FILE, 5).unwrap();
        bm.unpin(TEST_FILE, 5);
        let faults = bm.stats().page_faults;
        assert_eq!(faults, 2, "Random access should not trigger readahead");

        // Pages 6,7,8,9 should NOT be in cache
        for p in 6..=9 {
            let k = BufferManager::key(TEST_FILE, p);
            assert!(
                !bm.frames.contains_key(&k),
                "Page {} should NOT be in cache after random access",
                p
            );
        }
    }

    #[test]
    fn test_numa_detection() {
        let numa = NumaInfo::detect();
        assert!(
            numa.num_nodes >= 1,
            "NUMA detection should return at least 1 node, got {}",
            numa.num_nodes
        );
    }
}
