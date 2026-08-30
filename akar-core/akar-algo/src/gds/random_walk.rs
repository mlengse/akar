use crate::AlgoResult;
use akar_graph::CSRAdjacency;

use super::rng::SimpleRng;

/// Compute random walk on a graph.
///
/// Returns the hit count for each node across all walks.
pub fn compute_random_walk(
    csr: &CSRAdjacency,
    start_node: Option<usize>,
    steps: usize,
    walks_per_node: usize,
) -> AlgoResult {
    let n = csr.num_nodes();
    let mut hit_counts = vec![0.0; n];
    let mut rng = SimpleRng::new(42);

    let start_nodes = match start_node {
        Some(node) => vec![node],
        None => (0..n).collect(),
    };

    for &start in &start_nodes {
        if start >= n {
            continue;
        }

        for _ in 0..walks_per_node {
            let mut current = start;
            hit_counts[current] += 1.0;

            for _ in 0..steps {
                let neighbors = csr.neighbors(current);
                if neighbors.is_empty() {
                    break;
                }

                let idx = rng.gen_range(neighbors.len());
                current = neighbors[idx].1.offset as usize;

                if current < n {
                    hit_counts[current] += 1.0;
                } else {
                    break;
                }
            }
        }
    }

    AlgoResult {
        name: "random_walk".into(),
        values: hit_counts,
        metadata: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_graph::Edge;

    fn csr_from_edges(edges: &[(u64, u64)], num_nodes: usize) -> CSRAdjacency {
        let edges: Vec<Edge> = edges
            .iter()
            .map(|&(src, dst)| Edge {
                src_offset: src,
                dst_offset: dst,
                rel_id: 0,
                rel_table_id: 0,
            })
            .collect();
        CSRAdjacency::build(&edges, num_nodes)
    }

    #[test]
    fn test_two_node_path_exact_hits() {
        // Every node has exactly one neighbor, so gen_range(1) always picks index 0:
        // the walk from 0 alternates 0 -> 1 -> 0 -> 1 -> 0 regardless of the RNG.
        let csr = csr_from_edges(&[(0, 1)], 2);
        let result = compute_random_walk(&csr, Some(0), 4, 1);
        assert_eq!(result.values, vec![3.0, 2.0]);
    }

    #[test]
    fn test_zero_steps_counts_only_start_visit() {
        let csr = csr_from_edges(&[(0, 1), (1, 2)], 3);
        let result = compute_random_walk(&csr, None, 0, 2);
        assert_eq!(result.values, vec![2.0, 2.0, 2.0]);
    }

    #[test]
    fn test_isolated_node_breaks_walk_immediately() {
        let csr = csr_from_edges(&[(0, 1)], 3);
        let result = compute_random_walk(&csr, Some(2), 5, 3);
        assert_eq!(result.values, vec![0.0, 0.0, 3.0]);
    }

    #[test]
    fn test_out_of_bounds_start_node_is_skipped() {
        let csr = csr_from_edges(&[(0, 1)], 2);
        let result = compute_random_walk(&csr, Some(99), 4, 2);
        assert_eq!(result.name, "random_walk");
        assert!(result.metadata.is_none());
        assert_eq!(result.values, vec![0.0, 0.0]);
    }

    #[test]
    fn test_cycle_total_hits_invariant() {
        // On a cycle no node has empty neighbors, so every step lands a hit:
        // total = nodes * walks * (steps + 1) = 4 * 3 * 8 = 96.
        let csr = csr_from_edges(&[(0, 1), (1, 2), (2, 3), (3, 0)], 4);
        let result = compute_random_walk(&csr, None, 7, 3);
        assert_eq!(result.values.len(), 4);
        let total: f64 = result.values.iter().sum();
        assert!((total - 96.0).abs() < f64::EPSILON);
        assert!(result.values.iter().all(|&v| v > 0.0));
    }

    #[test]
    fn test_hits_scale_linearly_with_walk_count() {
        let csr = csr_from_edges(&[(0, 1)], 2);
        let single = compute_random_walk(&csr, Some(0), 4, 5).values;
        let scaled = compute_random_walk(&csr, Some(0), 4, 10).values;
        let doubled: Vec<f64> = single.iter().map(|v| v * 2.0).collect();
        assert_eq!(scaled, doubled);
    }

    #[test]
    fn test_empty_graph_returns_empty_values() {
        let csr = CSRAdjacency::build(&[], 0);
        let result = compute_random_walk(&csr, None, 5, 2);
        assert!(result.values.is_empty());
        assert_eq!(result.name, "random_walk");
    }

    #[test]
    fn test_same_inputs_same_output() {
        // Branching node 1 (degree 2) exercises the RNG; seeded LCG must stay deterministic.
        let csr = csr_from_edges(&[(0, 1), (1, 2)], 3);
        let first = compute_random_walk(&csr, Some(1), 10, 8).values;
        let second = compute_random_walk(&csr, Some(1), 10, 8).values;
        assert_eq!(first, second);
        let total: f64 = first.iter().sum();
        assert!(total > 0.0);
    }
}
