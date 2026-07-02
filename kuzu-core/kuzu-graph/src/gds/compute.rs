//! Compute abstractions for graph algorithms.
//!
//! Ported from C++ `compute.h` / `gds_vertex_compute.h`.
//!
//! Provides:
//! - `EdgeCompute`: Per-edge processing during frontier-based traversal
//! - `VertexCompute`: Per-vertex processing for algorithms that touch all nodes

use kuzu_common::types::InternalID;

// ==================== EdgeCompute ====================

/// Per-edge computation during frontier-based graph traversal.
///
/// Analogous to the `edgeUpdate` function in the Ligra graph processing framework.
/// Called for each edge from a node in the current frontier.
///
/// The implementor returns neighbors to add to the next frontier.
/// The caller (GDSUtils) handles frontier management.
pub trait EdgeCompute: Send + Sync {
    /// Process an edge from `bound_node_id` to `nbr_node_id` via `edge_id`.
    ///
    /// Returns `true` if the neighbor should be added to the next frontier.
    fn edge_compute(
        &mut self,
        bound_node_id: InternalID,
        nbr_node_id: InternalID,
        edge_id: u64,
        is_fwd_edge: bool,
    ) -> bool;

    /// Reset any per-thread state before a new batch of edges.
    fn reset_single_thread_state(&mut self) {}

    /// Check whether the algorithm should terminate early.
    fn terminate(&self) -> bool {
        false
    }

    /// Create a boxed clone of this edge compute.
    fn box_clone(&self) -> Box<dyn EdgeCompute>;
}

// ==================== VertexCompute ====================

/// Per-vertex computation executed on all nodes in the graph.
///
/// Used for algorithms that need to touch every node (e.g., PageRank, WCC).
pub trait VertexCompute: Send + Sync {
    /// Called before processing a table. Return false to skip.
    fn begin_on_table(&mut self, _table_id: u64) -> bool {
        true
    }

    /// Process a single node.
    fn vertex_compute(&mut self, _offset: u64, _table_id: u64) {}

    /// Create a boxed clone.
    fn box_clone(&self) -> Box<dyn VertexCompute>;
}

// ==================== Simple EdgeCompute implementations ====================

/// EdgeCompute that marks all unvisited neighbors as active.
pub struct DefaultEdgeCompute;

impl EdgeCompute for DefaultEdgeCompute {
    fn edge_compute(
        &mut self,
        _bound_node_id: InternalID,
        _nbr_node_id: InternalID,
        _edge_id: u64,
        _is_fwd_edge: bool,
    ) -> bool {
        true
    }

    fn box_clone(&self) -> Box<dyn EdgeCompute> {
        Box::new(DefaultEdgeCompute)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edge_compute_default() {
        let mut ec = DefaultEdgeCompute;
        let result = ec.edge_compute(
            InternalID {
                table_id: 0,
                offset: 0,
            },
            InternalID {
                table_id: 0,
                offset: 1,
            },
            0,
            true,
        );
        assert!(result);
    }
}
