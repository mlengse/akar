//! AFE phase: atomic fact extraction from memories.

use crate::backend::DreamBackend;
use crate::orchestrator::PhaseStats;

pub fn run_afe<B: DreamBackend>(backend: &B) -> PhaseStats {
    let mut stats = PhaseStats::default();

    // Sample memories for AFE
    let memories = backend.sample_for_dream(100, 0.5, 0.3, 0.2);
    if memories.is_empty() {
        return stats;
    }

    // Extract atomic facts
    let facts = backend.extract_afe_facts(&memories);
    stats.facts = facts.len();

    // Write facts as new memory nodes
    if !facts.is_empty() {
        backend.write_afe_facts(&facts);
    }

    stats
}
