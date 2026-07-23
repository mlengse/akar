//! Memory management utilities for the buffer manager.
//!
//! Tracks allocated memory and provides backpressure hints.

use std::sync::atomic::{AtomicU64, Ordering};

/// A simple memory tracker for the database instance.
#[derive(Debug)]
pub struct MemoryManager {
    total_allocated: AtomicU64,
    max_memory: u64,
}

impl MemoryManager {
    pub fn new(max_memory: u64) -> Self {
        Self {
            total_allocated: AtomicU64::new(0),
            max_memory,
        }
    }

    pub fn max_memory(&self) -> u64 {
        self.max_memory
    }

    pub fn total_allocated(&self) -> u64 {
        self.total_allocated.load(Ordering::Relaxed)
    }

    pub fn allocate(&self, amount: u64) {
        self.total_allocated.fetch_add(amount, Ordering::Relaxed);
    }

    pub fn deallocate(&self, amount: u64) {
        self.total_allocated.fetch_sub(amount, Ordering::Relaxed);
    }

    pub fn is_under_limit(&self) -> bool {
        self.total_allocated() <= self.max_memory
    }
}

impl Default for MemoryManager {
    fn default() -> Self {
        // Default to 80% of available system memory (approximate).
        Self::new(u64::MAX)
    }
}
