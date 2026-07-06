//! GDS execution utilities.
//!
//! Ported from C++ `gds_utils.h` / `gds_utils.cpp`.
//!
//! Provides the main execution orchestrator for graph algorithms:
//! - `run_edge_compute`: Iterative BFS using frontier pairs
//! - `run_vertex_compute`: Single-pass over all vertices
//!
//! Uses rayon for parallel execution.

use kuzu_common::types::InternalID;

use crate::gds::compute::EdgeCompute;
use crate::gds::frontier::FrontierPair;
use crate::graph::CSRAdjacency;

/// GDS execution utilities.
pub struct GDSUtils;

impl GDSUtils {
    /// Run iterative BFS-like computation on a CSR adjacency graph.
    ///
    /// Returns the number of iterations executed.
    #[allow(dead_code)]
    pub fn run_edge_compute(
        graph: &CSRAdjacency,
        source: u64,
        frontier_pair: &mut dyn FrontierPair,
        edge_compute: &mut dyn EdgeCompute,
        max_iteration: u64,
    ) -> u64 {
        let n = graph.num_nodes();

        // Initialize: mark source as active in iteration 0
        if (source as usize) < n {
            frontier_pair.add_node_to_next_frontier_offset(source);
            frontier_pair.set_active_nodes_for_next_iter();
        }

        let mut num_iters = 0u64;

        while frontier_pair.continue_next_iter(max_iteration as u16) {
            frontier_pair.begin_new_iteration();

            // Get active nodes on current frontier
            let active_nodes: Vec<u64> = (0..n)
                .filter(|&i| frontier_pair.is_active_on_current_frontier(i as u64))
                .map(|i| i as u64)
                .collect();

            if active_nodes.is_empty() {
                break;
            }

            // Process active nodes — sequential for simplicity (avoids &mut issues)
            let mut added = Vec::new();
            for &node_offset in &active_nodes {
                let bound_node_id = InternalID {
                    table_id: 0,
                    offset: node_offset,
                };
                let neighbors = graph.neighbors(node_offset as usize);

                for &(edge_id, ref dst) in neighbors {
                    if edge_compute.edge_compute(bound_node_id, *dst, edge_id, true) {
                        added.push(dst.offset);
                    }
                }
            }

            // Add all activated neighbors to the next frontier
            for &offset in &added {
                frontier_pair.add_node_to_next_frontier_offset(offset);
            }

            if !added.is_empty() {
                frontier_pair.set_active_nodes_for_next_iter();
            }

            num_iters += 1;

            // Check early termination
            if edge_compute.terminate() {
                break;
            }
        }

        num_iters
    }

    /// Run the shortest path BFS with path tracking.
    ///
    /// Returns a map from destination offsets to their parent lists.
    pub fn run_single_shortest_path(
        graph: &CSRAdjacency,
        source: u64,
        bfs_graph: &mut dyn crate::gds::bfs_graph::BaseBFSGraph,
        max_iteration: u16,
    ) -> u64 {
        let _n = graph.num_nodes() as u64;

        let mut cur_frontier: Vec<u64> = vec![source];
        let mut next_frontier: Vec<u64> = Vec::new();
        let mut visited = vec![false; graph.num_nodes()];
        if (source as usize) < graph.num_nodes() {
            visited[source as usize] = true;
        }
        let mut iter = 0u16;

        while !cur_frontier.is_empty() && iter < max_iteration {
            next_frontier.clear();

            for &node_offset in &cur_frontier {
                let bound_node_id = InternalID {
                    table_id: 0,
                    offset: node_offset,
                };
                let neighbors = graph.neighbors(node_offset as usize);

                for &(edge_id, ref dst) in neighbors {
                    let nbr_offset = dst.offset as usize;
                    if nbr_offset >= graph.num_nodes() {
                        continue;
                    }

                    // First visit = shortest path in BFS
                    if !visited[nbr_offset] {
                        visited[nbr_offset] = true;
                        bfs_graph.add_single_parent(iter + 1, bound_node_id, edge_id, *dst, true);
                        next_frontier.push(dst.offset);
                    }
                }
            }

            std::mem::swap(&mut cur_frontier, &mut next_frontier);
            iter += 1;
        }

        iter as u64
    }

    /// Run all shortest paths BFS with path tracking.
    ///
    /// Returns the number of iterations executed.
    pub fn run_all_shortest_paths(
        graph: &CSRAdjacency,
        source: u64,
        bfs_graph: &mut dyn crate::gds::bfs_graph::BaseBFSGraph,
        max_iteration: u16,
    ) -> u64 {
        let n = graph.num_nodes() as u64;

        // Track which iteration each node was first discovered in
        let mut discovered_at: Vec<Option<u16>> = vec![None; graph.num_nodes()];

        if (source as usize) < graph.num_nodes() {
            discovered_at[source as usize] = Some(0);
        }

        let mut cur_frontier: Vec<u64> = vec![source];
        let mut next_frontier: Vec<u64> = Vec::new();
        let mut iter = 0u16;

        while !cur_frontier.is_empty() && iter < max_iteration {
            next_frontier.clear();

            for &node_offset in &cur_frontier {
                let bound_node_id = InternalID {
                    table_id: 0,
                    offset: node_offset,
                };
                let neighbors = graph.neighbors(node_offset as usize);

                for &(edge_id, ref dst) in neighbors {
                    let nbr_offset = dst.offset;
                    if nbr_offset >= n {
                        continue;
                    }

                    // First visit: record and add to next frontier
                    if discovered_at[nbr_offset as usize].is_none() {
                        discovered_at[nbr_offset as usize] = Some(iter + 1);
                        bfs_graph.add_parent(iter + 1, bound_node_id, edge_id, *dst, true);
                        next_frontier.push(nbr_offset);
                    } else if discovered_at[nbr_offset as usize] == Some(iter + 1) {
                        // Same iteration: add as alternative parent
                        bfs_graph.add_parent(iter + 1, bound_node_id, edge_id, *dst, true);
                    }
                }
            }

            std::mem::swap(&mut cur_frontier, &mut next_frontier);
            iter += 1;
        }

        iter as u64
    }

    /// Run weighted shortest path (Dijkstra-like) with path tracking.
    ///
    /// The `get_weight` function maps (src_offset, dst_offset, edge_id) -> weight.
    pub fn run_weighted_shortest_path<F>(
        graph: &CSRAdjacency,
        source: u64,
        bfs_graph: &mut dyn crate::gds::bfs_graph::BaseBFSGraph,
        get_weight: F,
    ) -> u64
    where
        F: Fn(u64, u64, u64) -> f64 + Send + Sync,
    {
        use std::collections::BinaryHeap;

        let n = graph.num_nodes();

        if source as usize >= n {
            return 0;
        }

        // Min-heap: (cost, node_offset)
        let mut heap = BinaryHeap::new();
        let mut dist: Vec<f64> = vec![f64::MAX; n];
        let mut visited_count = 0u64;

        dist[source as usize] = 0.0;
        // Reverse ordering for min-heap
        heap.push(std::cmp::Reverse(HeapNode {
            cost: 0.0,
            node: source,
        }));

        while let Some(std::cmp::Reverse(HeapNode { cost, node })) = heap.pop() {
            if cost > dist[node as usize] {
                continue;
            }
            visited_count += 1;

            let bound_node_id = InternalID {
                table_id: 0,
                offset: node,
            };
            let neighbors = graph.neighbors(node as usize);

            for &(edge_id, ref dst) in neighbors {
                let nbr = dst.offset;
                if nbr as usize >= n {
                    continue;
                }

                let weight = get_weight(node, nbr, edge_id);
                if weight < 0.0 {
                    continue; // Skip negative weights
                }

                let new_cost = cost + weight;
                let old_cost = dist[nbr as usize];

                if new_cost < old_cost {
                    dist[nbr as usize] = new_cost;
                    // Replace parent with better path
                    bfs_graph.try_add_single_parent_with_weight(bound_node_id, edge_id, *dst, true, weight);
                    heap.push(std::cmp::Reverse(HeapNode {
                        cost: new_cost,
                        node: nbr,
                    }));
                } else if (new_cost - old_cost).abs() < f64::EPSILON {
                    // Alternative path with same cost (for all-weighted-shortest-paths)
                    bfs_graph.try_add_parent_with_weight(bound_node_id, edge_id, *dst, true, weight);
                }
            }
        }

        visited_count
    }

    /// Run all-weighted shortest paths (all shortest paths in weighted graph).
    pub fn run_all_weighted_shortest_paths<F>(
        graph: &CSRAdjacency,
        source: u64,
        bfs_graph: &mut dyn crate::gds::bfs_graph::BaseBFSGraph,
        get_weight: F,
    ) -> u64
    where
        F: Fn(u64, u64, u64) -> f64 + Send + Sync,
    {
        use std::collections::BinaryHeap;

        let n = graph.num_nodes();

        if source as usize >= n {
            return 0;
        }

        let mut heap = BinaryHeap::new();
        let mut dist: Vec<f64> = vec![f64::MAX; n];
        let mut visited_count = 0u64;

        dist[source as usize] = 0.0;
        heap.push(std::cmp::Reverse(HeapNode {
            cost: 0.0,
            node: source,
        }));

        while let Some(std::cmp::Reverse(HeapNode { cost, node })) = heap.pop() {
            if cost > dist[node as usize] {
                continue;
            }
            visited_count += 1;

            let bound_node_id = InternalID {
                table_id: 0,
                offset: node,
            };
            let neighbors = graph.neighbors(node as usize);

            for &(edge_id, ref dst) in neighbors {
                let nbr = dst.offset;
                if nbr as usize >= n {
                    continue;
                }

                let weight = get_weight(node, nbr, edge_id);
                if weight < 0.0 {
                    continue;
                }

                let new_cost = cost + weight;
                let old_cost = dist[nbr as usize];

                if new_cost < old_cost + f64::EPSILON
                    && bfs_graph.try_add_parent_with_weight(bound_node_id, edge_id, *dst, true, weight)
                    && new_cost < old_cost
                {
                    dist[nbr as usize] = new_cost;
                    heap.push(std::cmp::Reverse(HeapNode {
                        cost: new_cost,
                        node: nbr,
                    }));
                }
            }
        }

        visited_count
    }
}

/// Helper struct for min-heap operations in Dijkstra.
#[derive(Debug, Clone, Copy, PartialEq)]
struct HeapNode {
    cost: f64,
    node: u64,
}

impl Eq for HeapNode {}

impl PartialOrd for HeapNode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeapNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse for min-heap
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| self.node.cmp(&other.node))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gds::bfs_graph::{BaseBFSGraph, DenseBFSGraph};
    use crate::graph::{CSRAdjacency, Edge};

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
                src_offset: 1,
                dst_offset: 3,
                rel_id: 3,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 2,
                dst_offset: 3,
                rel_id: 4,
                rel_table_id: 0,
            },
            Edge {
                src_offset: 3,
                dst_offset: 4,
                rel_id: 5,
                rel_table_id: 0,
            },
        ];
        CSRAdjacency::build(&edges, 5)
    }

    #[test]
    fn test_single_shortest_path() {
        let csr = sample_csr();
        let mut bfs = DenseBFSGraph::new(5);
        let iters = GDSUtils::run_single_shortest_path(&csr, 0, &mut bfs, 10);
        assert!(iters > 0);

        // Node 4 should be reachable from 0 (0→1→3→4 or 0→2→3→4)
        let parent = bfs.get_parent_list_head_offset(4);
        assert!(parent.is_some(), "Node 4 should be reachable from 0");

        // Node 0's parent should be None
        let source_parent = bfs.get_parent_list_head_offset(0);
        assert!(source_parent.is_none());
    }

    #[test]
    fn test_all_shortest_paths() {
        let csr = sample_csr();
        let mut bfs = DenseBFSGraph::new(5);
        let iters = GDSUtils::run_all_shortest_paths(&csr, 0, &mut bfs, 10);
        assert!(iters > 0);

        // Node 3 should have at least one parent
        let parent = bfs.get_parent_list_head_offset(3);
        assert!(parent.is_some());
    }

    #[test]
    fn test_weighted_shortest_path() {
        let csr = sample_csr();
        let mut bfs = DenseBFSGraph::new(5);

        let iters = GDSUtils::run_weighted_shortest_path(&csr, 0, &mut bfs, |_src, _dst, _eid| 1.0);
        assert!(iters > 0);

        // Node 4 should be reachable
        let parent = bfs.get_parent_list_head_offset(4);
        assert!(parent.is_some());
    }
}
