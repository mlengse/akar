#![allow(dead_code)]
//! BFS graph for parent tracking in path-aware algorithms.
//!
//! Ported from C++ `bfs_graph.h` / `bfs_graph.cpp`.
//!
//! A `ParentList` is a singly-linked list node that records how a node was reached
//! during BFS traversal: which previous node, which edge, and at which iteration.
//!
//! BFS graph types:
//! - `DenseBFSGraph`: Array-based storage for large graphs (per-offset parent list)
//! - `SparseBFSGraph`: HashMap-based storage for small graphs
//! - `BFSGraphManager`: Manages sparse↔dense switching

use hashbrown::HashMap;
use kuzu_common::types::InternalID;

use crate::graph::Graph;

/// Sentinel for uninitialized parent list iteration.
pub const PARENT_LIST_UNINITIALIZED: u16 = u16::MAX;

/// Maximum number of parent lists (nodes) that can be stored in a block.
pub const PARENT_BLOCK_CAPACITY: u64 = 1 << 19; // 524_288

// ==================== ParentList ====================

/// A node in the parent-tracking linked list.
///
/// Records how a target node was reached during BFS traversal:
/// - `node_id`: The parent node (predecessor)
/// - `edge_id`: The edge used to reach this node
/// - `is_fwd`: Whether the edge was traversed forward
/// - `iter`: The BFS iteration when this parent was recorded
/// - `cost`: Path cost (for weighted algorithms)
/// - `next`: Pointer to the next parent (for multiple shortest paths)
#[derive(Debug, Clone)]
pub struct ParentList {
    pub node_id: InternalID,
    pub edge_id: u64,
    pub is_fwd: bool,
    pub iter: u16,
    pub cost: f64,
    pub next: Option<Box<ParentList>>,
}

impl ParentList {
    pub fn new(node_id: InternalID, edge_id: u64, is_fwd: bool, iter: u16) -> Self {
        Self {
            node_id,
            edge_id,
            is_fwd,
            iter,
            cost: f64::MAX,
            next: None,
        }
    }

    pub fn new_with_cost(
        node_id: InternalID,
        edge_id: u64,
        is_fwd: bool,
        iter: u16,
        cost: f64,
    ) -> Self {
        Self {
            node_id,
            edge_id,
            is_fwd,
            iter,
            cost,
            next: None,
        }
    }

    /// Get the last node in this linked list.
    pub fn last(&self) -> &ParentList {
        let mut current = self;
        while let Some(ref n) = current.next {
            current = n;
        }
        current
    }

    /// Count the length of this linked list.
    pub fn len(&self) -> usize {
        let mut count = 1;
        let mut current = self;
        while let Some(ref n) = current.next {
            count += 1;
            current = n;
        }
        count
    }
}

// ==================== BaseBFSGraph Trait ====================

/// Base trait for BFS graph implementations.
///
/// Manages parent list storage for path tracking during BFS traversal.
pub trait BaseBFSGraph: Send + Sync {
    /// Pin to a specific table ID.
    fn pin_table_id(&mut self, table_id: u64);
    /// Add a parent for a node (allows multiple parents — for all-shortest-paths).
    fn add_parent(
        &mut self,
        iter: u16,
        bound_node_id: InternalID,
        edge_id: u64,
        nbr_node_id: InternalID,
        fwd_edge: bool,
    );
    /// Add a single parent (only first parent is kept — for single-shortest-path).
    fn add_single_parent(
        &mut self,
        iter: u16,
        bound_node_id: InternalID,
        edge_id: u64,
        nbr_node_id: InternalID,
        fwd_edge: bool,
    );
    /// Try to add a parent with weight checking (for weighted paths).
    /// Returns true if the parent was added.
    fn try_add_parent_with_weight(
        &mut self,
        bound_node_id: InternalID,
        edge_id: u64,
        nbr_node_id: InternalID,
        fwd_edge: bool,
        weight: f64,
    ) -> bool;
    /// Try to add a single parent with weight checking.
    fn try_add_single_parent_with_weight(
        &mut self,
        bound_node_id: InternalID,
        edge_id: u64,
        nbr_node_id: InternalID,
        fwd_edge: bool,
        weight: f64,
    ) -> bool;
    /// Get the parent list head for a node offset.
    fn get_parent_list_head_offset(&self, offset: u64) -> Option<&ParentList>;
    /// Get the parent list head for a node ID.
    fn get_parent_list_head(&self, node_id: InternalID) -> Option<&ParentList>;
    /// Set the parent list for a node.
    fn set_parent_list(&mut self, offset: u64, parent: Option<Box<ParentList>>);
    /// Number of nodes tracked.
    fn num_nodes(&self) -> u64;
}

// ==================== DenseBFSGraph ====================

/// Dense BFS graph — array-based parent storage.
///
/// Each node offset maps to an optional `Box<ParentList>` pointer.
pub struct DenseBFSGraph {
    data: Vec<Option<Box<ParentList>>>,
    pinned_table_id: u64,
    max_offset_map: HashMap<u64, u64>, // table_id -> max_offset
}

impl DenseBFSGraph {
    pub fn new(num_nodes: usize) -> Self {
        let mut data = Vec::with_capacity(num_nodes);
        for _ in 0..num_nodes {
            data.push(None);
        }
        Self {
            data,
            pinned_table_id: 0,
            max_offset_map: HashMap::new(),
        }
    }

    pub fn from_graph(graph: &Graph) -> Self {
        let n = graph.num_nodes() as usize;
        Self::new(n)
    }

    fn get_mut(&mut self, offset: u64) -> Option<&mut Option<Box<ParentList>>> {
        self.data.get_mut(offset as usize)
    }

    fn get(&self, offset: u64) -> Option<&Option<Box<ParentList>>> {
        self.data.get(offset as usize)
    }
}

impl BaseBFSGraph for DenseBFSGraph {
    fn pin_table_id(&mut self, table_id: u64) {
        self.pinned_table_id = table_id;
    }

    fn add_parent(
        &mut self,
        iter: u16,
        bound_node_id: InternalID,
        edge_id: u64,
        nbr_node_id: InternalID,
        fwd_edge: bool,
    ) {
        let offset = nbr_node_id.offset;
        if let Some(slot) = self.get_mut(offset) {
            let mut new_parent = Box::new(ParentList::new(bound_node_id, edge_id, fwd_edge, iter));
            // Prepend: take existing, put it as next of new, then set new as head
            if let Some(old_head) = slot.take() {
                new_parent.next = Some(old_head);
            }
            *slot = Some(new_parent);
        }
    }

    fn add_single_parent(
        &mut self,
        iter: u16,
        bound_node_id: InternalID,
        edge_id: u64,
        nbr_node_id: InternalID,
        fwd_edge: bool,
    ) {
        let offset = nbr_node_id.offset;
        if let Some(slot) = self.get_mut(offset) {
            if slot.is_none() {
                *slot = Some(Box::new(ParentList::new(
                    bound_node_id, edge_id, fwd_edge, iter,
                )));
            }
        }
    }

    fn try_add_parent_with_weight(
        &mut self,
        bound_node_id: InternalID,
        edge_id: u64,
        nbr_node_id: InternalID,
        fwd_edge: bool,
        weight: f64,
    ) -> bool {
        let offset = nbr_node_id.offset;
        if let Some(slot) = self.get_mut(offset) {
            if slot.is_none() {
                *slot = Some(Box::new(ParentList::new_with_cost(
                    bound_node_id, edge_id, fwd_edge, 0, weight,
                )));
                return true;
            }
            // slot has Some value — check cost
            let existing_cost = slot.as_ref().unwrap().cost;
            if weight < existing_cost {
                *slot = Some(Box::new(ParentList::new_with_cost(
                    bound_node_id, edge_id, fwd_edge, 0, weight,
                )));
                true
            } else if (weight - existing_cost).abs() < f64::EPSILON {
                // Same cost — prepend
                let old_head = slot.take();
                let mut new_parent = Box::new(ParentList::new_with_cost(
                    bound_node_id, edge_id, fwd_edge, 0, weight,
                ));
                new_parent.next = old_head;
                *slot = Some(new_parent);
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    fn try_add_single_parent_with_weight(
        &mut self,
        bound_node_id: InternalID,
        edge_id: u64,
        nbr_node_id: InternalID,
        fwd_edge: bool,
        weight: f64,
    ) -> bool {
        let offset = nbr_node_id.offset;
        if let Some(slot) = self.get_mut(offset) {
            if slot.is_none() {
                *slot = Some(Box::new(ParentList::new_with_cost(
                    bound_node_id, edge_id, fwd_edge, 0, weight,
                )));
                return true;
            }
            let existing_cost = slot.as_ref().unwrap().cost;
            if weight < existing_cost {
                *slot = Some(Box::new(ParentList::new_with_cost(
                    bound_node_id, edge_id, fwd_edge, 0, weight,
                )));
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    fn get_parent_list_head_offset(&self, offset: u64) -> Option<&ParentList> {
        self.get(offset).and_then(|o| o.as_deref())
    }

    fn get_parent_list_head(&self, node_id: InternalID) -> Option<&ParentList> {
        self.get_parent_list_head_offset(node_id.offset)
    }

    fn set_parent_list(&mut self, offset: u64, parent: Option<Box<ParentList>>) {
        if let Some(slot) = self.get_mut(offset) {
            *slot = parent;
        }
    }

    fn num_nodes(&self) -> u64 {
        self.data.len() as u64
    }
}

// ==================== SparseBFSGraph ====================

/// Sparse BFS graph — HashMap-based parent storage.
pub struct SparseBFSGraph {
    data: HashMap<u64, Box<ParentList>>,
    pinned_table_id: u64,
}

impl SparseBFSGraph {
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
            pinned_table_id: 0,
        }
    }

    pub fn get_current_data(&self) -> &HashMap<u64, Box<ParentList>> {
        &self.data
    }
}

impl BaseBFSGraph for SparseBFSGraph {
    fn pin_table_id(&mut self, _table_id: u64) {}

    fn add_parent(
        &mut self,
        iter: u16,
        bound_node_id: InternalID,
        edge_id: u64,
        nbr_node_id: InternalID,
        fwd_edge: bool,
    ) {
        let offset = nbr_node_id.offset;
        let mut new_parent = Box::new(ParentList::new(bound_node_id, edge_id, fwd_edge, iter));
        // Prepend: take existing head, put as next of new, insert new
        if let Some(old_head) = self.data.remove(&offset) {
            new_parent.next = Some(old_head);
        }
        self.data.insert(offset, new_parent);
    }

    fn add_single_parent(
        &mut self,
        iter: u16,
        bound_node_id: InternalID,
        edge_id: u64,
        nbr_node_id: InternalID,
        fwd_edge: bool,
    ) {
        let offset = nbr_node_id.offset;
        if !self.data.contains_key(&offset) {
            let parent = Box::new(ParentList::new(bound_node_id, edge_id, fwd_edge, iter));
            self.data.insert(offset, parent);
        }
    }

    fn try_add_parent_with_weight(
        &mut self,
        bound_node_id: InternalID,
        edge_id: u64,
        nbr_node_id: InternalID,
        fwd_edge: bool,
        weight: f64,
    ) -> bool {
        let offset = nbr_node_id.offset;
        if let Some(existing) = self.data.remove(&offset) {
            if weight < existing.cost {
                self.data.insert(
                    offset,
                    Box::new(ParentList::new_with_cost(
                        bound_node_id, edge_id, fwd_edge, 0, weight,
                    )),
                );
                true
            } else if (weight - existing.cost).abs() < f64::EPSILON {
                // Same cost — prepend
                let mut new_parent = Box::new(ParentList::new_with_cost(
                    bound_node_id, edge_id, fwd_edge, 0, weight,
                ));
                new_parent.next = Some(existing);
                self.data.insert(offset, new_parent);
                true
            } else {
                self.data.insert(offset, existing);
                false
            }
        } else {
            self.data.insert(
                offset,
                Box::new(ParentList::new_with_cost(
                    bound_node_id, edge_id, fwd_edge, 0, weight,
                )),
            );
            true
        }
    }

    fn try_add_single_parent_with_weight(
        &mut self,
        bound_node_id: InternalID,
        edge_id: u64,
        nbr_node_id: InternalID,
        fwd_edge: bool,
        weight: f64,
    ) -> bool {
        let offset = nbr_node_id.offset;
        if let Some(existing) = self.data.remove(&offset) {
            if weight < existing.cost {
                self.data.insert(
                    offset,
                    Box::new(ParentList::new_with_cost(
                        bound_node_id, edge_id, fwd_edge, 0, weight,
                    )),
                );
                true
            } else {
                self.data.insert(offset, existing);
                false
            }
        } else {
            self.data.insert(
                offset,
                Box::new(ParentList::new_with_cost(
                    bound_node_id, edge_id, fwd_edge, 0, weight,
                )),
            );
            true
        }
    }

    fn get_parent_list_head_offset(&self, offset: u64) -> Option<&ParentList> {
        self.data.get(&offset).map(|b| b.as_ref())
    }

    fn get_parent_list_head(&self, node_id: InternalID) -> Option<&ParentList> {
        self.get_parent_list_head_offset(node_id.offset)
    }

    fn set_parent_list(&mut self, offset: u64, parent: Option<Box<ParentList>>) {
        if let Some(p) = parent {
            self.data.insert(offset, p);
        } else {
            self.data.remove(&offset);
        }
    }

    fn num_nodes(&self) -> u64 {
        self.data.len() as u64
    }
}

// ==================== BFSGraphManager ====================

/// Manages BFS graph sparse↔dense switching.
pub struct BFSGraphManager {
    state: GDSDensityState,
    dense_graph: Option<DenseBFSGraph>,
    sparse_graph: Option<SparseBFSGraph>,
}

impl BFSGraphManager {
    pub fn new_dense(num_nodes: usize) -> Self {
        Self {
            state: GDSDensityState::Dense,
            dense_graph: Some(DenseBFSGraph::new(num_nodes)),
            sparse_graph: None,
        }
    }

    pub fn new_sparse() -> Self {
        Self {
            state: GDSDensityState::Sparse,
            dense_graph: None,
            sparse_graph: Some(SparseBFSGraph::new()),
        }
    }

    /// Get the current active graph.
    pub fn get_current_graph(&self) -> &dyn BaseBFSGraph {
        match self.state {
            GDSDensityState::Dense => self.dense_graph.as_ref().unwrap() as &dyn BaseBFSGraph,
            GDSDensityState::Sparse => self.sparse_graph.as_ref().unwrap() as &dyn BaseBFSGraph,
        }
    }

    /// Get the current active graph (mutable).
    pub fn get_current_graph_mut(&mut self) -> &mut dyn BaseBFSGraph {
        match self.state {
            GDSDensityState::Dense => self.dense_graph.as_mut().unwrap() as &mut dyn BaseBFSGraph,
            GDSDensityState::Sparse => self.sparse_graph.as_mut().unwrap() as &mut dyn BaseBFSGraph,
        }
    }

    /// Switch from sparse to dense.
    pub fn switch_to_dense(&mut self, graph: &Graph) {
        let n = graph.num_nodes() as usize;
        let mut dense = DenseBFSGraph::new(n);

        if let Some(ref sparse) = self.sparse_graph {
            for (&offset, parent) in &sparse.data {
                let idx = offset as usize;
                if idx < dense.data.len() {
                    dense.data[idx] = Some(parent.clone());
                }
            }
        }

        self.dense_graph = Some(dense);
        self.sparse_graph = None;
        self.state = GDSDensityState::Dense;
    }
}

use crate::gds::frontier::GDSDensityState;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gds::bfs_graph::BaseBFSGraph;

    #[test]
    fn test_parent_list_basic() {
        let pl = ParentList::new(
            InternalID {
                table_id: 0,
                offset: 1,
            },
            42,
            true,
            1,
        );
        assert_eq!(pl.node_id.offset, 1);
        assert_eq!(pl.edge_id, 42);
        assert!(pl.is_fwd);
        assert_eq!(pl.iter, 1);
        assert!(pl.next.is_none());
    }

    #[test]
    fn test_dense_bfs_graph_add_single_parent() {
        let mut g = DenseBFSGraph::new(10);
        let src = InternalID {
            table_id: 0,
            offset: 0,
        };
        let dst = InternalID {
            table_id: 0,
            offset: 5,
        };
        g.add_single_parent(1, src, 100, dst, true);
        assert!(g.get_parent_list_head_offset(5).is_some());
        // Second add should be no-op (single parent)
        g.add_single_parent(1, src, 101, dst, true);
        let p = g.get_parent_list_head_offset(5).unwrap();
        assert_eq!(p.edge_id, 100);
    }

    #[test]
    fn test_dense_bfs_graph_add_parent_multiple() {
        let mut g = DenseBFSGraph::new(10);
        let src1 = InternalID {
            table_id: 0,
            offset: 1,
        };
        let src2 = InternalID {
            table_id: 0,
            offset: 2,
        };
        let dst = InternalID {
            table_id: 0,
            offset: 5,
        };
        g.add_parent(1, src1, 100, dst, true);
        g.add_parent(1, src2, 200, dst, true);
        let p = g.get_parent_list_head_offset(5).unwrap();
        // Second parent was prepended, so edge_id should be 200
        assert_eq!(p.edge_id, 200);
        assert!(p.next.is_some());
        assert_eq!(p.next.as_ref().unwrap().edge_id, 100);
    }

    #[test]
    fn test_dense_bfs_graph_weighted() {
        let mut g = DenseBFSGraph::new(10);
        let src = InternalID {
            table_id: 0,
            offset: 0,
        };
        let dst = InternalID {
            table_id: 0,
            offset: 5,
        };
        // First path with weight 10
        assert!(g.try_add_single_parent_with_weight(src, 100, dst, true, 10.0));
        let p = g.get_parent_list_head_offset(5).unwrap();
        assert_eq!(p.cost, 10.0);
        // Better path with weight 5
        assert!(g.try_add_single_parent_with_weight(src, 101, dst, true, 5.0));
        let p = g.get_parent_list_head_offset(5).unwrap();
        assert_eq!(p.cost, 5.0);
        // Worse path with weight 20 should be rejected
        assert!(!g.try_add_single_parent_with_weight(src, 102, dst, true, 20.0));
    }

    #[test]
    fn test_bfs_graph_manager() {
        let mgr = BFSGraphManager::new_dense(10);
        assert!(mgr.get_current_graph().num_nodes() >= 10);
    }
}
