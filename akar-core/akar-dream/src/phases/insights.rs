//! INSIGHT phase: Louvain community detection + write assignments.

use crate::backend::DreamBackend;
use crate::config::DreamConfig;
use crate::orchestrator::PhaseStats;

pub fn run_insights<B: DreamBackend>(backend: &B, _config: &DreamConfig) -> PhaseStats {
    let mut stats = PhaseStats::default();

    // Get community assignments via Louvain
    let assignments = backend.get_communities();
    if assignments.is_empty() {
        return stats;
    }

    // Count unique communities
    let unique: std::collections::HashSet<usize> = assignments.iter().cloned().collect();
    stats.insights = unique.len();

    // Write assignments to storage
    backend.write_communities(&assignments);

    stats
}
