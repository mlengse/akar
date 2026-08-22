//! REM phase: bridge discovery between isolated communities.

use crate::backend::DreamBackend;
use crate::config::DreamConfig;
use crate::orchestrator::PhaseStats;

pub fn run_rem<B: DreamBackend>(backend: &B, config: &DreamConfig) -> PhaseStats {
    let mut stats = PhaseStats::default();

    // Get community assignments
    let communities_raw = backend.get_communities();
    if communities_raw.is_empty() {
        return stats;
    }

    // Group nodes by community
    let mut communities: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    for (node_id, &comm_id) in communities_raw.iter().enumerate() {
        communities.entry(comm_id).or_default().push(node_id);
    }

    // Filter small communities (isolated)
    let isolated: Vec<Vec<usize>> = communities.values().filter(|nodes| nodes.len() < 5).cloned().collect();

    if isolated.len() < 2 {
        return stats;
    }

    // Placeholder: find bridges using centroid cosine
    // In production, this would use actual embeddings from the backend
    let embeddings: Vec<[f64; 384]> = Vec::new();
    let bridges = backend.find_bridges(&isolated, &embeddings, config.max_bridge_nodes);

    stats.bridges = bridges.len();
    if !bridges.is_empty() {
        backend.create_bridge_edges(&bridges);
    }

    stats
}
