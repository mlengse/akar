#![allow(dead_code)]
//! Frontier management for graph algorithms.
//!
//! Ported from C++ `gds_frontier.h` / `gds_frontier.cpp`.
//!
//! A frontier tracks which nodes are "active" in the current BFS/DFS iteration.
//! Instead of using booleans, we assign iteration numbers — a node with iteration `i`
//! was visited in the i-th iteration.
//!
//! Frontier types:
//! - `SparseFrontier`: HashMap-based, for small graphs
//! - `DenseFrontier`: Array-based, for large graphs
//! - `SparseFrontierReference` / `DenseFrontierReference`: Views into other frontiers
//!
//! FrontierPair types manage the current/next frontier pair during BFS:
//! - `SPFrontierPair`: Single shortes path (cur == next frontier)
//! - `DenseSparseDynamicFrontierPair`: Adaptive dense↔sparse
//! - `DenseFrontierPair`: Always dense

use std::collections::HashSet;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use hashbrown::HashMap;
use kuzu_common::types::InternalID;

use crate::graph::Graph;

/// Type alias for BFS iteration counters.
pub type Iteration = u16;

/// Sentinel: node has not been visited.
pub const FRONTIER_UNVISITED: Iteration = u16::MAX;

/// Sentinel: node was visited in the initial (0-th) iteration.
pub const FRONTIER_INITIAL_VISITED: Iteration = 0;

/// Density state for adaptive frontier switching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GDSDensityState {
    Sparse,
    Dense,
}

// ==================== Frontier Trait ====================

/// Base frontier trait.
///
/// A frontier maps node offsets to iteration numbers, tracking which nodes
/// are active in the current BFS iteration.
pub trait Frontier: Send + Sync {
    /// Add a single node ID to the frontier at the given iteration.
    fn add_node(&mut self, node_id: InternalID, iter: Iteration);
    /// Add a node by offset only (table ID already pinned).
    fn add_node_offset(&mut self, offset: u64, iter: Iteration);
    /// Get the iteration at which a node was visited.
    fn get_iteration(&self, offset: u64) -> Iteration;
    /// Pin to a specific table ID (for multi-label graphs).
    fn pin_table_id(&mut self, _table_id: u64) {}
    /// Approximate number of entries.
    fn size(&self) -> usize;
}

// ==================== SparseFrontier ====================

/// Sparse frontier using a HashMap — efficient when few nodes are active.
pub struct SparseFrontier {
    data: HashMap<u64, Iteration>,
}

impl Default for SparseFrontier {
    fn default() -> Self {
        Self::new()
    }
}

impl SparseFrontier {
    pub fn new() -> Self {
        Self { data: HashMap::new() }
    }

    pub fn get_current_data(&self) -> &HashMap<u64, Iteration> {
        &self.data
    }
}

impl Frontier for SparseFrontier {
    fn add_node(&mut self, node_id: InternalID, iter: Iteration) {
        self.data.insert(node_id.offset, iter);
    }

    fn add_node_offset(&mut self, offset: u64, iter: Iteration) {
        self.data.insert(offset, iter);
    }

    fn get_iteration(&self, offset: u64) -> Iteration {
        self.data.get(&offset).copied().unwrap_or(FRONTIER_UNVISITED)
    }

    fn size(&self) -> usize {
        self.data.len()
    }
}

// ==================== SparseFrontierReference ====================

/// A reference to a SparseFrontier's data, for use in SPFrontierPair
/// where cur and next frontiers share the same underlying storage.
pub struct SparseFrontierReference {
    data: Arc<Mutex<HashMap<u64, Iteration>>>,
}

impl SparseFrontierReference {
    pub fn new(source: &SparseFrontier) -> Self {
        // We share the underlying HashMap via Arc<Mutex<>>
        // This is a simplified approach; C++ uses raw pointers.
        let map = source.data.clone();
        Self {
            data: Arc::new(Mutex::new(map)),
        }
    }
}

impl Frontier for SparseFrontierReference {
    fn add_node(&mut self, node_id: InternalID, iter: Iteration) {
        if let Ok(mut guard) = self.data.lock() {
            guard.insert(node_id.offset, iter);
        }
    }

    fn add_node_offset(&mut self, offset: u64, iter: Iteration) {
        if let Ok(mut guard) = self.data.lock() {
            guard.insert(offset, iter);
        }
    }

    fn get_iteration(&self, offset: u64) -> Iteration {
        self.data
            .lock()
            .ok()
            .and_then(|guard| guard.get(&offset).copied())
            .unwrap_or(FRONTIER_UNVISITED)
    }

    fn size(&self) -> usize {
        self.data.lock().map(|g| g.len()).unwrap_or(0)
    }
}

// ==================== DenseFrontier ====================

/// Dense frontier using a flat array of atomic iteration counters.
/// Each node offset maps directly to an array slot.
pub struct DenseFrontier {
    data: Vec<AtomicU16>,
    pinned: bool,
}

impl DenseFrontier {
    /// Create a new dense frontier with `size` entries, initialized to `val`.
    pub fn new(size: usize, val: Iteration) -> Self {
        let data = (0..size).map(|_| AtomicU16::new(val)).collect();
        Self { data, pinned: false }
    }

    /// Create from an existing graph's max node count.
    pub fn from_graph(graph: &Graph, val: Iteration) -> Self {
        let n = graph.num_nodes() as usize;
        Self::new(n, val)
    }

    /// Reset all entries to `val`.
    pub fn reset(&mut self, val: Iteration) {
        for entry in &self.data {
            entry.store(val, Ordering::SeqCst);
        }
    }

    /// Get the underlying atomic data for CAS operations.
    pub fn get_atomic(&self, offset: u64) -> Option<&AtomicU16> {
        self.data.get(offset as usize)
    }

    /// CAS: if current == expected, set to new. Returns true on success.
    pub fn compare_exchange(&self, offset: u64, expected: Iteration, new: Iteration) -> bool {
        self.data
            .get(offset as usize)
            .map(|a| {
                a.compare_exchange(expected, new, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            })
            .unwrap_or(false)
    }
}

impl Frontier for DenseFrontier {
    fn add_node(&mut self, node_id: InternalID, iter: Iteration) {
        let idx = node_id.offset as usize;
        if idx < self.data.len() {
            self.data[idx].store(iter, Ordering::SeqCst);
        }
    }

    fn add_node_offset(&mut self, offset: u64, iter: Iteration) {
        let idx = offset as usize;
        if idx < self.data.len() {
            self.data[idx].store(iter, Ordering::SeqCst);
        }
    }

    fn get_iteration(&self, offset: u64) -> Iteration {
        self.data
            .get(offset as usize)
            .map(|a| a.load(Ordering::SeqCst))
            .unwrap_or(FRONTIER_UNVISITED)
    }

    fn size(&self) -> usize {
        self.data.len()
    }
}

// ==================== DenseFrontierReference ====================

/// A reference to a DenseFrontier's data, sharing the same atomic array.
pub struct DenseFrontierReference {
    data: Arc<Vec<AtomicU16>>,
}

impl DenseFrontierReference {
    pub fn new(source: &DenseFrontier) -> Self {
        // We can't easily share Vec<AtomicU16> by reference in a thread-safe way
        // without Arc. For the simplified Rust port, we clone the current state.
        let values: Vec<AtomicU16> = source
            .data
            .iter()
            .map(|a| AtomicU16::new(a.load(Ordering::SeqCst)))
            .collect();
        Self { data: Arc::new(values) }
    }
}

impl Frontier for DenseFrontierReference {
    fn add_node(&mut self, node_id: InternalID, iter: Iteration) {
        let idx = node_id.offset as usize;
        if let Some(a) = self.data.get(idx) {
            a.store(iter, Ordering::SeqCst);
        }
    }

    fn add_node_offset(&mut self, offset: u64, iter: Iteration) {
        let idx = offset as usize;
        if let Some(a) = self.data.get(idx) {
            a.store(iter, Ordering::SeqCst);
        }
    }

    fn get_iteration(&self, offset: u64) -> Iteration {
        self.data
            .get(offset as usize)
            .map(|a| a.load(Ordering::SeqCst))
            .unwrap_or(FRONTIER_UNVISITED)
    }

    fn size(&self) -> usize {
        self.data.len()
    }
}

// ==================== FrontierPair Trait ====================

/// A pair of frontiers (current + next) used in iterative BFS algorithms.
///
/// Manages iteration counting, frontier switching, and active-node tracking.
pub trait FrontierPair: Send + Sync {
    /// Get the current iteration number.
    fn current_iter(&self) -> Iteration;
    /// Reset current iteration to 0.
    fn reset_current_iter(&mut self);
    /// Signal that there are active nodes for the next iteration.
    fn set_active_nodes_for_next_iter(&mut self);
    /// Check whether the algorithm should continue to the next iteration.
    fn continue_next_iter(&self, max_iter: u16) -> bool;
    /// Begin a new iteration (swaps current/next frontiers).
    fn begin_new_iteration(&mut self);
    /// Pin the current frontier to a table ID.
    fn pin_current_frontier(&mut self, table_id: u64);
    /// Pin the next frontier to a table ID.
    fn pin_next_frontier(&mut self, table_id: u64);
    /// Pin both for a frontier compute between two tables.
    fn begin_frontier_compute_between_tables(&mut self, cur_table_id: u64, next_table_id: u64);
    /// Add a node to the next frontier.
    fn add_node_to_next_frontier(&mut self, node_id: InternalID);
    /// Add a node by offset to the next frontier.
    fn add_node_to_next_frontier_offset(&mut self, offset: u64);
    /// Check if a node is active on the current frontier (visited in current iter).
    fn is_active_on_current_frontier(&self, offset: u64) -> bool;
    /// Get the iteration value of a node in the next frontier.
    fn get_next_frontier_value(&self, offset: u64) -> Iteration;
    /// Get all active nodes on the current frontier.
    fn get_active_nodes_on_current_frontier(&self) -> HashSet<u64>;
    /// Get the density state.
    fn state(&self) -> GDSDensityState;
    /// Whether switching to dense is needed.
    fn need_switch_to_dense(&self, threshold: u64) -> bool;
    /// Switch to dense mode.
    fn switch_to_dense(&mut self, graph: &Graph);
}

// ==================== SPFrontierPair ====================

/// Shortest-path frontier pair.
///
/// Unlike other algorithms, shortest path guarantees each node is visited at most once,
/// so current and next frontiers write to the same underlying storage.
pub struct SPFrontierPair {
    state: GDSDensityState,
    dense_frontier: Option<DenseFrontier>,
    cur_dense_frontier: Option<DenseFrontierReference>,
    next_dense_frontier: Option<DenseFrontierReference>,
    sparse_frontier: Option<SparseFrontier>,
    cur_sparse_frontier: Option<SparseFrontierReference>,
    next_sparse_frontier: Option<SparseFrontierReference>,
    cur_iter: Iteration,
    has_active_nodes: AtomicU64,
    num_nodes: u64,
}

impl SPFrontierPair {
    pub fn new_dense(num_nodes: usize) -> Self {
        let dense = DenseFrontier::new(num_nodes, FRONTIER_UNVISITED);
        // Source node (offset 0 if we track it) is visited in iter 0
        let cur_ref = DenseFrontierReference::new(&dense);
        let next_ref = DenseFrontierReference::new(&dense);
        Self {
            state: GDSDensityState::Dense,
            dense_frontier: Some(dense),
            cur_dense_frontier: Some(cur_ref),
            next_dense_frontier: Some(next_ref),
            sparse_frontier: None,
            cur_sparse_frontier: None,
            next_sparse_frontier: None,
            cur_iter: 0,
            has_active_nodes: AtomicU64::new(0),
            num_nodes: num_nodes as u64,
        }
    }

    pub fn new_sparse() -> Self {
        let sparse = SparseFrontier::new();
        let cur_ref = SparseFrontierReference::new(&sparse);
        let next_ref = SparseFrontierReference::new(&sparse);
        Self {
            state: GDSDensityState::Sparse,
            dense_frontier: None,
            cur_dense_frontier: None,
            next_dense_frontier: None,
            sparse_frontier: Some(sparse),
            cur_sparse_frontier: Some(cur_ref),
            next_sparse_frontier: Some(next_ref),
            cur_iter: 0,
            has_active_nodes: AtomicU64::new(0),
            num_nodes: 0,
        }
    }

    /// Get the active frontier (same for cur and next in shortest path).
    pub fn get_frontier(&self) -> Option<&DenseFrontier> {
        self.dense_frontier.as_ref()
    }

    pub fn get_frontier_mut(&mut self) -> Option<&mut DenseFrontier> {
        self.dense_frontier.as_mut()
    }

    /// Get number of active nodes on current frontier.
    pub fn num_active_nodes(&self) -> u64 {
        self.has_active_nodes.load(Ordering::Relaxed)
    }
}

impl FrontierPair for SPFrontierPair {
    fn current_iter(&self) -> Iteration {
        self.cur_iter
    }

    fn reset_current_iter(&mut self) {
        self.cur_iter = 0;
    }

    fn set_active_nodes_for_next_iter(&mut self) {
        self.has_active_nodes.fetch_add(1, Ordering::Relaxed);
    }

    fn continue_next_iter(&self, max_iter: u16) -> bool {
        self.has_active_nodes.load(Ordering::Relaxed) > 0 && self.cur_iter < max_iter
    }

    fn begin_new_iteration(&mut self) {
        self.cur_iter += 1;
        self.has_active_nodes.store(0, Ordering::Relaxed);
    }

    fn pin_current_frontier(&mut self, _table_id: u64) {
        // Multi-label graph: no-op in simplified version
    }

    fn pin_next_frontier(&mut self, _table_id: u64) {
        // Multi-label graph: no-op in simplified version
    }

    fn begin_frontier_compute_between_tables(&mut self, cur_table_id: u64, next_table_id: u64) {
        self.pin_current_frontier(cur_table_id);
        self.pin_next_frontier(next_table_id);
    }

    fn add_node_to_next_frontier(&mut self, node_id: InternalID) {
        match self.state {
            GDSDensityState::Dense => {
                if let Some(ref frontier) = self.dense_frontier {
                    let iter = self.cur_iter + 1;
                    let idx = node_id.offset as usize;
                    if idx < frontier.data.len() {
                        frontier.data[idx].store(iter, Ordering::SeqCst);
                    }
                }
            }
            GDSDensityState::Sparse => {
                if let Some(ref mut frontier) = self.sparse_frontier {
                    frontier.add_node(node_id, self.cur_iter + 1);
                }
            }
        }
    }

    fn add_node_to_next_frontier_offset(&mut self, offset: u64) {
        match self.state {
            GDSDensityState::Dense => {
                if let Some(ref frontier) = self.dense_frontier {
                    let iter = self.cur_iter + 1;
                    let idx = offset as usize;
                    if idx < frontier.data.len() {
                        frontier.data[idx].store(iter, Ordering::SeqCst);
                    }
                }
            }
            GDSDensityState::Sparse => {
                if let Some(ref mut frontier) = self.sparse_frontier {
                    frontier.add_node_offset(offset, self.cur_iter + 1);
                }
            }
        }
    }

    fn is_active_on_current_frontier(&self, offset: u64) -> bool {
        match self.state {
            GDSDensityState::Dense => self
                .dense_frontier
                .as_ref()
                .map(|f| f.get_iteration(offset) == self.cur_iter)
                .unwrap_or(false),
            GDSDensityState::Sparse => self
                .sparse_frontier
                .as_ref()
                .map(|f| f.get_iteration(offset) == self.cur_iter)
                .unwrap_or(false),
        }
    }

    fn get_next_frontier_value(&self, offset: u64) -> Iteration {
        match self.state {
            GDSDensityState::Dense => self
                .dense_frontier
                .as_ref()
                .map(|f| f.get_iteration(offset))
                .unwrap_or(FRONTIER_UNVISITED),
            GDSDensityState::Sparse => self
                .sparse_frontier
                .as_ref()
                .map(|f| f.get_iteration(offset))
                .unwrap_or(FRONTIER_UNVISITED),
        }
    }

    fn get_active_nodes_on_current_frontier(&self) -> HashSet<u64> {
        let mut active = HashSet::new();
        match self.state {
            GDSDensityState::Dense => {
                if let Some(ref f) = self.dense_frontier {
                    for (i, a) in f.data.iter().enumerate() {
                        if a.load(Ordering::SeqCst) == self.cur_iter {
                            active.insert(i as u64);
                        }
                    }
                }
            }
            GDSDensityState::Sparse => {
                if let Some(ref f) = self.sparse_frontier {
                    for (&off, &iter) in &f.data {
                        if iter == self.cur_iter {
                            active.insert(off);
                        }
                    }
                }
            }
        }
        active
    }

    fn state(&self) -> GDSDensityState {
        self.state
    }

    fn need_switch_to_dense(&self, _threshold: u64) -> bool {
        // Simplified: stay in current mode
        false
    }

    fn switch_to_dense(&mut self, _graph: &Graph) {
        // Simplified: no-op
    }
}

// ==================== DenseSparseDynamicFrontierPair ====================

/// Frontier pair that uses separate current/next dense frontiers
/// (needed for weighted shortest path where nodes can be revisited).
pub struct DenseSparseDynamicFrontierPair {
    state: GDSDensityState,
    cur_dense_frontier: Option<DenseFrontier>,
    next_dense_frontier: Option<DenseFrontier>,
    cur_sparse_frontier: Option<SparseFrontier>,
    next_sparse_frontier: Option<SparseFrontier>,
    cur_iter: Iteration,
    has_active_nodes: AtomicU64,
}

impl DenseSparseDynamicFrontierPair {
    pub fn new_dense(num_nodes: usize) -> Self {
        let cur = DenseFrontier::new(num_nodes, FRONTIER_UNVISITED);
        let next = DenseFrontier::new(num_nodes, FRONTIER_UNVISITED);
        Self {
            state: GDSDensityState::Dense,
            cur_dense_frontier: Some(cur),
            next_dense_frontier: Some(next),
            cur_sparse_frontier: None,
            next_sparse_frontier: None,
            cur_iter: 0,
            has_active_nodes: AtomicU64::new(0),
        }
    }
}

impl FrontierPair for DenseSparseDynamicFrontierPair {
    fn current_iter(&self) -> Iteration {
        self.cur_iter
    }

    fn reset_current_iter(&mut self) {
        self.cur_iter = 0;
    }

    fn set_active_nodes_for_next_iter(&mut self) {
        self.has_active_nodes.fetch_add(1, Ordering::Relaxed);
    }

    fn continue_next_iter(&self, max_iter: u16) -> bool {
        self.has_active_nodes.load(Ordering::Relaxed) > 0 && self.cur_iter < max_iter
    }

    fn begin_new_iteration(&mut self) {
        // Swap cur and next frontiers
        std::mem::swap(&mut self.cur_dense_frontier, &mut self.next_dense_frontier);
        std::mem::swap(&mut self.cur_sparse_frontier, &mut self.next_sparse_frontier);
        self.cur_iter += 1;
        self.has_active_nodes.store(0, Ordering::Relaxed);
        // Reset next frontier
        if let Some(ref mut next) = self.next_dense_frontier {
            next.reset(FRONTIER_UNVISITED);
        }
        if let Some(ref mut next) = self.next_sparse_frontier {
            next.data.clear();
        }
    }

    fn pin_current_frontier(&mut self, _table_id: u64) {}
    fn pin_next_frontier(&mut self, _table_id: u64) {}

    fn begin_frontier_compute_between_tables(&mut self, cur_table_id: u64, next_table_id: u64) {
        self.pin_current_frontier(cur_table_id);
        self.pin_next_frontier(next_table_id);
    }

    fn add_node_to_next_frontier(&mut self, node_id: InternalID) {
        match self.state {
            GDSDensityState::Dense => {
                if let Some(ref mut next) = self.next_dense_frontier {
                    next.add_node(node_id, self.cur_iter + 1);
                }
            }
            GDSDensityState::Sparse => {
                if let Some(ref mut next) = self.next_sparse_frontier {
                    next.add_node(node_id, self.cur_iter + 1);
                }
            }
        }
    }

    fn add_node_to_next_frontier_offset(&mut self, offset: u64) {
        match self.state {
            GDSDensityState::Dense => {
                if let Some(ref mut next) = self.next_dense_frontier {
                    next.add_node_offset(offset, self.cur_iter + 1);
                }
            }
            GDSDensityState::Sparse => {
                if let Some(ref mut next) = self.next_sparse_frontier {
                    next.add_node_offset(offset, self.cur_iter + 1);
                }
            }
        }
    }

    fn is_active_on_current_frontier(&self, offset: u64) -> bool {
        match self.state {
            GDSDensityState::Dense => self
                .cur_dense_frontier
                .as_ref()
                .map(|f| f.get_iteration(offset) == self.cur_iter)
                .unwrap_or(false),
            GDSDensityState::Sparse => self
                .cur_sparse_frontier
                .as_ref()
                .map(|f| f.get_iteration(offset) == self.cur_iter)
                .unwrap_or(false),
        }
    }

    fn get_next_frontier_value(&self, offset: u64) -> Iteration {
        match self.state {
            GDSDensityState::Dense => self
                .next_dense_frontier
                .as_ref()
                .map(|f| f.get_iteration(offset))
                .unwrap_or(FRONTIER_UNVISITED),
            GDSDensityState::Sparse => self
                .next_sparse_frontier
                .as_ref()
                .map(|f| f.get_iteration(offset))
                .unwrap_or(FRONTIER_UNVISITED),
        }
    }

    fn get_active_nodes_on_current_frontier(&self) -> HashSet<u64> {
        let mut active = HashSet::new();
        match self.state {
            GDSDensityState::Dense => {
                if let Some(ref f) = self.cur_dense_frontier {
                    for (i, a) in f.data.iter().enumerate() {
                        if a.load(Ordering::SeqCst) == self.cur_iter {
                            active.insert(i as u64);
                        }
                    }
                }
            }
            GDSDensityState::Sparse => {
                if let Some(ref f) = self.cur_sparse_frontier {
                    for (&off, &iter) in &f.data {
                        if iter == self.cur_iter {
                            active.insert(off);
                        }
                    }
                }
            }
        }
        active
    }

    fn state(&self) -> GDSDensityState {
        self.state
    }

    fn need_switch_to_dense(&self, _threshold: u64) -> bool {
        false
    }

    fn switch_to_dense(&mut self, _graph: &Graph) {}
}

// ==================== DenseFrontierPair ====================

/// Always-dense frontier pair (for algorithms that touch all nodes).
pub struct DenseFrontierPair {
    cur_dense_frontier: DenseFrontier,
    next_dense_frontier: DenseFrontier,
    cur_iter: Iteration,
    has_active_nodes: AtomicU64,
}

impl DenseFrontierPair {
    pub fn new(num_nodes: usize) -> Self {
        Self {
            cur_dense_frontier: DenseFrontier::new(num_nodes, FRONTIER_UNVISITED),
            next_dense_frontier: DenseFrontier::new(num_nodes, FRONTIER_UNVISITED),
            cur_iter: 0,
            has_active_nodes: AtomicU64::new(0),
        }
    }

    pub fn reset(&mut self, val: Iteration) {
        self.cur_dense_frontier.reset(val);
        self.next_dense_frontier.reset(val);
    }
}

impl FrontierPair for DenseFrontierPair {
    fn current_iter(&self) -> Iteration {
        self.cur_iter
    }

    fn reset_current_iter(&mut self) {
        self.cur_iter = 0;
    }

    fn set_active_nodes_for_next_iter(&mut self) {
        self.has_active_nodes.fetch_add(1, Ordering::Relaxed);
    }

    fn continue_next_iter(&self, max_iter: u16) -> bool {
        self.has_active_nodes.load(Ordering::Relaxed) > 0 && self.cur_iter < max_iter
    }

    fn begin_new_iteration(&mut self) {
        std::mem::swap(&mut self.cur_dense_frontier, &mut self.next_dense_frontier);
        self.cur_iter += 1;
        self.has_active_nodes.store(0, Ordering::Relaxed);
        self.next_dense_frontier.reset(FRONTIER_UNVISITED);
    }

    fn pin_current_frontier(&mut self, _table_id: u64) {}
    fn pin_next_frontier(&mut self, _table_id: u64) {}

    fn begin_frontier_compute_between_tables(&mut self, cur_table_id: u64, next_table_id: u64) {
        self.pin_current_frontier(cur_table_id);
        self.pin_next_frontier(next_table_id);
    }

    fn add_node_to_next_frontier(&mut self, node_id: InternalID) {
        self.next_dense_frontier.add_node(node_id, self.cur_iter + 1);
    }

    fn add_node_to_next_frontier_offset(&mut self, offset: u64) {
        self.next_dense_frontier.add_node_offset(offset, self.cur_iter + 1);
    }

    fn is_active_on_current_frontier(&self, offset: u64) -> bool {
        self.cur_dense_frontier.get_iteration(offset) == self.cur_iter
    }

    fn get_next_frontier_value(&self, offset: u64) -> Iteration {
        self.next_dense_frontier.get_iteration(offset)
    }

    fn get_active_nodes_on_current_frontier(&self) -> HashSet<u64> {
        let mut active = HashSet::new();
        for (i, a) in self.cur_dense_frontier.data.iter().enumerate() {
            if a.load(Ordering::SeqCst) == self.cur_iter {
                active.insert(i as u64);
            }
        }
        active
    }

    fn state(&self) -> GDSDensityState {
        GDSDensityState::Dense
    }

    fn need_switch_to_dense(&self, _threshold: u64) -> bool {
        false
    }

    fn switch_to_dense(&mut self, _graph: &Graph) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sparse_frontier() {
        let mut f = SparseFrontier::new();
        assert_eq!(f.get_iteration(0), FRONTIER_UNVISITED);
        f.add_node_offset(0, 1);
        assert_eq!(f.get_iteration(0), 1);
        assert_eq!(f.size(), 1);
    }

    #[test]
    fn test_dense_frontier() {
        let mut f = DenseFrontier::new(10, FRONTIER_UNVISITED);
        assert_eq!(f.get_iteration(5), FRONTIER_UNVISITED);
        f.add_node_offset(5, 1);
        assert_eq!(f.get_iteration(5), 1);
    }

    #[test]
    fn test_sp_frontier_pair_active_nodes() {
        let mut pair = SPFrontierPair::new_dense(10);
        assert_eq!(pair.current_iter(), 0);
        // Add a node to next frontier
        pair.add_node_to_next_frontier(InternalID { table_id: 0, offset: 3 });
        // Mark active and check
        pair.set_active_nodes_for_next_iter();
        assert!(pair.continue_next_iter(10));
        pair.begin_new_iteration();
        assert_eq!(pair.current_iter(), 1);
        assert!(pair.is_active_on_current_frontier(3));
    }

    #[test]
    fn test_dense_frontier_pair() {
        let mut pair = DenseFrontierPair::new(5);
        assert_eq!(pair.current_iter(), 0);
        pair.add_node_to_next_frontier_offset(1);
        pair.set_active_nodes_for_next_iter();
        assert!(pair.continue_next_iter(10));
        pair.begin_new_iteration();
        assert!(pair.is_active_on_current_frontier(1));
    }

    #[test]
    fn test_dense_sparse_dynamic_pair() {
        let mut pair = DenseSparseDynamicFrontierPair::new_dense(5);
        assert_eq!(pair.current_iter(), 0);
        pair.add_node_to_next_frontier_offset(2);
        pair.set_active_nodes_for_next_iter();
        assert!(pair.continue_next_iter(10));
        pair.begin_new_iteration();
        assert_eq!(pair.current_iter(), 1);
        assert!(pair.is_active_on_current_frontier(2));
    }
}
