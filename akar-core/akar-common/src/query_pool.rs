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
//!   so later grants shrink under concurrency. [`MemoryGovernor::admit`] adds the
//!   admission gate (P110.2) and [`MemoryGovernor::reclaim_under_pressure`] the
//!   active reclaim driver (P110.3).
//! - [`QueryMemoryPool`] — per-query grant enforcement. Operators reserve bytes
//!   before growing in-memory structures; an exhausted grant signals "spill
//!   now" instead of allowing unbounded growth (OOM).

use crate::memory::MemoryManager;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};

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
    ///
    /// Atomic rather than plain so the governor can shrink it on a live query
    /// (active reclaim, P110.3) — the pool stays reachable from the query while
    /// the governor adjusts the ceiling from another thread.
    grant_bytes: AtomicU64,
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
            grant_bytes: AtomicU64::new(grant_bytes),
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
        self.grant_bytes.load(Ordering::Relaxed)
    }

    /// Replace the grant (grow or shrink). Shrinking below current reserved
    /// saturates `remaining()` to 0, which immediately signals "spill" to
    /// reserving operators — the mechanism used by active reclaim (P110.3).
    pub fn set_grant(&self, grant_bytes: u64) {
        self.grant_bytes.store(grant_bytes, Ordering::Relaxed);
    }

    /// Shrink the grant down to what the query has already reserved, so
    /// `remaining()` collapses to 0 and every subsequent reservation is
    /// refused. Returns the headroom that was removed.
    ///
    /// This is the active-reclaim primitive (P110.3): it cannot free memory by
    /// itself, it makes the query's operators *choose* to spill.
    pub fn shrink_to_reserved(&self) -> u64 {
        let reserved = self.reserved();
        let previous = self.grant_bytes.swap(reserved, Ordering::Relaxed);
        previous.saturating_sub(reserved)
    }

    /// Bytes currently reserved by the query's operators.
    pub fn reserved(&self) -> u64 {
        self.reserved_bytes.load(Ordering::Relaxed)
    }

    /// Headroom left before the grant is exhausted (saturates at 0).
    pub fn remaining(&self) -> u64 {
        self.grant_bytes().saturating_sub(self.reserved())
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
    ///
    /// Also counted on the governor, so total spilling stays observable after
    /// the query — and its pool — are gone.
    pub fn note_spill(&self) {
        self.spill_events.fetch_add(1, Ordering::Relaxed);
        if let Some(governor) = &self.governor {
            governor.spill_events.fetch_add(1, Ordering::Relaxed);
        }
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
        let grant = self.grant_bytes();
        if grant == 0 {
            return 1.0;
        }
        (self.reserved() as f64 / grant as f64).clamp(0.0, 1.0)
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
    /// Queries refused by [`MemoryGovernor::admit`] since the governor started.
    rejected_queries: AtomicU64,
    /// Partitioning passes that reached disk, across every query this governor
    /// has admitted.
    spill_events: AtomicU64,
    /// Admission/reclaim thresholds (P110.2/P110.3).
    policy: RwLock<GovernorPolicy>,
    /// Weak handles to live pools, so the governor can shrink a grant on a query
    /// it does not own (active reclaim, P110.3). Weak — the registry never keeps
    /// a query's pool alive.
    live_pools: Mutex<Vec<Weak<QueryMemoryPool>>>,
}

impl GovernorShared {
    /// Fair share of the current headroom for one of `active` queries.
    fn fair_share(&self) -> u64 {
        let active = self.active_queries.load(Ordering::Relaxed).max(1);
        self.memory.effective_spill_threshold() / active
    }
}

/// Thresholds governing admission of new queries and active reclaim (P110.2,
/// P110.3).
///
/// Every threshold defaults to *inert* — no ceiling, no cap, no floor — so an
/// instance that arms nothing behaves exactly as it did before the gate existed.
/// That default is deliberate: the budget a governor draws from is also the
/// configured `max_db_size`, and a database whose budget is smaller than its own
/// baseline buffer-pool allocation sits at 100% pressure permanently. A gate that
/// refused queries in that state would brick small instances rather than protect
/// them. Embedders that want protection arm specific thresholds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GovernorPolicy {
    /// Refuse a new query when the global budget is at least this fraction
    /// allocated. `f64::INFINITY` (the default) never refuses on pressure.
    pub admit_max_pressure: f64,
    /// Refuse a new query when this many already hold a pool. `0` = no cap.
    pub admit_max_concurrent_queries: u64,
    /// Refuse a new query whose fair share would fall below this many bytes.
    /// `0` = no floor (a zero grant simply means "run fully external").
    pub admit_min_grant_bytes: u64,
    /// [`MemoryGovernor::reclaim_under_pressure`] acts at or above this fraction
    /// of the global budget.
    pub reclaim_pressure: f64,
}

impl Default for GovernorPolicy {
    fn default() -> Self {
        Self {
            admit_max_pressure: f64::INFINITY,
            admit_max_concurrent_queries: 0,
            admit_min_grant_bytes: 0,
            reclaim_pressure: 0.9,
        }
    }
}

/// Outcome of [`MemoryGovernor::admit`].
#[derive(Debug)]
pub enum Admission {
    /// The query may run; its memory pool is attached.
    Admitted(Arc<QueryMemoryPool>),
    /// The query was refused; the caller should report the reason instead of
    /// running it.
    Rejected(AdmissionRejection),
}

/// Why [`MemoryGovernor::admit`] refused a query.
#[derive(Debug, Clone, PartialEq)]
pub enum AdmissionRejection {
    /// The global budget is at or above the policy ceiling.
    MemoryPressure { pressure: f64, max_pressure: f64 },
    /// The concurrency cap is already reached.
    TooManyConcurrentQueries { active: u64, max_concurrent: u64 },
    /// The fair share the query would receive is below the policy floor.
    InsufficientHeadroom { grant: u64, min_grant: u64 },
}

impl AdmissionRejection {
    /// Human-readable reason, for callers that surface admission failures as
    /// query errors.
    pub fn reason(&self) -> String {
        match self {
            Self::MemoryPressure { pressure, max_pressure } => format!(
                "instance memory pressure {:.0}% is at or above the admission ceiling {:.0}%",
                pressure * 100.0,
                max_pressure * 100.0
            ),
            Self::TooManyConcurrentQueries { active, max_concurrent } => {
                format!("{active} queries are already running (limit {max_concurrent})")
            }
            Self::InsufficientHeadroom { grant, min_grant } => {
                format!("only {grant} bytes of memory could be granted for this query (minimum {min_grant})")
            }
        }
    }
}

impl std::fmt::Display for AdmissionRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason())
    }
}

impl std::error::Error for AdmissionRejection {}

/// What [`MemoryGovernor::reclaim`] managed to take back.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReclaimOutcome {
    /// Number of live pools whose grant was shrunk.
    pub reclaimed_queries: u64,
    /// Headroom removed from those grants, in bytes.
    pub headroom_freed: u64,
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
        Self::with_policy(memory, GovernorPolicy::default())
    }

    /// Create a governor with an explicit admission/reclaim policy (P110.2).
    pub fn with_policy(memory: Arc<MemoryManager>, policy: GovernorPolicy) -> Self {
        Self {
            inner: Arc::new(GovernorShared {
                memory,
                active_queries: AtomicU64::new(0),
                next_query_id: AtomicU64::new(1),
                rejected_queries: AtomicU64::new(0),
                spill_events: AtomicU64::new(0),
                policy: RwLock::new(policy),
                live_pools: Mutex::new(Vec::new()),
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

    /// How many queries [`MemoryGovernor::admit`] has refused so far.
    pub fn rejected_queries(&self) -> u64 {
        self.inner.rejected_queries.load(Ordering::Relaxed)
    }

    /// Partitioning passes that spilled to disk across every query this governor
    /// has admitted (P111). Stays readable after those queries have finished.
    pub fn spill_events(&self) -> u64 {
        self.inner.spill_events.load(Ordering::Relaxed)
    }

    /// The current admission/reclaim policy.
    pub fn policy(&self) -> GovernorPolicy {
        match self.inner.policy.read() {
            Ok(guard) => *guard,
            // A poisoned policy lock still holds a usable value; refusing to
            // honour it would silently disable the OOM gate.
            Err(poisoned) => *poisoned.into_inner(),
        }
    }

    /// Replace the admission/reclaim policy (P110.2, P110.3).
    pub fn set_policy(&self, policy: GovernorPolicy) {
        match self.inner.policy.write() {
            Ok(mut guard) => *guard = policy,
            Err(poisoned) => *poisoned.into_inner() = policy,
        }
    }

    /// Grant a memory pool to a new query.
    ///
    /// The grant is a fair share of [`MemoryManager::effective_spill_threshold`]
    /// (headroom after buffer-pool/index allocations). Drops with zero headroom
    /// yield a grant of 0 — the pool refuses every reservation, so the query
    /// must run fully external.
    ///
    /// This is the unconditional path: it never refuses. Use
    /// [`MemoryGovernor::admit`] when the caller wants the admission gate.
    pub fn grant_new_query(&self) -> Arc<QueryMemoryPool> {
        let query_id = self.inner.next_query_id.fetch_add(1, Ordering::Relaxed);
        let grant = {
            // Count this query in the share before computing it.
            let _ = self.inner.active_queries.fetch_add(1, Ordering::Relaxed);
            self.inner.fair_share()
        };
        let pool = Arc::new(QueryMemoryPool {
            query_id,
            grant_bytes: AtomicU64::new(grant),
            reserved_bytes: AtomicU64::new(0),
            peak_bytes: AtomicU64::new(0),
            spill_events: AtomicU64::new(0),
            governor: Some(self.inner.clone()),
        });
        // Register a weak handle so active reclaim (P110.3) can reach this
        // pool later without keeping it alive.
        match self.inner.live_pools.lock() {
            Ok(mut live) => live.push(Arc::downgrade(&pool)),
            Err(poisoned) => poisoned.into_inner().push(Arc::downgrade(&pool)),
        }
        pool
    }

    /// Admission gate for a new query (P110.2).
    ///
    /// Applies [`GovernorPolicy`] before handing out a grant. Under the default
    /// policy every threshold is inert, so this is equivalent to
    /// [`MemoryGovernor::grant_new_query`]; callers that want protection arm a
    /// ceiling, a cap or a floor via [`MemoryGovernor::set_policy`].
    ///
    /// Rejections are counted and can be read back via
    /// [`MemoryGovernor::rejected_queries`].
    pub fn admit(&self) -> Admission {
        let policy = self.policy();
        let pressure = self.inner.memory.memory_pressure();
        let active = self.active_queries();
        // The share this query would receive, computed the same way
        // `grant_new_query` does (this query included in the divisor).
        let prospective_grant = self.inner.memory.effective_spill_threshold() / (active + 1).max(1);

        let rejection = if pressure >= policy.admit_max_pressure {
            Some(AdmissionRejection::MemoryPressure {
                pressure,
                max_pressure: policy.admit_max_pressure,
            })
        } else if policy.admit_max_concurrent_queries > 0 && active >= policy.admit_max_concurrent_queries {
            Some(AdmissionRejection::TooManyConcurrentQueries {
                active,
                max_concurrent: policy.admit_max_concurrent_queries,
            })
        } else if policy.admit_min_grant_bytes > 0 && prospective_grant < policy.admit_min_grant_bytes {
            Some(AdmissionRejection::InsufficientHeadroom {
                grant: prospective_grant,
                min_grant: policy.admit_min_grant_bytes,
            })
        } else {
            None
        };

        match rejection {
            Some(reason) => {
                self.inner.rejected_queries.fetch_add(1, Ordering::Relaxed);
                Admission::Rejected(reason)
            }
            None => Admission::Admitted(self.grant_new_query()),
        }
    }

    /// Number of live pools (queries whose pool has not been dropped).
    pub fn live_pools(&self) -> u64 {
        match self.inner.live_pools.lock() {
            Ok(live) => live.iter().filter(|w| w.strong_count() > 0).count() as u64,
            Err(poisoned) => poisoned.into_inner().iter().filter(|w| w.strong_count() > 0).count() as u64,
        }
    }

    /// Active reclaim (P110.3): shrink the grants of the `max_queries` youngest
    /// live queries so their operators observe [`Grant::Exhausted`] and spill.
    ///
    /// Youngest-first is deliberate — long-running queries have already invested
    /// memory, while the newest one has the least to lose. Shrinking a grant
    /// frees no memory by itself: it converts global pressure into a per-query
    /// spill signal, which is the only lever a governor has that cannot corrupt
    /// or silently truncate an in-flight query.
    pub fn reclaim(&self, max_queries: usize) -> ReclaimOutcome {
        if max_queries == 0 {
            return ReclaimOutcome::default();
        }
        let pools: Vec<Arc<QueryMemoryPool>> = match self.inner.live_pools.lock() {
            Ok(mut live) => {
                // Drop dead handles while we are here, so the registry cannot
                // grow with every query a long-lived process runs.
                live.retain(|w| w.strong_count() > 0);
                live.iter().filter_map(Weak::upgrade).collect()
            }
            Err(poisoned) => {
                let mut live = poisoned.into_inner();
                live.retain(|w| w.strong_count() > 0);
                live.iter().filter_map(Weak::upgrade).collect()
            }
        };

        // Only pools that still hold headroom are candidates. A pool already
        // shrunk to its reservation has nothing left to give, and skipping it
        // keeps repeated `reclaim` calls making progress instead of re-picking
        // the same youngest query forever.
        let mut ordered: Vec<Arc<QueryMemoryPool>> = pools.into_iter().filter(|p| p.remaining() > 0).collect();
        // Youngest first.
        ordered.sort_by_key(|p| std::cmp::Reverse(p.query_id()));

        let mut outcome = ReclaimOutcome::default();
        for pool in ordered.iter().take(max_queries) {
            outcome.reclaimed_queries += 1;
            outcome.headroom_freed += pool.shrink_to_reserved();
        }
        outcome
    }

    /// Reclaim only when the global budget is at or above
    /// [`GovernorPolicy::reclaim_pressure`]; otherwise a no-op.
    pub fn reclaim_under_pressure(&self, max_queries: usize) -> ReclaimOutcome {
        if self.inner.memory.memory_pressure() < self.policy().reclaim_pressure {
            return ReclaimOutcome::default();
        }
        self.reclaim(max_queries)
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
        let pool = QueryMemoryPool::new(9, 1000);
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
    fn test_shrink_to_reserved_reports_headroom_freed() {
        let pool = QueryMemoryPool::new(10, 1000);
        pool.try_reserve(400);
        // 600 bytes of headroom are taken back and reported.
        assert_eq!(pool.shrink_to_reserved(), 600);
        assert_eq!(pool.grant_bytes(), 400);
        assert_eq!(pool.remaining(), 0);
        // Idempotent: already at the reservation, nothing further to take.
        assert_eq!(pool.shrink_to_reserved(), 0);
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

    // ---- P110.2: admission gate -------------------------------------------

    #[test]
    fn test_default_policy_admits_while_headroom_remains() {
        let mm = Arc::new(MemoryManager::new(1000));
        let governor = MemoryGovernor::new(mm.clone());
        // Half the budget still free: admitted under the default policy.
        mm.allocate_with(BUFFER_POOL, 500);
        match governor.admit() {
            Admission::Admitted(pool) => assert_eq!(pool.grant_bytes(), 500),
            Admission::Rejected(r) => panic!("expected admission, got {r}"),
        }
        assert_eq!(governor.rejected_queries(), 0);
    }

    #[test]
    fn test_admission_refused_when_budget_exhausted() {
        let mm = Arc::new(MemoryManager::new(1000));
        // The ceiling must be armed explicitly: the default policy never refuses
        // on pressure, so that a small-budget instance is not locked out.
        let governor = MemoryGovernor::with_policy(
            mm.clone(),
            GovernorPolicy {
                admit_max_pressure: 1.0,
                ..GovernorPolicy::default()
            },
        );
        mm.allocate_with(BUFFER_POOL, 1000);
        match governor.admit() {
            Admission::Rejected(AdmissionRejection::MemoryPressure { pressure, max_pressure }) => {
                assert!((pressure - 1.0).abs() < 1e-9);
                assert!((max_pressure - 1.0).abs() < 1e-9);
            }
            other => panic!("expected memory-pressure rejection, got {other:?}"),
        }
        assert_eq!(governor.rejected_queries(), 1);
        // A refusal must not consume an active-query slot.
        assert_eq!(governor.active_queries(), 0);
    }

    #[test]
    fn test_exhausted_budget_is_admitted_under_the_default_policy() {
        // Regression guard for the default: a database whose baseline allocation
        // already exceeds its configured budget must still be usable.
        let mm = Arc::new(MemoryManager::new(1000));
        let governor = MemoryGovernor::new(mm.clone());
        mm.allocate_with(BUFFER_POOL, 10_000);
        match governor.admit() {
            Admission::Admitted(pool) => {
                assert_eq!(pool.grant_bytes(), 0);
                assert_eq!(pool.try_reserve(1), Grant::Exhausted);
            }
            Admission::Rejected(r) => panic!("default policy must not refuse on pressure: {r}"),
        }
    }

    #[test]
    fn test_admission_concurrency_cap() {
        let mm = Arc::new(MemoryManager::new(1000));
        let governor = MemoryGovernor::with_policy(
            mm,
            GovernorPolicy {
                admit_max_concurrent_queries: 2,
                ..GovernorPolicy::default()
            },
        );
        let q1 = governor.admit();
        let q2 = governor.admit();
        assert!(matches!(q1, Admission::Admitted(_)));
        assert!(matches!(q2, Admission::Admitted(_)));
        match governor.admit() {
            Admission::Rejected(AdmissionRejection::TooManyConcurrentQueries { active, max_concurrent }) => {
                assert_eq!(active, 2);
                assert_eq!(max_concurrent, 2);
            }
            other => panic!("expected concurrency rejection, got {other:?}"),
        }
        // Releasing a running query frees the slot again.
        drop(q1);
        assert!(matches!(governor.admit(), Admission::Admitted(_)));
    }

    #[test]
    fn test_admission_headroom_floor() {
        let mm = Arc::new(MemoryManager::new(1000));
        let governor = MemoryGovernor::with_policy(
            mm.clone(),
            GovernorPolicy {
                admit_min_grant_bytes: 400,
                ..GovernorPolicy::default()
            },
        );
        // 400 free and no other query: the share is 400, which clears the floor.
        mm.allocate_with(BUFFER_POOL, 600);
        let first = governor.admit();
        assert!(matches!(first, Admission::Admitted(_)));

        // 100 free but one query is already live, so the share a second query
        // would get is 50 — below the 400-byte floor. `first` must stay alive
        // for this to be observable, since dropping a pool frees its slot.
        mm.allocate_with(BUFFER_POOL, 300);
        match governor.admit() {
            Admission::Rejected(AdmissionRejection::InsufficientHeadroom { grant, min_grant }) => {
                assert_eq!(grant, 50);
                assert_eq!(min_grant, 400);
            }
            other => panic!("expected headroom rejection, got {other:?}"),
        }
    }

    #[test]
    fn test_policy_roundtrip_and_rejection_reason() {
        let mm = Arc::new(MemoryManager::new(1000));
        let governor = MemoryGovernor::new(mm);
        assert_eq!(governor.policy(), GovernorPolicy::default());
        let policy = GovernorPolicy {
            admit_max_pressure: 0.5,
            admit_max_concurrent_queries: 7,
            admit_min_grant_bytes: 1234,
            reclaim_pressure: 0.4,
        };
        governor.set_policy(policy);
        assert_eq!(governor.policy(), policy);

        let reason = AdmissionRejection::MemoryPressure {
            pressure: 0.93,
            max_pressure: 0.9,
        };
        let text = reason.reason();
        assert!(text.contains("93%"), "unhelpful reason: {text}");
        assert!(text.contains("90%"), "unhelpful reason: {text}");
    }

    // ---- P110.3: active reclaim -------------------------------------------

    #[test]
    fn test_reclaim_shrinks_youngest_first() {
        let mm = Arc::new(MemoryManager::new(1000));
        let governor = MemoryGovernor::new(mm);
        let q1 = governor.grant_new_query();
        let q2 = governor.grant_new_query();
        assert_eq!(q1.grant_bytes(), 1000);
        assert_eq!(q2.grant_bytes(), 500);

        let outcome = governor.reclaim(1);
        assert_eq!(outcome.reclaimed_queries, 1);
        assert_eq!(outcome.headroom_freed, 500);
        // The youngest (q2) was shrunk; the elder keeps its grant.
        assert_eq!(q2.grant_bytes(), 0);
        assert_eq!(q1.grant_bytes(), 1000);
        assert_eq!(q2.try_reserve(1), Grant::Exhausted);

        // Reclaiming the next youngest takes the elder's headroom too.
        let outcome = governor.reclaim(1);
        assert_eq!(outcome.reclaimed_queries, 1);
        assert_eq!(outcome.headroom_freed, 1000);
        assert_eq!(q1.grant_bytes(), 0);
    }

    #[test]
    fn test_reclaim_keeps_reserved_bytes() {
        let mm = Arc::new(MemoryManager::new(1000));
        let governor = MemoryGovernor::new(mm);
        let q = governor.grant_new_query();
        q.try_reserve(400);
        let outcome = governor.reclaim(4);
        // Grant drops to what is already reserved — the query keeps its memory,
        // it just cannot grow further without spilling.
        assert_eq!(outcome.headroom_freed, 600);
        assert_eq!(q.grant_bytes(), 400);
        assert_eq!(q.reserved(), 400);
        assert_eq!(q.remaining(), 0);
    }

    #[test]
    fn test_reclaim_ignores_dropped_pools_and_zero_budget() {
        let mm = Arc::new(MemoryManager::new(1000));
        let governor = MemoryGovernor::new(mm);
        let q1 = governor.grant_new_query();
        let q2 = governor.grant_new_query();
        drop(q2);
        assert_eq!(governor.live_pools(), 1);
        // A dead handle is pruned, not counted as reclaimed.
        let outcome = governor.reclaim(10);
        assert_eq!(outcome.reclaimed_queries, 1);
        assert_eq!(outcome.headroom_freed, 1000);
        assert_eq!(governor.live_pools(), 1);
        drop(q1);
        assert_eq!(governor.live_pools(), 0);
        assert_eq!(governor.reclaim(10), ReclaimOutcome::default());
        assert_eq!(governor.reclaim(0), ReclaimOutcome::default());
    }

    #[test]
    fn test_reclaim_under_pressure_respects_threshold() {
        let mm = Arc::new(MemoryManager::new(1000));
        let governor = MemoryGovernor::with_policy(
            mm.clone(),
            GovernorPolicy {
                reclaim_pressure: 0.8,
                ..GovernorPolicy::default()
            },
        );
        let q = governor.grant_new_query();
        assert_eq!(q.grant_bytes(), 1000);

        // Below the threshold: reclaim is a no-op and the pool keeps its grant.
        mm.allocate_with(BUFFER_POOL, 700);
        assert_eq!(governor.reclaim_under_pressure(4), ReclaimOutcome::default());
        assert_eq!(q.remaining(), 1000);

        // At the threshold the grant shrinks to what is reserved (nothing), so
        // the pool refuses every new reservation — the spill signal.
        mm.allocate_with(BUFFER_POOL, 100);
        let outcome = governor.reclaim_under_pressure(4);
        assert_eq!(outcome.reclaimed_queries, 1);
        assert_eq!(outcome.headroom_freed, 1000);
        assert_eq!(q.grant_bytes(), 0);
        assert_eq!(q.try_reserve(1), Grant::Exhausted);
    }
}
