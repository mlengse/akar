//! SYNTHESIS phase: merge AFE clusters into synthesis memories.

use crate::backend::DreamBackend;
use crate::orchestrator::PhaseStats;

pub fn run_synthesis<B: DreamBackend>(backend: &B) -> PhaseStats {
    PhaseStats {
        synthesized: backend.run_synthesis(),
        ..PhaseStats::default()
    }
}
