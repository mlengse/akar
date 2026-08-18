//! PyO3 bindings for Louvain community detection (akar-algo).

use pyo3::prelude::*;
use std::collections::HashMap;

use akar_algo::{compute_louvain, compute_louvain_weighted};
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

/// Run Louvain community detection (unweighted).
///
/// - `edges`: list of `(src, dst)` tuples.
/// - `num_nodes`: total number of nodes.
///
/// Returns list of community IDs (as floats, one per node position).
#[pyfunction]
fn louvain(edges: Vec<(usize, usize)>, num_nodes: usize) -> PyResult<Vec<f64>> {
    let csr = build_csr(&edges, num_nodes);
    let result = compute_louvain(&csr);
    Ok(result.values)
}

/// Run weighted Louvain community detection.
///
/// - `edges`: list of `(src, dst)` tuples.
/// - `num_nodes`: total number of nodes.
/// - `weights`: `{(src, dst): float}` edge weight map.
/// - `min_gain`: minimum modularity gain threshold (default 0.001).
/// - `max_iterations`: maximum passes (default 20).
///
/// Returns `(community_ids, modularity)`.
#[pyfunction]
#[pyo3(signature = (edges, num_nodes, weights, min_gain=0.001, max_iterations=20))]
fn louvain_weighted(
    edges: Vec<(usize, usize)>,
    num_nodes: usize,
    weights: HashMap<(usize, usize), f64>,
    min_gain: f64,
    max_iterations: usize,
) -> PyResult<(Vec<f64>, f64)> {
    let csr = build_csr(&edges, num_nodes);
    let result = compute_louvain_weighted(
        &csr,
        |u, v| *weights.get(&(u, v)).unwrap_or(&1.0),
        None,
        min_gain,
        max_iterations,
    );
    let modularity = result
        .metadata
        .as_ref()
        .and_then(|m| m.get("modularity").copied())
        .unwrap_or(0.0);
    Ok((result.values, modularity))
}

/// Register this submodule on the parent `akar` module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "louvain")?;
    sub.add_function(wrap_pyfunction!(louvain, &sub)?)?;
    sub.add_function(wrap_pyfunction!(louvain_weighted, &sub)?)?;
    m.add_submodule(&sub)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_louvain_basic() {
        // Triangle + isolated node
        let edges = vec![(0, 1), (1, 2), (2, 0)];
        let result = louvain(edges, 4).unwrap();
        assert_eq!(result.len(), 4);
        // Nodes 0,1,2 should share a community
        assert!((result[0] - result[1]).abs() < 1e-10);
        assert!((result[1] - result[2]).abs() < 1e-10);
    }

    #[test]
    fn test_louvain_empty() {
        let result = louvain(vec![], 0).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_louvain_weighted_basic() {
        let edges = vec![(0, 1), (1, 2), (2, 0)];
        let weights: HashMap<(usize, usize), f64> = edges
            .iter()
            .flat_map(|&(s, d)| [((s, d), 1.0), ((d, s), 1.0)])
            .collect();
        let (communities, _mod) = louvain_weighted(edges, 4, weights, 0.001, 20).unwrap();
        assert_eq!(communities.len(), 4);
    }
}
