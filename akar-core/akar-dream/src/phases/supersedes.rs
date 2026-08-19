//! SUPERSEDES phase: mark edges superseded by newer ones.

use crate::backend::DreamBackend;
use crate::orchestrator::PhaseStats;

pub fn run_supersedes<B: DreamBackend>(backend: &B) -> PhaseStats {
    let mut stats = PhaseStats::default();
    stats.strengthened = backend.update_supersedes();
    stats
}
