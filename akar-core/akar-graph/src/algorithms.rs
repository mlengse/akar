//! Graph algorithms for analysis and traversal.
//!
//! Implementations:
//! - BFS/DFS traversal
//! - PageRank
//! - Weakly Connected Components (WCC)
//! - Shortest path (BFS-based)

use crate::graph::CSRAdjacency;
use hashbrown::HashMap;
use std::collections::VecDeque;

/// Result of a graph algorithm containing node-level values.
#[derive(Debug, Clone)]
pub struct AlgorithmResult {
    /// Per-node values indexed by node offset.
    pub values: Vec<f64>,
    /// Label/name for the result.
    pub name: String,
}

// ==================== BFS ====================

/// BFS traversal from a source node.
/// Returns (distance, parent) for each reachable node.
pub fn bfs(csr: &CSRAdjacency, source: usize) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let n = csr.num_nodes();
    let mut distance = vec![None; n];
    let mut parent = vec![None; n];
    let mut queue = VecDeque::new();

    distance[source] = Some(0);
    queue.push_back(source);

    while let Some(node) = queue.pop_front() {
        let dist = distance[node].unwrap();
        for (_rel, dst) in csr.neighbors(node) {
            let neighbor = dst.offset as usize;
            if neighbor < n && distance[neighbor].is_none() {
                distance[neighbor] = Some(dist + 1);
                parent[neighbor] = Some(node);
                queue.push_back(neighbor);
            }
        }
    }

    (distance, parent)
}

// ==================== PageRank ====================

/// Compute PageRank for all nodes.
/// Uses iterative power method with damping factor.
pub fn page_rank(csr: &CSRAdjacency, damping: f64, max_iter: usize, tol: f64) -> AlgorithmResult {
    let n = csr.num_nodes();
    if n == 0 {
        return AlgorithmResult {
            values: vec![],
            name: "page_rank".into(),
        };
    }

    let mut pr = vec![1.0 / n as f64; n];
    let base = (1.0 - damping) / n as f64;

    for _iter in 0..max_iter {
        let mut new_pr = vec![base; n];

        // Dangling mass: total PR of nodes with no outgoing edges, distributed
        // once per iteration instead of O(n) per dangling node (P52.48).
        let dangling_mass: f64 = (0..n)
            .filter(|&i| csr.neighbors(i).is_empty())
            .map(|i| pr[i])
            .sum();

        for i in 0..n {
            let neighbors = csr.neighbors(i);
            if neighbors.is_empty() {
                continue;
            }
            let share = pr[i] / neighbors.len() as f64;
            for (_, dst) in neighbors {
                let j = dst.offset as usize;
                if j < n {
                    new_pr[j] += damping * share;
                }
            }
        }

        if dangling_mass > 0.0 {
            let dangling_share = damping * dangling_mass / n as f64;
            for val in new_pr.iter_mut() {
                *val += dangling_share;
            }
        }

        // Check convergence
        let diff: f64 = pr.iter().zip(new_pr.iter()).map(|(a, b)| (a - b).abs()).sum();
        pr = new_pr;
        if diff < tol {
            break;
        }
    }

    AlgorithmResult {
        values: pr,
        name: "page_rank".into(),
    }
}

// ==================== WCC (Weakly Connected Components) ====================

/// Find weakly connected components using union-find.
pub fn weakly_connected_components(csr: &CSRAdjacency) -> AlgorithmResult {
    let n = csr.num_nodes();
    let mut parent: Vec<usize> = (0..n).collect();

    // Union-Find: find with iterative path compression (P52.44 — the recursive
    // variant overflowed the stack on deep chains).
    fn find(parent: &mut [usize], x: usize) -> usize {
        let mut root = x;
        while parent[root] != root {
            root = parent[root];
        }
        let mut cur = x;
        while parent[cur] != root {
            let next = parent[cur];
            parent[cur] = root;
            cur = next;
        }
        root
    }

    // Union
    fn union(parent: &mut [usize], a: usize, b: usize) {
        let ra = find(parent, a);
        let rb = find(parent, b);
        if ra != rb {
            parent[ra] = rb;
        }
    }

    for i in 0..n {
        for (_, dst) in csr.neighbors(i) {
            let j = dst.offset as usize;
            if j < n {
                union(&mut parent, i, j);
            }
        }
    }

    // Path compression for all
    for i in 0..n {
        find(&mut parent, i);
    }

    // Assign component IDs
    let mut comp_id: HashMap<usize, usize> = HashMap::new();
    let values: Vec<f64> = parent
        .iter()
        .map(|&p| {
            let len = comp_id.len();
            *comp_id.entry(p).or_insert(len) as f64
        })
        .collect();

    AlgorithmResult {
        values,
        name: "wcc".into(),
    }
}

// ==================== Shortest Path ====================

/// BFS-based shortest path distance between two nodes.
pub fn shortest_path(csr: &CSRAdjacency, source: usize, target: usize) -> Option<usize> {
    let (distance, _parent) = bfs(csr, source);
    distance.get(target).copied().flatten()
}

/// All-pairs reachable nodes within a given radius using BFS.
pub fn reachable_within(csr: &CSRAdjacency, source: usize, max_dist: usize) -> Vec<usize> {
    let (distance, _) = bfs(csr, source);
    distance
        .iter()
        .enumerate()
        .filter(|&(_, d)| d.is_some() && d.unwrap() <= max_dist)
        .map(|(i, _)| i)
        .collect()
}

// ==================== Degree Centrality ====================

/// Compute degree centrality for all nodes.
pub fn degree_centrality(csr: &CSRAdjacency) -> AlgorithmResult {
    let n = csr.num_nodes();
    let values: Vec<f64> = (0..n).map(|i| csr.neighbors(i).len() as f64).collect();

    AlgorithmResult {
        values,
        name: "degree_centrality".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Edge;

    fn sample_csr() -> CSRAdjacency {
        let edges = vec![
            Edge {
                src_offset: 0,
                dst_offset: 1,
                rel_id: 0,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 0,
                dst_offset: 2,
                rel_id: 1,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 1,
                dst_offset: 2,
                rel_id: 2,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 2,
                dst_offset: 3,
                rel_id: 3,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 3,
                dst_offset: 0,
                rel_id: 4,
                rel_table_id: 0,
            },
        ];
        CSRAdjacency::build(&edges, 4)
    }

    #[test]
    fn test_bfs() {
        let csr = sample_csr();
        let (dist, _parent) = bfs(&csr, 0);
        assert_eq!(dist[0], Some(0)); // Self distance 0
        assert_eq!(dist[1], Some(1)); // Direct neighbor
        assert_eq!(dist[2], Some(1)); // Direct neighbor
        assert_eq!(dist[3], Some(1)); // Direct edge: 3→0
    }

    #[test]
    fn test_page_rank_basic() {
        let csr = sample_csr();
        let result = page_rank(&csr, 0.85, 20, 1e-6);
        assert_eq!(result.values.len(), 4);
        // All scores should be positive
        for &v in &result.values {
            assert!(v > 0.0);
            assert!(v < 1.0);
        }
        // Sum should be approximately 1
        let sum: f64 = result.values.iter().sum();
        assert!((sum - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_wcc() {
        let csr = sample_csr();
        let result = weakly_connected_components(&csr);
        assert_eq!(result.values.len(), 4);
        // All nodes are in the same component (the graph is connected)
        assert!((result.values[0] - result.values[1]).abs() < 1e-10);
        assert!((result.values[1] - result.values[2]).abs() < 1e-10);
        assert!((result.values[2] - result.values[3]).abs() < 1e-10);
    }

    #[test]
    fn test_wcc_disconnected() {
        // Two disconnected triangles
        let edges = vec![
            Edge {
                src_offset: 0,
                dst_offset: 1,
                rel_id: 0,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 1,
                dst_offset: 2,
                rel_id: 1,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 3,
                dst_offset: 4,
                rel_id: 2,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 4,
                dst_offset: 5,
                rel_id: 3,
                rel_table_id: 0,
            },
        ];
        let csr = CSRAdjacency::build(&edges, 6);
        let result = weakly_connected_components(&csr);
        // Nodes 0,1,2 are in one component; 3,4,5 in another
        assert!((result.values[0] - result.values[1]).abs() < 1e-10);
        assert!((result.values[1] - result.values[2]).abs() < 1e-10);
        assert!((result.values[3] - result.values[4]).abs() < 1e-10);
        assert!((result.values[4] - result.values[5]).abs() < 1e-10);
        assert!((result.values[0] - result.values[3]).abs() >= 1e-10);
    }

    #[test]
    fn test_shortest_path() {
        let csr = sample_csr();
        let dist = shortest_path(&csr, 0, 3);
        assert_eq!(dist, Some(1)); // Direct edge: 3→0
    }

    #[test]
    fn test_shortest_path_same_node() {
        let csr = sample_csr();
        let dist = shortest_path(&csr, 0, 0);
        assert_eq!(dist, Some(0));
    }

    #[test]
    fn test_reachable_within() {
        let csr = sample_csr();
        let reachable = reachable_within(&csr, 0, 1);
        // Nodes 0, 1, 2 are within distance 1 (0 itself + direct neighbors)
        assert!(reachable.contains(&0));
        assert!(reachable.contains(&1));
        assert!(reachable.contains(&2));
        assert!(reachable.contains(&3)); // Node 3 is distance 1 (direct edge)
    }

    #[test]
    fn test_degree_centrality() {
        let csr = sample_csr();
        let result = degree_centrality(&csr);
        assert_eq!(result.values.len(), 4);
        // Node 0 has degree 3 (connected to 1, 2, 3)
        assert!((result.values[0] - 3.0).abs() < 1e-10);
        // Node 3 has degree 2 (connected to 2, 0)
        assert!((result.values[3] - 2.0).abs() < 1e-10);
    }

    #[test]
    fn test_empty_graph_algorithms() {
        let csr = CSRAdjacency::new(0);
        let pr = page_rank(&csr, 0.85, 20, 1e-6);
        assert!(pr.values.is_empty());

        let wcc = weakly_connected_components(&csr);
        assert!(wcc.values.is_empty());
    }

    #[test]
    fn test_wcc_deep_chain_no_stack_overflow() {
        // P52.44: the recursive union-find find() overflowed the stack on a
        // deep chain (200k nodes, 0→1→2→…). Iterative find must handle it.
        let n: usize = 200_000;
        let edges: Vec<Edge> = (0..n - 1)
            .map(|i| Edge {
                src_offset: i as u64,
                dst_offset: (i + 1) as u64,
                rel_id: i as u64,
                rel_table_id: 0,
            })
            .collect();
        let csr = CSRAdjacency::build(&edges, n);
        let result = weakly_connected_components(&csr);
        assert_eq!(result.values.len(), n);
        assert!((result.values[0] - result.values[n - 1]).abs() < 1e-10);
    }
}
