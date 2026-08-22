//! NREM phase: spread activation → strengthen → weaken → prune.

use crate::backend::DreamBackend;
use crate::config::DreamConfig;
use crate::orchestrator::PhaseStats;

pub fn run_nrem<B: DreamBackend>(backend: &B, config: &DreamConfig) -> PhaseStats {
    let mut stats = PhaseStats::default();

    // Sample memories
    let memories = backend.sample_for_dream(
        config.max_memories,
        config.sample_recent_pct,
        config.sample_random_old_pct,
        config.sample_low_salience_pct,
    );

    if memories.is_empty() {
        return stats;
    }

    // Get all edges
    let connections = backend.get_connections();
    if connections.is_empty() {
        return stats;
    }

    // Build edge list for batch spread activation
    let edges: Vec<(usize, usize)> = connections.iter().map(|e| (e.source_id, e.target_id)).collect();

    let num_nodes = connections
        .iter()
        .map(|e| e.source_id.max(e.target_id) + 1)
        .max()
        .unwrap_or(0);

    let seed_positions: Vec<(usize, f64)> = memories.iter().map(|m| (m.id, 1.0)).collect();

    // Run batch spread activation
    let batch_results = akar_algo::batch_spread_activation(
        &edges,
        num_nodes,
        &seed_positions,
        config.decay,
        config.threshold,
        config.max_hops,
        config.k_per_seed,
    );

    // Collect all activated edges
    let mut activated_edges: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    for activated in batch_results.values() {
        for &(node_id, _, _) in activated {
            // Mark edge from seed to activated node
            // (we don't know which seed→node edge was used, so mark all)
            for conn in &connections {
                if conn.source_id == node_id || conn.target_id == node_id {
                    activated_edges.insert((conn.source_id.min(conn.target_id), conn.source_id.max(conn.target_id)));
                }
            }
        }
    }

    // Strengthen activated edges, weaken non-activated, prune below threshold
    for conn in &connections {
        let edge_key = (conn.source_id.min(conn.target_id), conn.source_id.max(conn.target_id));

        if activated_edges.contains(&edge_key) {
            backend.strengthen_edge(conn.source_id, conn.target_id, 0.1);
            stats.strengthened += 1;
        } else if conn.weight < config.prune_threshold {
            backend.prune_edge(conn.source_id, conn.target_id);
            stats.pruned += 1;
        } else {
            backend.weaken_edge(conn.source_id, conn.target_id, 0.05);
            stats.weakened += 1;
        }
    }

    stats
}
