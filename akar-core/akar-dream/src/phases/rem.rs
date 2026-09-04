//! REM phase: bridge discovery between isolated communities.

#[cfg(feature = "embed")]
use crate::EmbeddingProvider;
use crate::backend::DreamBackend;
use crate::config::DreamConfig;
use crate::orchestrator::PhaseStats;

pub fn run_rem<B: DreamBackend>(
    backend: &B,
    config: &DreamConfig,
    #[cfg(feature = "embed")] embedding: Option<&dyn EmbeddingProvider>,
) -> PhaseStats {
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

    // Compute embeddings for bridge discovery
    #[cfg(feature = "embed")]
    let embeddings: Vec<[f64; 384]> = if let Some(provider) = embedding {
        // Collect all node IDs from isolated communities
        let all_node_ids: Vec<usize> = isolated.iter().flat_map(|nodes| nodes.iter().copied()).collect();

        // TODO: fetch actual node content from backend and embed
        // For now, the provider is available but content fetching requires
        // a new DreamBackend method (deferred to P89.5)
        let _ = (provider, &all_node_ids);
        Vec::new()
    } else {
        Vec::new()
    };
    #[cfg(not(feature = "embed"))]
    let embeddings: Vec<[f64; 384]> = Vec::new();

    let bridges = backend.find_bridges(&isolated, &embeddings, config.max_bridge_nodes);

    stats.bridges = bridges.len();
    if !bridges.is_empty() {
        backend.create_bridge_edges(&bridges);
    }

    stats
}
