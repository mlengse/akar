//! Dream engine control state for the akar server (P77).
//!
//! This module owns the per-server dream lifecycle ("status" / "pause" /
//! "resume" / "run") and the [`DreamOrchestrator`] that executes a
//! consolidation cycle. It is wired into [`SessionConfig`](crate::session::SessionConfig)
//! as a shared [`Arc`], so every client connection observes the same engine.
//!
//! The backend used today is a graceful-degradation stub ([`GraceBackend`]):
//! all 17 [`DreamBackend`] methods are no-ops that return zero/empty so the
//! lifecycle and wire contract are real and testable while the production
//! mapping onto the akar graph [`Database`](akar_main::database::Database) is
//! built out (P77b).

use akar_dream::backend::{self, DreamBackend};
use akar_dream::config::DreamConfig;
use akar_dream::orchestrator::{DreamOrchestrator, DreamStats};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Human-readable state of the dream engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DreamState {
    Idle,
    Running,
    Paused,
}

impl DreamState {
    pub fn as_str(&self) -> &'static str {
        match self {
            DreamState::Idle => "idle",
            DreamState::Running => "running",
            DreamState::Paused => "paused",
        }
    }
}

/// Backend that degrades gracefully: every [`DreamBackend`] method is a no-op
/// returning a zero/empty result, so a full cycle can run safely against an
/// empty graph until the real backend (P77b) is wired in.
#[derive(Default)]
pub struct GraceBackend;

impl DreamBackend for GraceBackend {
    fn sample_for_dream(
        &self,
        _max_memories: usize,
        _recent_pct: f64,
        _random_old_pct: f64,
        _low_salience_pct: f64,
    ) -> Vec<backend::Memory> {
        Vec::new()
    }

    fn get_connections(&self) -> Vec<backend::Edge> {
        Vec::new()
    }

    fn strengthen_edge(&self, _source_id: usize, _target_id: usize, _amount: f64) {}

    fn weaken_edge(&self, _source_id: usize, _target_id: usize, _amount: f64) {}

    fn prune_edge(&self, _source_id: usize, _target_id: usize) {}

    fn update_supersedes(&self) -> usize {
        0
    }

    fn find_bridges(
        &self,
        _communities: &[Vec<usize>],
        _embeddings: &[[f64; 384]],
        _max_bridges: usize,
    ) -> Vec<(usize, usize)> {
        Vec::new()
    }

    fn create_bridge_edges(&self, _bridges: &[(usize, usize)]) {}

    fn get_communities(&self) -> Vec<usize> {
        Vec::new()
    }

    fn write_communities(&self, _assignments: &[usize]) {}

    fn extract_afe_facts(&self, _memories: &[backend::Memory]) -> Vec<(String, usize)> {
        Vec::new()
    }

    fn write_afe_facts(&self, _facts: &[(String, usize)]) {}

    fn run_synthesis(&self) -> usize {
        0
    }

    fn recompute_dae(&self) -> usize {
        0
    }
}

/// Per-server dream engine lifecycle guarded behind a [`Mutex`].
///
/// Clients never lock the orchestrator for long: `run_cycle` may execute all
/// seven phases (though the graceful backend makes them instant today), so a
/// single shared instance serializes dreams across connections.
pub struct DreamControl {
    orchestrator: Mutex<DreamOrchestrator<GraceBackend>>,
    paused: AtomicBool,
    last_stats: Mutex<Option<DreamStats>>,
}

impl DreamControl {
    /// Create a fresh engine with default config. Dreams start enabled.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            orchestrator: Mutex::new(DreamOrchestrator::new(DreamConfig::default(), GraceBackend)),
            paused: AtomicBool::new(false),
            last_stats: Mutex::new(None),
        })
    }

    /// Current state from the caller's perspective.
    pub fn state(&self) -> DreamState {
        let paused = self.paused.load(Ordering::SeqCst);
        let has_stats = self.last_stats.lock().map(|s| s.is_some()).unwrap_or(false);
        match (paused, has_stats) {
            (true, _) => DreamState::Paused,
            (false, true) => DreamState::Running,
            (false, false) => DreamState::Idle,
        }
    }

    /// Run a full consolidation cycle, recording the resulting stats.
    ///
    /// No-op (and returns the current state) when the engine is paused.
    pub fn run(&self) -> DreamStats {
        if self.paused.load(Ordering::SeqCst) {
            return self.last_stats().unwrap_or_default();
        }
        let mut guard = match self.orchestrator.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let stats = guard.run_cycle();
        drop(guard);
        if let Ok(mut slot) = self.last_stats.lock() {
            *slot = Some(stats.clone());
        }
        stats
    }

    /// Optionally execute a cycle when unpaused; used by `run`/`resume`.
    pub fn resume(&self) -> DreamStats {
        self.paused.store(false, Ordering::SeqCst);
        self.run()
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    pub fn last_stats(&self) -> Option<DreamStats> {
        self.last_stats.lock().ok().and_then(|s| s.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dream_state_transitions() {
        let ctrl = DreamControl::new();
        assert_eq!(ctrl.state(), DreamState::Idle);

        let stats = ctrl.run();
        assert_eq!(stats.dream_id, 1);
        assert_eq!(ctrl.state(), DreamState::Running);

        // Paused: run() is a no-op and state flips to paused.
        ctrl.pause();
        assert_eq!(ctrl.state(), DreamState::Paused);
        let paused_stats = ctrl.run();
        assert_eq!(paused_stats.dream_id, 1, "paused run should not advance dream_id");

        // Resume restores running and runs a new cycle.
        let resumed = ctrl.resume();
        assert_eq!(resumed.dream_id, 2);
        assert_eq!(ctrl.state(), DreamState::Running);
    }

    #[test]
    fn test_grace_backend_all_zero() {
        let backend = GraceBackend;
        assert!(backend.sample_for_dream(10, 0.5, 0.2, 0.3).is_empty());
        assert!(backend.get_connections().is_empty());
        assert_eq!(backend.update_supersedes(), 0);
        assert_eq!(backend.recompute_dae(), 0);
        assert!(backend.get_communities().is_empty());
        assert_eq!(backend.run_synthesis(), 0);
    }
}
