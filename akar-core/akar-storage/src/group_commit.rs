//! Group commit — batches concurrent WAL flushes into a single fsync.
//!
//! # Model
//!
//! Leader/follower group commit sitting at the WAL durability boundary
//! (StorageManager `commit_transaction` step 1). Each submitting thread:
//!
//! 1. Appends its WAL records (including the `Commit` record) under the WAL
//!    lock — *without* an fsync.
//! 2. Enqueues a pending flush request tagged with the thread's LSN.
//! 3. Tries to become the *leader*; the leader drains the queue for
//!    [`GroupCommitConfig::drain_timeout`], then performs a **single** fsync
//!    covering every drained request and delivers the shared `durable_batch_lsn`
//!    back to each waiter.
//! 4. Followers wait on their own result slot; if no leader finishes them
//!    within [`GroupCommitConfig::leader_timeout`], they retry leadership
//!    (self-healing — e.g. a leader thread was descheduled).
//!
//! The durability guarantee is preserved: a submitter only returns success
//! after an fsync that started after it enqueued. Recovery format is
//! untouched — this coordinates *when* an fsync happens, never *what* is
//! written.
//!
//! # Fsync target
//!
//! [`GroupCommit`] is generic over [`WalLike`]; `akar-storage` implements it
//! for `Mutex<WAL>`, so a whole group shares the same underlying WAL file.

use std::collections::VecDeque;
use std::io::{self};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Default time a leader keeps draining the queue before fsyncing.
pub const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_micros(200);

/// Default time a follower waits before it attempts to take over leadership.
pub const DEFAULT_LEADER_TIMEOUT: Duration = Duration::from_millis(50);

/// A durable flush target that many commits can share.
///
/// For the WAL this maps to `wal.flush_to_disk()`, i.e. one `fsync`.
pub trait WalLike: Send + Sync {
    /// Durably persist all records appended to the target so far.
    fn flush_to_disk(&self) -> io::Result<()>;
}

impl WalLike for std::sync::Mutex<crate::wal::WAL> {
    fn flush_to_disk(&self) -> io::Result<()> {
        let mut wal = self
            .lock()
            .map_err(|e| io::Error::other(format!("WAL lock poisoned: {e}")))?;
        wal.flush_to_disk()
    }
}

/// Group-commit configuration.
#[derive(Debug, Clone, Copy)]
pub struct GroupCommitConfig {
    /// Master switch; when `false` callers should fall back to an inline
    /// fsync (the coordinator itself is inert about the flag).
    pub enabled: bool,
    /// Window a leader keeps collecting submissions before fsyncing.
    pub drain_timeout: Duration,
    /// How long a follower waits before attempting to take over leadership.
    pub leader_timeout: Duration,
}

impl Default for GroupCommitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            drain_timeout: DEFAULT_DRAIN_TIMEOUT,
            leader_timeout: DEFAULT_LEADER_TIMEOUT,
        }
    }
}

/// Result delivered to a submitter once its group is durable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupCommitResult {
    /// This commit's LSN (monotonic across submissions, FIFO).
    pub lsn: u64,
    /// Highest LSN covered by the fsync that made this commit durable.
    /// `>= lsn` because the whole group shares one fsync.
    pub durable_batch_lsn: u64,
    /// Number of requests coalesced into this commit's fsync group.
    pub group_size: usize,
}

/// Simple cumulative statistics for observability.
#[derive(Debug, Default, Clone, Copy)]
pub struct GroupCommitStats {
    /// Number of fsync groups completed.
    pub batches: u64,
    /// Number of commits that each waited for a group fsync.
    pub commits: u64,
    /// Total group sizes across all batches (for average sizing).
    pub total_group_entries: u64,
}

impl GroupCommitStats {
    /// Mean group size across completed batches (0.0 when none yet).
    pub fn avg_group_size(&self) -> f64 {
        if self.batches == 0 {
            0.0
        } else {
            self.total_group_entries as f64 / self.batches as f64
        }
    }
}

/// Pending flush request waiting for a group-leader fsync.
///
/// `None` means not yet delivered (still pending); `Some(Ok/Err)` is the
/// group outcome recorded by the leading commit.
struct PendingCommit {
    lsn: u64,
    slot: Arc<(Mutex<Option<io::Result<GroupCommitResult>>>, Condvar)>,
}

fn new_slot() -> Arc<(Mutex<Option<io::Result<GroupCommitResult>>>, Condvar)> {
    Arc::new((Mutex::new(None), Condvar::new()))
}

/// Leader/follower group-commit coordinator over a shared [`WalLike`] target.
pub struct GroupCommit<W: WalLike> {
    wal: Arc<W>,
    config: GroupCommitConfig,
    /// Monotonic LSN assigned at submission time (FIFO for committed groups).
    next_lsn: AtomicU64,
    queue: Mutex<VecDeque<PendingCommit>>,
    /// Serializes leadership — at most one leader drains+fsyncs at a time.
    leader_lock: Mutex<()>,
    stats: Mutex<GroupCommitStats>,
}

impl<W: WalLike> GroupCommit<W> {
    /// Create a coordinator over `wal` with the given config.
    pub fn new(wal: Arc<W>, config: GroupCommitConfig) -> Self {
        Self {
            wal,
            config,
            next_lsn: AtomicU64::new(0),
            queue: Mutex::new(VecDeque::new()),
            leader_lock: Mutex::new(()),
            stats: Mutex::new(GroupCommitStats::default()),
        }
    }

    pub fn config(&self) -> &GroupCommitConfig {
        &self.config
    }

    /// Current cumulative statistics.
    pub fn stats(&self) -> GroupCommitStats {
        *self.stats.lock().unwrap()
    }

    /// Wait for a durable flush of everything appended so far.
    ///
    /// Enqueues this submission, then either leads this group or waits for a
    /// leader to include it. Returns once an fsync that started after the
    /// enqueue has completed.
    pub fn flush(&self) -> io::Result<GroupCommitResult> {
        let lsn = self.next_lsn.fetch_add(1, Ordering::Relaxed) + 1;
        let slot = new_slot();
        {
            let mut queue = self.queue.lock().unwrap();
            queue.push_back(PendingCommit {
                lsn,
                slot: slot.clone(),
            });
        }

        // Try to become leader — the guard is handed to `run_leader` and held
        // for the whole drain, so at most one leader flushes per group.
        if let Ok(guard) = self.leader_lock.try_lock() {
            self.run_leader(guard);
        }
        let deadline = Instant::now() + self.config.leader_timeout;
        let (lock, cvar) = &*slot;
        let mut result = lock.lock().unwrap();
        loop {
            if let Some(outcome) = result.take() {
                return match outcome {
                    Ok(ok) => Ok(ok),
                    Err(e) => Err(io::Error::new(e.kind(), e.to_string())),
                };
            }
            if Instant::now() >= deadline {
                // Release our slot lock before leading: `run_leader` must be
                // able to deliver into every drained slot, including our own.
                drop(result);
                if let Ok(guard) = self.leader_lock.try_lock() {
                    self.run_leader(guard);
                }
                result = lock.lock().unwrap();
                continue;
            }
            let (guard, _to) = cvar.wait_timeout(result, Duration::from_millis(1)).unwrap();
            result = guard;
        }
    }

    /// Become the group leader: collect submissions for `drain_timeout`, then
    /// perform one fsync covering the whole drained batch.
    ///
    /// `_guard` is the acquired leader lock, kept alive for the whole drain so
    /// exactly one leader flushes per group.
    fn run_leader(&self, _guard: MutexGuard<'_, ()>) {
        let deadline = Instant::now() + self.config.drain_timeout;
        let mut batch: Vec<PendingCommit> = Vec::new();

        loop {
            let mut drained = {
                let mut queue = self.queue.lock().unwrap();
                let mut v = Vec::with_capacity(queue.len());
                while let Some(entry) = queue.pop_front() {
                    v.push(entry);
                }
                drop(queue);
                v
            };
            if !drained.is_empty() {
                batch.append(&mut drained);
            }
            if batch.is_empty() {
                // Tasked but nothing queued yet (caller enques before
                // proposing leadership, so this only happens on re-entry).
                return;
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_micros(100));
        }

        let group_size = batch.len();
        let durable_batch_lsn = batch.iter().map(|e| e.lsn).max().unwrap_or(0);
        let io_result = self.wal.flush_to_disk();

        {
            let mut stats = self.stats.lock().unwrap();
            stats.batches += 1;
            stats.commits += group_size as u64;
            stats.total_group_entries += group_size as u64;
        }

        for entry in batch {
            let outcome = match &io_result {
                Ok(()) => Ok(GroupCommitResult {
                    lsn: entry.lsn,
                    durable_batch_lsn,
                    group_size,
                }),
                Err(e) => Err(io::Error::new(e.kind(), format!("group fsync failed: {e}"))),
            };
            let (lock, cvar) = &*entry.slot;
            let mut slot = lock.lock().unwrap();
            *slot = Some(outcome);
            drop(slot);
            cvar.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    use std::sync::atomic::Ordering as AtomicOrdering;
    use std::thread;

    /// Deterministic fake WAL: counts fsyncs and sleeps to widen the window so
    /// concurrent submissions can coalesce.
    struct MockWal {
        fsyncs: AtomicU64,
        delay: Duration,
    }

    impl WalLike for MockWal {
        fn flush_to_disk(&self) -> io::Result<()> {
            if !self.delay.is_zero() {
                thread::sleep(self.delay);
            }
            self.fsyncs.fetch_add(1, AtomicOrdering::SeqCst);
            Ok(())
        }
    }

    fn make_gc(drain: Duration) -> (Arc<GroupCommit<MockWal>>, Arc<MockWal>) {
        let wal = Arc::new(MockWal {
            fsyncs: AtomicU64::new(0),
            delay: Duration::from_millis(2),
        });
        let gc = Arc::new(GroupCommit::new(
            wal.clone(),
            GroupCommitConfig {
                enabled: true,
                drain_timeout: drain,
                leader_timeout: Duration::from_millis(100),
            },
        ));
        (gc, wal)
    }

    #[test]
    fn test_single_commit_flushes_once() {
        let (gc, wal) = make_gc(Duration::from_millis(2));
        let result = gc.flush().unwrap();
        assert_eq!(result.lsn, 1);
        assert_eq!(result.durable_batch_lsn, 1);
        assert_eq!(result.group_size, 1);
        assert_eq!(wal.fsyncs.load(AtomicOrdering::SeqCst), 1);
        let stats = gc.stats();
        assert_eq!(stats.batches, 1);
        assert_eq!(stats.commits, 1);
    }

    #[test]
    fn test_concurrent_commits_coalesce_into_one_fsync() {
        let (gc, wal) = make_gc(Duration::from_millis(10));
        let threads: Vec<_> = (0..12)
            .map(|_| {
                let gc = gc.clone();
                thread::spawn(move || gc.flush().unwrap())
            })
            .collect();
        let results: Vec<GroupCommitResult> = threads.into_iter().map(|t| t.join().unwrap()).collect();

        assert_eq!(results.len(), 12);
        // LSNs are strictly monotonic across submissions.
        let mut sorted: Vec<u64> = results.iter().map(|r| r.lsn).collect();
        sorted.sort_unstable();
        assert_eq!(sorted, (1..=12).collect::<Vec<u64>>());
        // Every commit's durable watermark is at least its own LSN.
        for r in &results {
            assert!(r.durable_batch_lsn >= r.lsn);
        }
        // At least one group covered more than one commit.
        assert!(results.iter().any(|r| r.group_size > 1), "expected coalescing");
        // Far fewer fsyncs than commits — exactly one per group.
        let fsyncs = wal.fsyncs.load(AtomicOrdering::SeqCst);
        assert!(fsyncs < 12, "group commit should batch fsyncs, got {fsyncs}");
        assert_eq!(fsyncs, gc.stats().batches);
        assert_eq!(gc.stats().commits, 12);
        assert!(gc.stats().avg_group_size() > 1.0);
    }

    #[test]
    fn test_concurrent_commits_no_coalescing_guarantee_needed() {
        // Even when coalescing doesn't happen, every commit must still return
        // success with correct per-LSN results.
        let (gc, wal) = make_gc(Duration::from_micros(0));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let gc = gc.clone();
                thread::spawn(move || gc.flush().unwrap())
            })
            .collect();
        let results: Vec<GroupCommitResult> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(results.len(), 8);
        let mut sorted: Vec<u64> = results.iter().map(|r| r.lsn).collect();
        sorted.sort_unstable();
        assert_eq!(sorted, (1..=8).collect::<Vec<u64>>());
        assert!(wal.fsyncs.load(AtomicOrdering::SeqCst) <= 8);
    }

    #[test]
    fn test_stale_follower_self_heals() {
        // A follower whose group was already flushed (its entry drained by a
        // leader before it reached the queue) grabs leadership on timeout and
        // fsyncs its own bytes — no hangs, no lost durability.
        let (gc, wal) = make_gc(Duration::from_millis(1));
        let garbage: Arc<(Mutex<Option<io::Result<GroupCommitResult>>>, Condvar)> = new_slot();
        // Push an orphaned entry nobody will complete.
        {
            let mut queue = gc.queue.lock().unwrap();
            queue.push_back(PendingCommit {
                lsn: 999,
                slot: garbage,
            });
        }
        // A real flush must complete despite the orphaned entry (it leads and
        // drains the orphan too).
        let result = gc.flush().unwrap();
        assert!(result.lsn >= 1);
        assert!(wal.fsyncs.load(AtomicOrdering::SeqCst) >= 1);
    }

    #[test]
    fn test_fsync_error_propagates() {
        struct FailingWal;
        impl WalLike for FailingWal {
            fn flush_to_disk(&self) -> io::Result<()> {
                Err(io::Error::other("disk on fire"))
            }
        }
        let gc = Arc::new(GroupCommit::new(Arc::new(FailingWal), GroupCommitConfig::default()));
        let err = gc.flush().unwrap_err();
        assert!(err.to_string().contains("disk on fire"));
    }
}
