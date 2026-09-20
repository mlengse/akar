//! NREM phase: spread activation → strengthen → weaken → prune.

use crate::backend::{DreamBackend, Memory};
use crate::config::DreamConfig;
use crate::orchestrator::PhaseStats;
use std::collections::HashMap;

/// Seconds in a day, for converting unix timestamps to Ebbinghaus age.
const SECONDS_PER_DAY: f64 = 86_400.0;

/// Retention of an endpoint whose memory the backend did not return.
///
/// The NREM phase samples memories but weakens *all* edges, so most endpoints
/// are unknown. No evidence either way has to mean "decayed at the full base
/// rate" — the behaviour this phase had before retention scoring existed —
/// rather than silently exempting unsampled edges from decay.
const UNKNOWN_ENDPOINT_RETENTION: f64 = 0.0;

/// Retention of one memory, given the observables the dream backend carries.
///
/// `access_count`, `days_since_access` and `distinct_actors` are not tracked by
/// [`DreamBackend`] yet, so age stands in for the forgetting interval: the
/// approximation is "not recalled since it was created". That under-states
/// nothing important here — NREM strengthens edges it just activated, and only
/// the *residual* decay of everything else depends on this estimate, so an
/// approximation shifts the decay curve rather than the reachability decision.
fn memory_retention(memory: &Memory, now: f64) -> f64 {
    let age_days = ((now - memory.created_at) / SECONDS_PER_DAY).max(0.0);
    akar_function::retention_score(age_days, 0.0, age_days, memory.salience, 0.0, 0.0)
}

/// How much to weaken an edge this cycle.
///
/// Replaces the flat constant the phase used before P119.2. An edge is as
/// retained as its **best** endpoint: a connection to a memory the agent keeps
/// using should not decay just because its other end is stale. The result is
/// always in `[0, nrem_weaken_rate]` — retention can only ever slow decay down,
/// never speed it up, so this change cannot make the graph decay faster than
/// the old constant did.
fn edge_decay(
    config: &DreamConfig,
    sampled: &HashMap<usize, &Memory>,
    source_id: usize,
    target_id: usize,
    now: f64,
) -> f64 {
    let retention = [source_id, target_id]
        .into_iter()
        .map(|id| {
            sampled
                .get(&id)
                .map_or(UNKNOWN_ENDPOINT_RETENTION, |m| memory_retention(m, now))
        })
        .fold(0.0_f64, f64::max);

    config.nrem_weaken_rate * (1.0 - retention).max(0.0)
}

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

    // Retention is scored against the sampled set, so it is computed once per
    // phase rather than per edge (P119.2).
    let sampled: HashMap<usize, &Memory> = memories.iter().map(|m| (m.id, m)).collect();

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
        for &(node_id, _, hop) in activated {
            // Skip the seed itself. It is trivially "activated" by definition,
            // and since every sampled memory is also a seed, counting it would
            // mark the *entire* 1-hop neighbourhood of every sampled memory as
            // reinforced. Marking only edges the activation actually propagated
            // to (hop >= 1) is what makes the strengthen/weaken split mean
            // something — and it is a precondition for retention scoring
            // (P119.2), which otherwise could never see an edge with a sampled
            // endpoint.
            if hop == 0 {
                continue;
            }
            // We don't know which of this node's edges the activation travelled
            // through, so mark all of them.
            for conn in &connections {
                if conn.source_id == node_id || conn.target_id == node_id {
                    activated_edges.insert((conn.source_id.min(conn.target_id), conn.source_id.max(conn.target_id)));
                }
            }
        }
    }

    // Strengthen activated edges, weaken non-activated, prune below threshold
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);

    for conn in &connections {
        let edge_key = (conn.source_id.min(conn.target_id), conn.source_id.max(conn.target_id));

        if activated_edges.contains(&edge_key) {
            backend.strengthen_edge(conn.source_id, conn.target_id, 0.1);
            stats.strengthened += 1;
        } else if conn.weight < config.prune_threshold {
            backend.prune_edge(conn.source_id, conn.target_id);
            stats.pruned += 1;
        } else {
            let decay = edge_decay(config, &sampled, conn.source_id, conn.target_id, now);
            backend.weaken_edge(conn.source_id, conn.target_id, decay);
            stats.weakened += 1;
        }
    }

    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Edge, MockBackend};

    fn memory(id: usize, salience: f64, age_days: f64, now: f64) -> Memory {
        Memory {
            id,
            salience,
            created_at: now - age_days * SECONDS_PER_DAY,
            content: String::new(),
        }
    }

    fn now() -> f64 {
        1_800_000_000.0
    }

    fn decay_for(endpoints: [Option<(f64, f64)>; 2]) -> f64 {
        let config = DreamConfig::default();
        let owned: Vec<Memory> = endpoints
            .into_iter()
            .enumerate()
            .filter_map(|(i, e)| e.map(|(sal, age)| memory(i, sal, age, now())))
            .collect();
        let sampled: HashMap<usize, &Memory> = owned.iter().map(|m| (m.id, m)).collect();
        edge_decay(&config, &sampled, 0, 1, now())
    }

    #[test]
    fn unknown_endpoints_decay_at_the_full_base_rate() {
        // The pre-P119 behaviour, preserved whenever there is no information.
        let decay = decay_for([None, None]);
        assert!((decay - 0.05).abs() < 1e-12, "expected the full base rate, got {decay}");
    }

    #[test]
    fn a_just_created_salient_memory_barely_decays() {
        let decay = decay_for([Some((1.0, 0.0)), None]);
        assert!(decay < 0.05, "retention must slow the decay, got {decay}");
        assert!(decay >= 0.0, "decay must never go negative, got {decay}");
        assert!((decay - 0.0).abs() < 1e-9, "a fresh memory is fully retained: {decay}");
    }

    #[test]
    fn decay_grows_as_retention_falls() {
        // Ages are kept in the unsaturated range: past a few half-lives every
        // score is ~0, so differences between memories vanish by construction.
        let fresh = decay_for([Some((1.0, 0.0)), None]);
        let day_old = decay_for([Some((1.0, 1.0)), None]);
        let few_days_old = decay_for([Some((1.0, 3.0)), None]);
        assert!(fresh < day_old, "{fresh} vs {day_old}");
        assert!(day_old < few_days_old, "{day_old} vs {few_days_old}");
        assert!(few_days_old <= 0.05, "never faster than the base rate");

        // Low salience decays faster at the same age.
        let unsalient = decay_for([Some((0.0, 1.0)), None]);
        assert!(
            day_old < unsalient,
            "salience must slow the decay: {day_old} vs {unsalient}"
        );

        // Fully forgotten observations decay exactly at the base rate.
        let long_gone = decay_for([Some((1.0, 10_000.0)), None]);
        assert!((long_gone - 0.05).abs() < 1e-9, "{long_gone}");
    }

    #[test]
    fn the_best_endpoint_decides() {
        // One stale end must not drag down an edge that the other end keeps alive.
        let both_stale = decay_for([Some((1.0, 365.0)), Some((1.0, 365.0))]);
        let one_fresh = decay_for([Some((1.0, 0.0)), Some((1.0, 365.0))]);
        assert!(one_fresh < both_stale, "{one_fresh} vs {both_stale}");
        assert!((one_fresh - 0.0).abs() < 1e-9, "the fresh end wins: {one_fresh}");
    }

    /// Two stars, each with far more neighbours than `k_per_seed`, so the
    /// activation truncation leaves most edges untouched. Those edges still have
    /// a sampled endpoint — the seed — which is the only situation in which the
    /// retention term can differ from the base rate.
    fn two_star_backend(now: f64) -> MockBackend {
        let backend = MockBackend::new();
        backend.memories.borrow_mut().push(memory(0, 1.0, 0.0, now));
        backend.memories.borrow_mut().push(memory(1, 0.0, 10_000.0, now));
        for target in 2..=61usize {
            backend.edges.borrow_mut().push(Edge {
                source_id: 0,
                target_id: target,
                weight: 0.9,
            });
        }
        for target in 62..=121usize {
            backend.edges.borrow_mut().push(Edge {
                source_id: 1,
                target_id: target,
                weight: 0.9,
            });
        }
        backend
    }

    #[test]
    fn nrem_decays_untraversed_edges() {
        // Wire-level check: the phase weakens the edges activation did not
        // reach, even though those edges have a sampled endpoint.
        //
        // This is the behaviour the hop-0 skip buys: before it, every seed
        // marked its whole neighbourhood as activated (a seed is trivially
        // "activated" by itself) and no edge with a sampled endpoint ever
        // reached the decay branch.
        let backend = two_star_backend(now());
        let stats = run_nrem(&backend, &DreamConfig::default());
        assert!(stats.weakened > 0, "untraversed edges must decay: {stats:?}");
        assert!(
            stats.strengthened > 0,
            "reached edges must still be reinforced: {stats:?}"
        );
        assert_eq!(stats.pruned, 0, "weight 0.9 is above the prune threshold: {stats:?}");

        let amounts = backend.weaken_amounts.borrow();
        assert_eq!(amounts.len(), stats.weakened);
        assert!(
            amounts.iter().all(|a| (0.0..=0.05).contains(a)),
            "decay must stay within [0, base rate]: {amounts:?}"
        );
    }

    #[test]
    fn nrem_applies_retention_derived_decay() {
        // Same wiring, but the whole point of P119.2: the amount handed to the
        // backend tracks the retention of the edge's best endpoint. The fresh,
        // salient seed's untraversed tail is barely touched; the stale, unsalient
        // seed's tail decays at the full base rate.
        let backend = two_star_backend(now());
        run_nrem(&backend, &DreamConfig::default());

        let amounts = backend.weaken_amounts.borrow();
        assert!(!amounts.is_empty(), "expected untraversed edges to decay");
        assert!(
            amounts.iter().any(|a| *a < 1e-9),
            "the fresh endpoint keeps its edges alive: {amounts:?}"
        );
        assert!(
            amounts.iter().any(|a| (*a - 0.05).abs() < 1e-9),
            "the stale endpoint decays at the full base rate: {amounts:?}"
        );
    }

    #[test]
    fn activation_outranks_decay_for_a_seeds_direct_neighbour() {
        // Pins the interaction the retention term has to live with: the phase
        // marks every edge touching an activated node, so a single-neighbour
        // seed edge is strengthened, never weakened.
        let now = now();
        let backend = MockBackend::new();
        backend.memories.borrow_mut().push(memory(0, 0.1, 5_000.0, now));
        backend.edges.borrow_mut().push(Edge {
            source_id: 0,
            target_id: 1,
            weight: 0.9,
        });

        let stats = run_nrem(&backend, &DreamConfig::default());
        assert_eq!(stats.strengthened, 1, "{stats:?}");
        assert_eq!(stats.weakened, 0, "{stats:?}");
        assert!(backend.weaken_amounts.borrow().is_empty());
    }
}
