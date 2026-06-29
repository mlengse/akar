//! Page and Frame types for the buffer manager.

use std::path::PathBuf;

/// Default page size: 8KB.
pub const DEFAULT_PAGE_SIZE: usize = 8192;

/// A database page number (logical identifier).
pub type PageNum = u64;

/// A frame in the buffer pool holding a cached page.
#[derive(Debug, Clone)]
pub struct Frame {
    /// The page number this frame holds.
    pub page_num: PageNum,
    /// Reference count (number of pins).
    pub pin_count: u32,
    /// Whether the page has been modified since loading.
    pub is_dirty: bool,
    /// Clock hand flag for Clock eviction policy.
    pub clock_ref: bool,
    /// The actual page data.
    pub data: Vec<u8>,
}

impl Frame {
    pub fn new(page_num: PageNum, data: Vec<u8>) -> Self {
        Self {
            page_num,
            pin_count: 0,
            is_dirty: false,
            clock_ref: true,
            data,
        }
    }

    /// Pin the frame (increment reference count).
    pub fn pin(&mut self) {
        self.pin_count += 1;
    }

    /// Unpin the frame (decrement reference count).
    pub fn unpin(&mut self) {
        if self.pin_count > 0 {
            self.pin_count -= 1;
        }
    }

    /// Check if the frame is pinned (in use).
    pub fn is_pinned(&self) -> bool {
        self.pin_count > 0
    }

    /// Mark frame as dirty (modified).
    pub fn mark_dirty(&mut self) {
        self.is_dirty = true;
    }
}

/// A file handle mapping logical page numbers to file offsets.
#[derive(Debug, Clone)]
pub struct FileHandle {
    /// Path to the database file.
    pub path: PathBuf,
    /// Number of pages currently in the file.
    pub num_pages: u64,
    /// Page size in bytes.
    pub page_size: usize,
}

impl FileHandle {
    pub fn new(path: PathBuf, page_size: usize) -> Self {
        let num_pages = if path.exists() {
            let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            len / page_size as u64
        } else {
            0
        };
        Self {
            path,
            num_pages,
            page_size,
        }
    }

    /// Get the file offset for a given page number.
    pub fn page_offset(&self, page_num: PageNum) -> u64 {
        page_num * self.page_size as u64
    }

    /// Read a page from disk.
    pub fn read_page(&self, page_num: PageNum) -> std::io::Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = std::fs::File::open(&self.path)?;
        let offset = self.page_offset(page_num);
        file.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; self.page_size];
        file.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// Write a page to disk.
    pub fn write_page(&self, page_num: PageNum, data: &[u8]) -> std::io::Result<()> {
        use std::io::{Seek, SeekFrom, Write};
        let mut file = std::fs::File::create(&self.path)?;
        let offset = self.page_offset(page_num);
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(data)?;
        Ok(())
    }

    /// Allocate a new page (extend the file).
    pub fn allocate_page(&mut self) -> PageNum {
        let page_num = self.num_pages;
        self.num_pages += 1;
        page_num
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_pin_unpin() {
        let mut frame = Frame::new(0, vec![0u8; DEFAULT_PAGE_SIZE]);
        assert!(!frame.is_pinned());
        assert_eq!(frame.pin_count, 0);

        frame.pin();
        assert!(frame.is_pinned());
        assert_eq!(frame.pin_count, 1);

        frame.unpin();
        assert!(!frame.is_pinned());
    }

    #[test]
    fn test_frame_dirty() {
        let mut frame = Frame::new(1, vec![0u8; DEFAULT_PAGE_SIZE]);
        assert!(!frame.is_dirty);
        frame.mark_dirty();
        assert!(frame.is_dirty);
    }

    #[test]
    fn test_page_offset() {
        let fh = FileHandle::new(PathBuf::from("test.db"), DEFAULT_PAGE_SIZE);
        assert_eq!(fh.page_offset(0), 0);
        assert_eq!(fh.page_offset(1), 8192);
        assert_eq!(fh.page_offset(5), 40960);
    }
}
