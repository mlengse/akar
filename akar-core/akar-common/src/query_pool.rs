//! Per-query memory pools and grants — the memory governor foundation.
//!
//! A [`QueryMemoryPool`] bounds how much memory a single query may hold before
//! its operators must spill work to disk. Grants are derived from the global
//! [`crate::memory::MemoryManager`] headroom so concurrent queries *share* the
//! budget fairly instead of each assuming it owns the whole instance.
//!
//! This is the P110.1 primitive consumed by the admission gate (P110.2), the
//! active reclaim path (P110.3), and the external spill hash join (P111):
//!
//! - [`MemoryGovernor`] — hands out per-query pools whose grant is a fair share
//!   of [`MemoryManager::effective_spill_threshold`], tracking live query count
//!   so later grants shrink under concurrency.
//! - [`QueryMemoryPool`] — per-query grant enforcement. Operators reserve bytes
//!   before growing in-memory structures; an exhausted grant signals "spill
//!   now" instead of allowing unbounded growth (OOM).

use crate::memory::MemoryManager;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Outcome of reserving bytes against a [`QueryMemoryPool`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    /// The requested bytes fit within the pool's remaining grant; the caller
    /// may grow its in-memory structure by `bytes`.
    Granted,
    /// The pool has no room for the request; the caller must spill the data it
    /// wanted to keep in memory (freeing previously reserved bytes) before it
    /// can grow again.
    Exhausted,
}

/// A per-query memory pool enforcing a grant derived from the global budget.
///
/// Thread-safe: operators reserve/release via atomics, so a query's build and
/// probe stages can share one pool across threads.
#[derive(Debug)]
pub struct QueryMemoryPool {
    /// Process-wide unique query identifier (used for `join_<qid>_*.bin`
    /// spill files and EXPLAIN `Spill=N` markers).
    query_id: u64,
    /// Bytes this query may hold in memory before operators must spill.
    grant_bytes: u64,
    /// Bytes currently reserved by the query's operators.
    reserved_bytes: AtomicU64,
    /// Peak reservation observed (for stats/EXPLAIN).
    peak_bytes: AtomicU64,
    /// Number of spill events recorded by operators (EXPLAIN `Spill=N`).
    spill_events: AtomicU64,
    /// Back-reference to the governor that granted this pool; when present,
    /// dropping the pool releases the governor's active-query slot.
    governor: Option<Arc<GovernorShared>>,
}

impl QueryMemoryPool {
    /// Create a standalone pool with a fixed grant (no governor tracking).
    pub fn new(query_id: u64, grant_bytes: u64) -> Self {
        Self {
            query_id,
            grant_bytes,
            reserved_bytes: AtomicU64::new(0),
            peak_bytes: AtomicU64::new(0),
            spill_events: AtomicU64::new(0),
            governor: None,
        }
    }

    /// The pool's unique query identifier.
    pub fn query_id(&self) -> u64 {
        self.query_id
    }

    /// Bytes granted to this query (its in-memory ceiling before spilling).
    pub fn grant_bytes(&self) -> u64 {
        self.grant_bytes
    }

    /// Replace the grant (grow or shrink). Shrinking below current reserved
    /// saturates `remaining()` to 0, which immediately signals "spill" to
    /// reserving operators — the mechanism used by active reclaim (P110.3).
    pub fn set_grant(&mut self, grant_bytes: u64) {
        self.grant_bytes = grant_bytes;
    }

    /// Bytes currently reserved by the query's operators.
    pub fn reserved(&self) -> u64 {
        self.reserved_bytes.load(Ordering::Relaxed)
    }

    /// Headroom left before the grant is exhausted (saturates at 0).
    pub fn remaining(&self) -> u64 {
        self.grant_bytes.saturating_sub(self.reserved())
    }

    /// Peak reservation observed since the pool was created.
    pub fn peak(&self) -> u64 {
        self.peak_bytes.load(Ordering::Relaxed)
    }

    /// Number of spill events recorded via [`QueryMemoryPool::note_spill`].
    pub fn spill_events(&self) -> u64 {
        self.spill_events.load(Ordering::Relaxed)
    }

    /// Record one spill event (increments the EXPLAIN `Spill=N` counter).
    pub fn note_spill(&self) {
        self.spill_events.fetch_add(1, Ordering::Relaxed);
    }

    /// Whether `bytes` would fit within the remaining grant right now.
    pub fn can_reserve(&self, bytes: u64) -> bool {
        bytes <= self.remaining()
    }

    /// Reserve `bytes` against the grant.
    ///
    /// Returns [`Grant::Granted`] and increases the pool's reserved/peak
    /// counters when the request fits; otherwise returns
    /// [`Grant::Exhausted`] and leaves the pool untouched — the caller must
    /// spill instead of growing (P111).
    pub fn try_reserve(&self, bytes: u64) -> Grant {
        if !self.can_reserve(bytes) {
            return Grant::Exhausted;
        }
        let reserved = self.reserved_bytes.fetch_add(bytes, Ordering::Relaxed) + bytes;
        self.peak_bytes.fetch_max(reserved, Ordering::Relaxed);
        Grant::Granted
    }

    /// Release `bytes` back to the grant (saturates so over-release can never
    /// make reservations negative).
    pub fn release(&self, bytes: u64) {
        self.reserved_bytes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |prev| {
                Some(prev.saturating_sub(bytes))
            })
            .ok();
    }

    /// Fraction of the grant currently reserved, in `0.0..=1.0`.
    pub fn pressure(&self) -> f64 {
        if self.grant_bytes == 0 {
            return 1.0;
        }
        (self.reserved() as f64 / self.grant_bytes as f64).clamp(0.0, 1.0)
    }

    /// True when reserved memory consumes at least `ratio` of the grant.
    pub fn is_under_pressure(&self, ratio: f64) -> bool {
        self.pressure() >= ratio
    }
}

impl Drop for QueryMemoryPool {
    fn drop(&mut self) {
        if let Some(governor) = &self.governor {
            governor.active_queries.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

/// Shared governor state referenced by every granted pool.
#[derive(Debug)]
struct GovernorShared {
    /// Global budget tracker grants are derived from.
    memory: Arc<MemoryManager>,
    /// Queries currently holding a live pool (sizes the fair share).
    active_queries: AtomicU64,
    /// Monotonic pool/query-id allocator.
    next_query_id: AtomicU64,
}

impl GovernorShared {
    /// Fair share of the current headroom for one of `active` queries.
    fn fair_share(&self) -> u64 {
        let active = self.active_queries.load(Ordering::Relaxed).max(1);
        self.memory.effective_spill_threshold() / active
    }
}

/// Distributes per-query memory grants from the global [`MemoryManager`].
///
/// Every [`MemoryGovernor::grant_new_query`] call hands out a
/// [`QueryMemoryPool`] whose grant is `effective_spill_threshold / active`,
/// where `active` counts live pools *including* the new one — so the first
/// query of a session may take the whole headroom, while the second shares it
/// and the third gets only a third, and so on. This is the "grant per-query"
/// half of P110.1; admission (P110.2) and reclaim (P110.3) build on it.
#[derive(Debug)]
pub struct MemoryGovernor {
    inner: Arc<GovernorShared>,
}

impl MemoryGovernor {
    /// Create a governor deriving grants from `memory`.
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self {
            inner: Arc::new(GovernorShared {
                memory,
                active_queries: AtomicU64::new(0),
                next_query_id: AtomicU64::new(1),
            }),
        }
    }

    /// The global budget tracker this governor draws grants from.
    pub fn memory(&self) -> &MemoryManager {
        &self.inner.memory
    }

    /// Number of queries currently holding a live pool.
    pub fn active_queries(&self) -> u64 {
        self.inner.active_queries.load(Ordering::Relaxed)
    }

    /// Grant a memory pool to a new query.
    ///
    /// The grant is a fair share of [`MemoryManager::effective_spill_threshold`]
    /// (headroom after buffer-pool/index allocations). Drops with zero headroom
    /// yield a grant of 0 — the pool refuses every reservation, so the query
    /// must run fully external.
    pub fn grant_new_query(&self) -> QueryMemoryPool {
        let query_id = self.inner.next_query_id.fetch_add(1, Ordering::Relaxed);
        let grant = {
            // Count this query in the share before computing it.
            let _ = self.inner.active_queries.fetch_add(1, Ordering::Relaxed);
            self.inner.fair_share()
        };
        QueryMemoryPool {
            query_id,
            grant_bytes: grant,
            reserved_bytes: AtomicU64::new(0),
            peak_bytes: AtomicU64::new(0),
            spill_events: AtomicU64::new(0),
            governor: Some(self.inner.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_account::BUFFER_POOL;

    #[test]
    fn test_reserve_and_release_roundtrip() {
        let pool = QueryMemoryPool::new(7, 1000);
        assert_eq!(pool.query_id(), 7);
        assert_eq!(pool.grant_bytes(), 1000);
        assert_eq!(pool.try_reserve(400), Grant::Granted);
        assert_eq!(pool.reserved(), 400);
        assert_eq!(pool.remaining(), 600);
        assert_eq!(pool.peak(), 400);
        pool.release(250);
        assert_eq!(pool.reserved(), 150);
        assert_eq!(pool.remaining(), 850);
    }

    #[test]
    fn test_refuses_beyond_grant() {
        let pool = QueryMemoryPool::new(1, 100);
        assert_eq!(pool.try_reserve(100), Grant::Granted);
        // Exactly at the grant: no room left.
        assert_eq!(pool.try_reserve(1), Grant::Exhausted);
        assert_eq!(pool.reserved(), 100);
        assert_eq!(pool.remaining(), 0);
        assert!(!pool.can_reserve(1));
    }

    #[test]
    fn test_try_reserve_does_not_partially_commit() {
        let pool = QueryMemoryPool::new(2, 100);
        assert_eq!(pool.try_reserve(90), Grant::Granted);
        // A request larger than the remaining grant is refused wholesale.
        assert_eq!(pool.try_reserve(20), Grant::Exhausted);
        assert_eq!(pool.reserved(), 90);
        pool.release(90);
        assert_eq!(pool.try_reserve(20), Grant::Granted);
        assert_eq!(pool.reserved(), 20);
    }

    #[test]
    fn test_release_never_negative() {
        let pool = QueryMemoryPool::new(3, 100);
        pool.try_reserve(50);
        pool.release(10_000);
        assert_eq!(pool.reserved(), 0);
        assert_eq!(pool.remaining(), 100);
    }

    #[test]
    fn test_peak_tracks_maximum() {
        let pool = QueryMemoryPool::new(4, 1000);
        pool.try_reserve(200);
        pool.try_reserve(300);
        assert_eq!(pool.peak(), 500);
        pool.release(500);
        pool.try_reserve(100);
        assert_eq!(pool.peak(), 500);
    }

    #[test]
    fn test_pressure_and_under_pressure() {
        let pool = QueryMemoryPool::new(5, 100);
        assert!((pool.pressure() - 0.0).abs() < 1e-9);
        pool.try_reserve(80);
        assert!((pool.pressure() - 0.8).abs() < 1e-9);
        assert!(pool.is_under_pressure(0.8));
        assert!(!pool.is_under_pressure(0.9));
    }

    #[test]
    fn test_zero_grant_always_exhausted() {
        let pool = QueryMemoryPool::new(6, 0);
        assert!(!pool.can_reserve(1));
        assert_eq!(pool.try_reserve(1), Grant::Exhausted);
        // Zero-grant pool reports full pressure (no headroom at all).
        assert_eq!(pool.pressure(), 1.0);
    }

    #[test]
    fn test_note_spill_counts() {
        let pool = QueryMemoryPool::new(8, 100);
        assert_eq!(pool.spill_events(), 0);
        pool.note_spill();
        pool.note_spill();
        assert_eq!(pool.spill_events(), 2);
    }

    #[test]
    fn test_shrunk_grant_signals_spill() {
        let mut pool = QueryMemoryPool::new(9, 1000);
        pool.try_reserve(600);
        // Reclaim shrinks the grant (P110.3): remaining collapses to 0.
        pool.set_grant(400);
        assert_eq!(pool.remaining(), 0);
        assert_eq!(pool.try_reserve(1), Grant::Exhausted);
        // After releasing below the new grant, reserving works again.
        pool.release(300);
        assert_eq!(pool.try_reserve(50), Grant::Granted);
    }

    #[test]
    fn test_governor_fair_share_grants() {
        let mm = Arc::new(MemoryManager::new(1000));
        let governor = MemoryGovernor::new(mm);
        let q1 = governor.grant_new_query();
        let q2 = governor.grant_new_query();
        let q3 = governor.grant_new_query();
        assert_eq!(q1.grant_bytes(), 1000);
        assert_eq!(q2.grant_bytes(), 500);
        assert_eq!(q3.grant_bytes(), 333);
        assert_eq!(governor.active_queries(), 3);
        assert_eq!(q1.query_id(), 1);
        assert_eq!(q2.query_id(), 2);
        assert_eq!(q3.query_id(), 3);
    }

    #[test]
    fn test_governor_drop_releases_active_slot() {
        let mm = Arc::new(MemoryManager::new(1000));
        let governor = MemoryGovernor::new(mm);
        let q1 = governor.grant_new_query();
        assert_eq!(q1.grant_bytes(), 1000);
        assert_eq!(governor.active_queries(), 1);
        drop(q1);
        assert_eq!(governor.active_queries(), 0);
        let q2 = governor.grant_new_query();
        assert_eq!(q2.grant_bytes(), 1000);
    }

    #[test]
    fn test_governor_reflects_global_pressure() {
        let mm = Arc::new(MemoryManager::new(1000));
        let governor = MemoryGovernor::new(mm.clone());
        // Buffer pool eats 700 of the budget: headroom is 300.
        mm.allocate_with(BUFFER_POOL, 700);
        let q = governor.grant_new_query();
        assert_eq!(q.grant_bytes(), 300);
        // Two concurrent queries under pressure get 150 each.
        let q2 = governor.grant_new_query();
        assert_eq!(q2.grant_bytes(), 150);
    }

    #[test]
    fn test_governor_depleted_headroom_saturates_zero() {
        let mm = Arc::new(MemoryManager::new(1000));
        let governor = MemoryGovernor::new(mm.clone());
        mm.allocate_with(BUFFER_POOL, 10_000);
        let q = governor.grant_new_query();
        assert_eq!(q.grant_bytes(), 0);
        assert_eq!(q.try_reserve(1), Grant::Exhausted);
    }
}
