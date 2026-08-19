//! PyO3 bindings for spread activation (akar-algo).

use pyo3::prelude::*;
use std::collections::HashMap;

use akar_algo::{compute_spread_activation, batch_spread_activation};
use akar_graph::graph::{CSRAdjacency, Edge};

/// Build a CSR adjacency list from a Python edge list.
fn build_csr(edges: &[(usize, usize)], num_nodes: usize) -> CSRAdjacency {
    let graph_edges: Vec<Edge> = edges
        .iter()
        .enumerate()
        .map(|(i, &(src, dst))| Edge {
            src_offset: src as u64,
            dst_offset: dst as u64,
            rel_id: i as u64,
            rel_table_id: 0,
        })
        .collect();
    CSRAdjacency::build(&graph_edges, num_nodes)
}

/// Run spread activation on a graph.
///
/// - `edges`: list of `(src, dst)` tuples (undirected — both directions added).
/// - `num_nodes`: total number of nodes.
/// - `seeds`: list of `(node_position, initial_activation)` tuples.
/// - `decay`: decay factor per hop (default 0.5).
/// - `threshold`: minimum activation to propagate (default 0.01).
/// - `max_hops`: maximum propagation depth (default 3).
/// - `weights`: optional `{(src, dst): float}` edge weight map.
///
/// Returns list of `(node_position, activation_score, hop_reached)` tuples.
#[pyfunction]
#[pyo3(signature = (edges, num_nodes, seeds, decay=0.5, threshold=0.01, max_hops=3, weights=None))]
fn spread_activation(
    edges: Vec<(usize, usize)>,
    num_nodes: usize,
    seeds: Vec<(usize, f64)>,
    decay: f64,
    threshold: f64,
    max_hops: usize,
    weights: Option<HashMap<(usize, usize), f64>>,
) -> PyResult<Vec<(usize, f64, usize)>> {
    let csr = build_csr(&edges, num_nodes);

    let result = if let Some(w) = weights {
        compute_spread_activation(
            &csr,
            &seeds,
            |u, v| *w.get(&(u, v)).unwrap_or(&1.0),
            decay,
            threshold,
            max_hops,
        )
    } else {
        compute_spread_activation(&csr, &seeds, |_u, _v| 1.0, decay, threshold, max_hops)
    };

    Ok(result.activated)
}

/// Run batch spread activation on a graph.
///
/// Builds the CSR adjacency once, then runs BFS from every seed independently.
/// Much faster than N individual `spread_activation()` calls when the graph
/// fits in memory.
///
/// - `edges`: list of `(src, dst)` tuples.
/// - `num_nodes`: total number of nodes.
/// - `start_ids`: list of node positions to spread from.
/// - `depth`: max BFS depth per seed (default 1).
/// - `decay`: decay factor per hop (default 0.85).
/// - `threshold`: minimum activation to propagate (default 0.01).
/// - `k_per_seed`: max results per seed (default 20).
///
/// Returns a dict mapping each seed ID → list of `(node_position, activation, hop)` tuples.
#[pyfunction]
#[pyo3(signature = (edges, num_nodes, start_ids, depth=1, decay=0.85, threshold=0.01, k_per_seed=20))]
fn batch_spread(
    edges: Vec<(usize, usize)>,
    num_nodes: usize,
    start_ids: Vec<usize>,
    depth: usize,
    decay: f64,
    threshold: f64,
    k_per_seed: usize,
) -> PyResult<HashMap<usize, Vec<(usize, f64, usize)>>> {
    let seed_positions: Vec<(usize, f64)> = start_ids.iter().map(|&id| (id, 1.0)).collect();

    Ok(batch_spread_activation(
        &edges,
        num_nodes,
        &seed_positions,
        decay,
        threshold,
        depth,
        k_per_seed,
    ))
}

/// Register this submodule on the parent `akar` module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "spread")?;
    sub.add_function(wrap_pyfunction!(spread_activation, &sub)?)?;
    sub.add_function(wrap_pyfunction!(batch_spread, &sub)?)?;
    m.add_submodule(&sub)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spread_basic() {
        // 0 -> 1 -> 2
        let edges = vec![(0, 1), (1, 2)];
        let seeds = vec![(0, 1.0)];
        let result = spread_activation(edges, 3, seeds, 0.5, 0.01, 3, None).unwrap();
        assert!(!result.is_empty());
        // Node 1 should be activated at hop 1
        let n1 = result.iter().find(|(id, _, _)| *id == 1);
        assert!(n1.is_some());
    }

    #[test]
    fn test_spread_with_weights() {
        let edges = vec![(0, 1), (1, 2)];
        let seeds = vec![(0, 1.0)];
        let weights = vec![((0, 1), 2.0), ((1, 0), 2.0)].into_iter().collect();
        let result = spread_activation(edges, 3, seeds, 0.5, 0.01, 2, Some(weights)).unwrap();
        let n1 = result.iter().find(|(id, _, _)| *id == 1).unwrap();
        assert!((n1.1 - 1.0).abs() < 1e-10); // 1.0 * 2.0 * 0.5 = 1.0
    }

    #[test]
    fn test_spread_empty() {
        let result = spread_activation(vec![], 0, vec![], 0.5, 0.01, 3, None).unwrap();
        assert!(result.is_empty());
    }
}
