//! Memory management utilities for the buffer manager.
//!
//! Tracks allocated memory and provides backpressure hints.

use crate::memory_account::{MemoryAccountant, MemoryAccountingClass, MemoryAttribution};
use std::sync::atomic::{AtomicU64, Ordering};

/// A memory tracker for the database instance.
///
/// In addition to a flat `total_allocated` counter, the manager routes every
/// allocation through a [`MemoryAccountant`] so the same budget can be broken
/// down by subsystem (buffer pool, indexes, graphs) and used to derive an
/// *effective* spill threshold for the memory governor.
#[derive(Debug)]
pub struct MemoryManager {
    total_allocated: AtomicU64,
    max_memory: u64,
    accountant: MemoryAccountant,
}

impl MemoryManager {
    pub fn new(max_memory: u64) -> Self {
        Self {
            total_allocated: AtomicU64::new(0),
            max_memory,
            accountant: MemoryAccountant::new(),
        }
    }

    pub fn max_memory(&self) -> u64 {
        self.max_memory
    }

    pub fn total_allocated(&self) -> u64 {
        self.total_allocated.load(Ordering::Relaxed)
    }

    /// Allocate `amount` bytes attributed to [`MemoryAccountingClass::Other`].
    ///
    /// Kept for callers that do not care about attribution; prefer
    /// [`MemoryManager::allocate_with`].
    pub fn allocate(&self, amount: u64) {
        self.allocate_with(
            MemoryAttribution {
                domain: "other",
                class: MemoryAccountingClass::Other,
            },
            amount,
        );
    }

    /// Allocate `amount` bytes attributed to `attr` (accountable allocation).
    pub fn allocate_with(&self, attr: MemoryAttribution, amount: u64) {
        self.total_allocated.fetch_add(amount, Ordering::Relaxed);
        self.accountant.allocate(attr, amount);
    }

    /// Release `amount` bytes (unattributed). Prefer
    /// [`MemoryManager::deallocate_with`] to keep per-class books balanced.
    pub fn deallocate(&self, amount: u64) {
        self.deallocate_with(
            MemoryAttribution {
                domain: "other",
                class: MemoryAccountingClass::Other,
            },
            amount,
        );
    }

    /// Release `amount` bytes previously attributed to `attr`.
    pub fn deallocate_with(&self, attr: MemoryAttribution, amount: u64) {
        self.total_allocated.fetch_sub(amount, Ordering::Relaxed);
        self.accountant.deallocate(attr, amount);
    }

    pub fn is_under_limit(&self) -> bool {
        self.total_allocated() <= self.max_memory
    }

    /// The accountant backing this manager (live totals break down by class).
    pub fn accountant(&self) -> &MemoryAccountant {
        &self.accountant
    }

    // -------------------------------------------------------------------
    // Memory-governor hooks (effective spill threshold & pressure)
    // -------------------------------------------------------------------

    /// Headroom left before the configured budget is exhausted.
    ///
    /// Intended to be read by the spiller when deciding whether a NodeGroup /
    /// frame batch crosses the spill threshold: "how much can I still grow?".
    pub fn effective_spill_threshold(&self) -> u64 {
        self.max_memory.saturating_sub(self.total_allocated())
    }

    /// Fraction of the configured budget currently allocated, in `0.0..=1.0`.
    pub fn memory_pressure(&self) -> f64 {
        if self.max_memory == 0 {
            return 0.0;
        }
        (self.total_allocated() as f64 / self.max_memory as f64).clamp(0.0, 1.0)
    }

    /// True when allocated memory consumes at least `ratio` of the budget.
    pub fn is_under_memory_pressure(&self, ratio: f64) -> bool {
        self.memory_pressure() >= ratio
    }
}

impl Default for MemoryManager {
    fn default() -> Self {
        // Default to 80% of available system memory (approximate).
        Self::new(u64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_account::{BUFFER_POOL, MemoryAccountingClass};

    #[test]
    fn test_allocate_with_is_accounted() {
        let mm = MemoryManager::new(1024);
        mm.allocate_with(BUFFER_POOL, 100);
        assert_eq!(mm.total_allocated(), 100);
        assert_eq!(mm.accountant().class_usage(MemoryAccountingClass::BufferPool), 100);
        mm.deallocate_with(BUFFER_POOL, 100);
        assert_eq!(mm.total_allocated(), 0);
    }

    #[test]
    fn test_plain_allocate_uses_other_class() {
        let mm = MemoryManager::new(1024);
        mm.allocate(64);
        assert_eq!(mm.accountant().class_usage(MemoryAccountingClass::Other), 64);
    }

    #[test]
    fn test_effective_spill_threshold() {
        let mm = MemoryManager::new(1000);
        assert_eq!(mm.effective_spill_threshold(), 1000);
        mm.allocate_with(BUFFER_POOL, 300);
        assert_eq!(mm.effective_spill_threshold(), 700);
        // Saturates rather than going negative.
        mm.allocate_with(BUFFER_POOL, 10_000);
        assert_eq!(mm.effective_spill_threshold(), 0);
    }

    #[test]
    fn test_memory_pressure() {
        let mm = MemoryManager::new(100);
        assert!((mm.memory_pressure() - 0.0).abs() < 1e-9);
        assert!(!mm.is_under_memory_pressure(0.8));
        mm.allocate_with(BUFFER_POOL, 90);
        assert!((mm.memory_pressure() - 0.9).abs() < 1e-9);
        assert!(mm.is_under_memory_pressure(0.8));
    }
}
