//! Memory accounting classes and attribution.
//!
//! The buffer manager and index structures allocate/deallocate against
//! [`crate::memory::MemoryManager`]. Attribution lets the same manager track
//! *why* memory is used (which subsystem/class), so the effective spill
//! threshold can be derived from live usage instead of a fixed constant.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Accounting class = which subsystem caused the allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryAccountingClass {
    /// Buffer-manager page cache frames.
    BufferPool,
    /// Vector (HNSW) indexes.
    VectorIndex,
    /// Full-text indexes.
    FtsIndex,
    /// Graph adjacency structures.
    Graph,
    /// Compiled execution plans.
    CompiledPlan,
    /// Anything not covered above.
    Other,
}

impl MemoryAccountingClass {
    /// Stable machine-readable label.
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryAccountingClass::BufferPool => "buffer_pool",
            MemoryAccountingClass::VectorIndex => "vector_index",
            MemoryAccountingClass::FtsIndex => "fts_index",
            MemoryAccountingClass::Graph => "graph",
            MemoryAccountingClass::CompiledPlan => "compiled_plan",
            MemoryAccountingClass::Other => "other",
        }
    }

    /// Stable 64-bit discriminator (for Cheap-to-store accounting).
    pub fn discriminant(self) -> u8 {
        match self {
            MemoryAccountingClass::BufferPool => 0,
            MemoryAccountingClass::VectorIndex => 1,
            MemoryAccountingClass::FtsIndex => 2,
            MemoryAccountingClass::Graph => 3,
            MemoryAccountingClass::CompiledPlan => 4,
            MemoryAccountingClass::Other => 5,
        }
    }
}

impl From<u8> for MemoryAccountingClass {
    fn from(v: u8) -> Self {
        match v {
            0 => MemoryAccountingClass::BufferPool,
            1 => MemoryAccountingClass::VectorIndex,
            2 => MemoryAccountingClass::FtsIndex,
            3 => MemoryAccountingClass::Graph,
            4 => MemoryAccountingClass::CompiledPlan,
            _ => MemoryAccountingClass::Other,
        }
    }
}

/// Attribution for one allocation: which subsystem/domain and accounting class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryAttribution {
    /// Free-form subsystem/domain name, e.g. `"buffer_pool"` or `"catalog"`.
    pub domain: &'static str,
    /// Accounting class for bucketed reporting.
    pub class: MemoryAccountingClass,
}

/// Canonical attribution for buffer-pool page frames.
pub const BUFFER_POOL: MemoryAttribution = MemoryAttribution {
    domain: "buffer_pool",
    class: MemoryAccountingClass::BufferPool,
};

/// Canonical attribution for vector (HNSW/ANN) indexes.
pub const VECTOR_INDEX: MemoryAttribution = MemoryAttribution {
    domain: "vector_index",
    class: MemoryAccountingClass::VectorIndex,
};

/// Canonical attribution for full-text indexes.
pub const FTS_INDEX: MemoryAttribution = MemoryAttribution {
    domain: "fts_index",
    class: MemoryAccountingClass::FtsIndex,
};

/// Per-class memory usage bookkeeping.
///
/// O(1) allocate/deallocate and cheap snapshots; safe to share via `Mutex`
/// since these sit behind the [`crate::memory::MemoryManager`] handle.
#[derive(Debug, Default)]
pub struct MemoryAccountant {
    total: AtomicU64,
    by_class: Mutex<HashMap<MemoryAccountingClass, u64>>,
}

impl MemoryAccountant {
    /// Create an empty accountant.
    pub fn new() -> Self {
        Self::default()
    }

    /// Attribute `amount` bytes to `attr`.
    pub fn allocate(&self, attr: MemoryAttribution, amount: u64) {
        self.total.fetch_add(amount, Ordering::Relaxed);
        let mut by_class = self.by_class.lock().unwrap();
        *by_class.entry(attr.class).or_insert(0) += amount;
    }

    /// Release `amount` bytes attributed to `attr` (saturates per-class so an
    /// over-release can never make totals negative).
    pub fn deallocate(&self, attr: MemoryAttribution, amount: u64) {
        self.total
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |prev| {
                Some(prev.saturating_sub(amount))
            })
            .ok();
        let mut by_class = self.by_class.lock().unwrap();
        if let Some(bytes) = by_class.get_mut(&attr.class) {
            *bytes = bytes.saturating_sub(amount);
        }
    }

    /// Total attributed bytes currently live.
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Attributed bytes for one class (0 if never allocated).
    pub fn class_usage(&self, class: MemoryAccountingClass) -> u64 {
        self.by_class.lock().unwrap().get(&class).copied().unwrap_or(0)
    }

    /// Snapshot of `(class, bytes)` for reporting, non-zero classes only.
    pub fn snapshot(&self) -> Vec<(MemoryAccountingClass, u64)> {
        let mut out: Vec<(MemoryAccountingClass, u64)> = self
            .by_class
            .lock()
            .unwrap()
            .iter()
            .map(|(&class, &bytes)| (class, bytes))
            .filter(|(_, bytes)| *bytes > 0)
            .collect();
        out.sort_by_key(|(class, _)| class.discriminant());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocate_deallocate_roundtrip() {
        let acc = MemoryAccountant::new();
        acc.allocate(BUFFER_POOL, 4096);
        acc.allocate(VECTOR_INDEX, 2048);
        assert_eq!(acc.total(), 6144);
        assert_eq!(acc.class_usage(MemoryAccountingClass::BufferPool), 4096);
        acc.deallocate(BUFFER_POOL, 4096);
        assert_eq!(acc.total(), 2048);
        assert_eq!(acc.class_usage(MemoryAccountingClass::BufferPool), 0);
    }

    #[test]
    fn test_deallocate_never_negative() {
        let acc = MemoryAccountant::new();
        acc.allocate(BUFFER_POOL, 100);
        acc.deallocate(BUFFER_POOL, 10_000);
        assert_eq!(acc.class_usage(MemoryAccountingClass::BufferPool), 0);
        assert_eq!(acc.total(), 0);
    }

    #[test]
    fn test_snapshot_sorted_non_zero() {
        let acc = MemoryAccountant::new();
        acc.allocate(FTS_INDEX, 128);
        acc.allocate(BUFFER_POOL, 256);
        let snap = acc.snapshot();
        assert_eq!(
            snap,
            vec![
                (MemoryAccountingClass::BufferPool, 256),
                (MemoryAccountingClass::FtsIndex, 128),
            ]
        );
    }

    #[test]
    fn test_class_discriminants_roundtrip() {
        for class in [
            MemoryAccountingClass::BufferPool,
            MemoryAccountingClass::VectorIndex,
            MemoryAccountingClass::FtsIndex,
            MemoryAccountingClass::Graph,
            MemoryAccountingClass::CompiledPlan,
            MemoryAccountingClass::Other,
        ] {
            assert_eq!(MemoryAccountingClass::from(class.discriminant()), class);
        }
    }
}
