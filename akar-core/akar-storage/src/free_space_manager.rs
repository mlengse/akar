//! Free space manager for tracking reusable disk pages.
//!
//! Tracks free page ranges using a buddy-system-like approach:
//! - Free ranges are stored in sorted lists organized by "level" (power-of-2 size)
//! - `level = log2(numPages)` — e.g., size 2 → level 1, size 5 → level 2
//! - On allocation: find the smallest level with a suitable range, split if needed
//! - On free: add the range back (no merging — simplifies correctness)
//!
//! Ported from C++ `src/include/storage/free_space_manager.h` and
//! `src/storage/free_space_manager.cpp`.

use std::collections::BTreeSet;
use std::sync::{
    RwLock,
    atomic::{AtomicU64, Ordering},
};

/// A contiguous range of free pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRange {
    pub start_page_idx: u64,
    pub num_pages: u64,
}

impl PageRange {
    pub fn new(start_page_idx: u64, num_pages: u64) -> Self {
        Self {
            start_page_idx,
            num_pages,
        }
    }

    /// Create a subrange starting at `new_start` pages into this range.
    pub fn subrange(&self, new_start: u64) -> Self {
        assert!(new_start <= self.num_pages);
        Self {
            start_page_idx: self.start_page_idx + new_start,
            num_pages: self.num_pages - new_start,
        }
    }
}

impl PartialOrd for PageRange {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PageRange {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Sort by num_pages first, then start_page_idx (matches C++ entryCmp)
        match self.num_pages.cmp(&other.num_pages) {
            std::cmp::Ordering::Equal => self.start_page_idx.cmp(&other.start_page_idx),
            other => other,
        }
    }
}

/// Invalid page index constant.
pub const INVALID_PAGE_IDX: u64 = u64::MAX;

/// Tracks free page ranges for efficient allocation.
///
/// Uses multiple `BTreeSet<PageRange>` organized by power-of-2 levels.
/// Level `i` stores ranges whose size is in `[2^i, 2^{i+1})`.
#[derive(Debug)]
pub struct FreeSpaceManager {
    /// One sorted free list per power-of-2 level.
    free_lists: Vec<RwLock<BTreeSet<PageRange>>>,
    /// Uncheckpointed free page ranges (not reusable until next checkpoint finalizes).
    uncheckpointed_free_page_ranges: RwLock<Vec<PageRange>>,
    /// Number of entries across all free lists.
    num_entries: AtomicU64,
}

impl Default for FreeSpaceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl FreeSpaceManager {
    pub fn new() -> Self {
        let mut free_lists = Vec::with_capacity(64);
        for _ in 0..64 {
            free_lists.push(RwLock::new(BTreeSet::new()));
        }
        Self {
            free_lists,
            uncheckpointed_free_page_ranges: RwLock::new(Vec::new()),
            num_entries: AtomicU64::new(0),
        }
    }

    /// Serialize the FreeSpaceManager state to a byte vector.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        // Format: [num_ranges: u32] [range1: start(u64), num(u64)] ...
        let mut ranges = Vec::new();
        for list in &self.free_lists {
            for range in list.read().unwrap().iter() {
                ranges.push(*range);
            }
        }
        buf.extend_from_slice(&(ranges.len() as u32).to_le_bytes());
        for range in ranges {
            buf.extend_from_slice(&range.start_page_idx.to_le_bytes());
            buf.extend_from_slice(&range.num_pages.to_le_bytes());
        }
        buf
    }

    /// Deserialize FreeSpaceManager state from a byte slice.
    pub fn deserialize(data: &[u8]) -> std::io::Result<Self> {
        if data.len() < 4 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Data too short for FSM",
            ));
        }
        let num_ranges = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
        let expected_len = 4 + num_ranges * 16;
        if data.len() < expected_len {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid FSM data length",
            ));
        }
        let fsm = Self::new();
        for i in 0..num_ranges {
            let offset = 4 + i * 16;
            let start_page_idx = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
            let num_pages = u64::from_le_bytes(data[offset + 8..offset + 16].try_into().unwrap());
            fsm.add_free_pages(PageRange::new(start_page_idx, num_pages));
        }
        Ok(fsm)
    }

    /// Compute the power-of-2 level for a given number of pages.
    /// Level = position of highest set bit (floor(log2(numPages))).
    /// e.g., 1 → 0, 2 → 1, 3 → 1, 4 → 2, 5 → 2
    fn get_level(num_pages: u64) -> usize {
        assert!(num_pages > 0);
        // floor(log2(numPages)) = (bits - 1 - leading_zeros)
        (u64::BITS - 1 - num_pages.leading_zeros()) as usize
    }

    /// Get the free list for a given level.
    fn get_free_list(&self, level: usize) -> &RwLock<BTreeSet<PageRange>> {
        assert!(level < 64);
        &self.free_lists[level]
    }

    /// Add a free page range to the manager.
    pub fn add_free_pages(&self, entry: PageRange) {
        assert!(entry.num_pages > 0);
        let level = Self::get_level(entry.num_pages);
        let list_lock = self.get_free_list(level);
        let mut list = list_lock.write().unwrap();
        assert!(!list.contains(&entry), "duplicate free page range");
        list.insert(entry);
        self.num_entries.fetch_add(1, Ordering::SeqCst);
    }

    /// Evict pages from buffer manager then add as free.
    /// Note: Simplified — just adds to free list; actual eviction requires BufferManager access.
    pub fn evict_and_add_free_pages(&self, entry: PageRange) {
        // In a full implementation, this would call BufferManager::evict() for each page.
        // For now, just track the free pages.
        self.add_free_pages(entry);
    }

    /// Pop a free page range of at least `num_pages` from the manager.
    /// Returns `None` if no suitable range is available.
    pub fn pop_free_pages(&self, num_pages: u64) -> Option<PageRange> {
        if num_pages == 0 {
            return None;
        }

        let level_to_search = Self::get_level(num_pages);
        for level in level_to_search..self.free_lists.len() {
            let entry = {
                let list = self.free_lists[level].read().unwrap();
                // Find the first entry with num_pages >= requested
                let probe = PageRange::new(0, num_pages);
                if let Some(found) = list.range(probe..).next() {
                    *found
                } else {
                    continue;
                }
            };

            // Remove the entry
            {
                let mut list = self.free_lists[level].write().unwrap();
                list.remove(&entry);
            }
            self.num_entries.fetch_sub(1, Ordering::SeqCst);

            // Split if the range is larger than needed
            return Some(self.split_page_range(entry, num_pages));
        }

        None
    }

    /// Split a page range, returning the requested prefix and re-adding the remainder.
    fn split_page_range(&self, chunk: PageRange, num_required_pages: u64) -> PageRange {
        assert!(chunk.num_pages >= num_required_pages);
        let result = PageRange::new(chunk.start_page_idx, num_required_pages);
        if num_required_pages < chunk.num_pages {
            let remaining = PageRange::new(
                chunk.start_page_idx + num_required_pages,
                chunk.num_pages - num_required_pages,
            );
            self.add_free_pages(remaining);
        }
        result
    }

    /// Add pages that are freed but not yet reusable until checkpoint.
    pub fn add_uncheckpointed_free_pages(&self, entry: PageRange) {
        self.uncheckpointed_free_page_ranges.write().unwrap().push(entry);
    }

    /// Roll back an incomplete checkpoint — discard uncheckpointed entries.
    pub fn rollback_checkpoint(&self) {
        self.uncheckpointed_free_page_ranges.write().unwrap().clear();
    }

    /// Finalize checkpoint — move uncheckpointed entries into the main free lists.
    pub fn finalize_checkpoint(&self) {
        let entries = std::mem::take(&mut *self.uncheckpointed_free_page_ranges.write().unwrap());
        for entry in entries {
            self.add_free_pages(entry);
        }
    }

    /// Get total number of entries across all free lists.
    pub fn num_entries(&self) -> u64 {
        self.num_entries.load(Ordering::SeqCst)
    }

    /// Get total number of free pages across all lists.
    pub fn total_free_pages(&self) -> u64 {
        self.free_lists
            .iter()
            .flat_map(|list_lock| list_lock.read().unwrap().clone().into_iter())
            .map(|r| r.num_pages)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_level() {
        assert_eq!(FreeSpaceManager::get_level(1), 0);
        assert_eq!(FreeSpaceManager::get_level(2), 1);
        assert_eq!(FreeSpaceManager::get_level(3), 1);
        assert_eq!(FreeSpaceManager::get_level(4), 2);
        assert_eq!(FreeSpaceManager::get_level(8), 3);
        assert_eq!(FreeSpaceManager::get_level(15), 3);
    }

    #[test]
    fn test_add_and_pop_exact() {
        let fsm = FreeSpaceManager::new();
        fsm.add_free_pages(PageRange::new(10, 5));
        assert_eq!(fsm.num_entries(), 1);

        let result = fsm.pop_free_pages(5).unwrap();
        assert_eq!(result.start_page_idx, 10);
        assert_eq!(result.num_pages, 5);
        assert_eq!(fsm.num_entries(), 0);
    }

    #[test]
    fn test_pop_splits_larger_range() {
        let fsm = FreeSpaceManager::new();
        // Add a range of 10 pages starting at index 0
        fsm.add_free_pages(PageRange::new(0, 10));

        // Request 3 pages — should split off 3, leaving 7
        let result = fsm.pop_free_pages(3).unwrap();
        assert_eq!(result.start_page_idx, 0);
        assert_eq!(result.num_pages, 3);

        // The remaining 7 pages should still be in the free list
        assert_eq!(fsm.num_entries(), 1);
        assert_eq!(fsm.total_free_pages(), 7);
    }

    #[test]
    fn test_pop_no_suitable_range() {
        let fsm = FreeSpaceManager::new();
        fsm.add_free_pages(PageRange::new(0, 3));

        // Request more pages than available
        let result = fsm.pop_free_pages(10);
        assert!(result.is_none());
    }

    #[test]
    fn test_pop_from_empty() {
        let fsm = FreeSpaceManager::new();
        assert!(fsm.pop_free_pages(1).is_none());
    }

    #[test]
    fn test_multiple_free_lists() {
        let fsm = FreeSpaceManager::new();
        fsm.add_free_pages(PageRange::new(0, 2)); // level 1
        fsm.add_free_pages(PageRange::new(10, 4)); // level 2
        fsm.add_free_pages(PageRange::new(20, 8)); // level 3

        assert_eq!(fsm.num_entries(), 3);
        assert_eq!(fsm.total_free_pages(), 14);

        // Pop 3 pages — should find from level 2 range (size 4)
        let r1 = fsm.pop_free_pages(3).unwrap();
        assert_eq!(r1.start_page_idx, 10);
        assert_eq!(r1.num_pages, 3);
    }

    #[test]
    fn test_uncheckpointed_pages() {
        let fsm = FreeSpaceManager::new();
        fsm.add_uncheckpointed_free_pages(PageRange::new(100, 5));

        // Not yet available for allocation
        assert_eq!(fsm.num_entries(), 0);
        assert!(fsm.pop_free_pages(5).is_none());

        // After rollback — discarded
        fsm.rollback_checkpoint();
        assert!(fsm.pop_free_pages(5).is_none());

        fsm.add_uncheckpointed_free_pages(PageRange::new(200, 5));
        fsm.finalize_checkpoint();
        assert_eq!(fsm.num_entries(), 1);
        assert!(fsm.pop_free_pages(5).is_some());
    }

    #[test]
    fn test_multiple_pop_from_same_range() {
        let fsm = FreeSpaceManager::new();
        fsm.add_free_pages(PageRange::new(0, 100));

        // Pop multiple times from the same range
        for i in 0..10 {
            let r = fsm.pop_free_pages(10).unwrap();
            assert_eq!(r.start_page_idx, i * 10);
            assert_eq!(r.num_pages, 10);
        }

        // All 100 pages consumed
        assert_eq!(fsm.total_free_pages(), 0);
    }

    #[test]
    fn test_page_range_ordering() {
        let mut set = BTreeSet::new();
        set.insert(PageRange::new(5, 2)); // level 1
        set.insert(PageRange::new(0, 4)); // level 2
        set.insert(PageRange::new(10, 1)); // level 0

        // Order: by num_pages first (1 < 2 < 4)
        let ordered: Vec<PageRange> = set.into_iter().collect();
        assert_eq!(ordered[0].num_pages, 1);
        assert_eq!(ordered[1].num_pages, 2);
        assert_eq!(ordered[2].num_pages, 4);
    }
}
