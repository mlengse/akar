//! SYNTHESIS phase: merge AFE clusters into synthesis memories.

use crate::backend::DreamBackend;
use crate::orchestrator::PhaseStats;

pub fn run_synthesis<B: DreamBackend>(backend: &B) -> PhaseStats {
    let mut stats = PhaseStats::default();
    stats.synthesized = backend.run_synthesis();
    stats
}
