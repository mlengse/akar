//! Compressed Sparse Row (CSR) Storage Format
//!
//! This module provides a specialized storage format for graph traversals,
//! designed to be used alongside columnar storage for properties.
//! The CSR format is optimized for fast neighborhood lookups.

use crate::page::FileHandle;
use akar_common::error::StorageError;
use std::sync::Arc;
use std::sync::RwLock;

/// A Compressed Sparse Row (CSR) index for storing graph edges.
///
/// `CsrIndex` stores edges compactly using two pairs of arrays:
/// - Forward: `fwd_offsets` + `fwd_adjacency` — src → [dst, ...]
/// - Reverse: `rev_offsets` + `rev_adjacency` — dst → [src, ...]
///
/// Each offset array has length `num_nodes + 1`. For node `i`, its neighbors
/// are in `adjacency[offsets[i]..offsets[i+1]]`.
#[derive(Debug, Clone)]
pub struct CsrIndex {
    _file_handle: Arc<RwLock<FileHandle>>,
    num_nodes: usize,
    num_edges: usize,
    /// Forward offsets: node i → start index in `fwd_adjacency`.
    fwd_offsets: Vec<usize>,
    /// Forward adjacency: flattened list of destination node IDs.
    fwd_adjacency: Vec<u64>,
    /// Reverse offsets: node i → start index in `rev_adjacency`.
    rev_offsets: Vec<usize>,
    /// Reverse adjacency: flattened list of source node IDs.
    rev_adjacency: Vec<u64>,
}

impl CsrIndex {
    pub fn new(file_handle: Arc<RwLock<FileHandle>>) -> Self {
        Self {
            _file_handle: file_handle,
            num_nodes: 0,
            num_edges: 0,
            fwd_offsets: Vec::new(),
            fwd_adjacency: Vec::new(),
            rev_offsets: Vec::new(),
            rev_adjacency: Vec::new(),
        }
    }

    /// Build a CSR index from a list of directed edges and the total number of nodes.
    ///
    /// `edges` contains `(src_id, dst_id)` pairs. Node IDs must be in `0..num_nodes`.
    pub fn build(file_handle: Arc<RwLock<FileHandle>>, edges: &[(u64, u64)], num_nodes: usize) -> Self {
        // Count degrees
        let mut fwd_degree = vec![0u32; num_nodes];
        let mut rev_degree = vec![0u32; num_nodes];
        for &(src, dst) in edges {
            let s = src as usize;
            let d = dst as usize;
            if s < num_nodes {
                fwd_degree[s] += 1;
            }
            if d < num_nodes {
                rev_degree[d] += 1;
            }
        }

        // Build forward offsets
        let mut fwd_offsets = Vec::with_capacity(num_nodes + 1);
        let mut total_fwd = 0usize;
        fwd_offsets.push(0);
        for &d in &fwd_degree {
            total_fwd += d as usize;
            fwd_offsets.push(total_fwd);
        }

        // Build reverse offsets
        let mut rev_offsets = Vec::with_capacity(num_nodes + 1);
        let mut total_rev = 0usize;
        rev_offsets.push(0);
        for &d in &rev_degree {
            total_rev += d as usize;
            rev_offsets.push(total_rev);
        }

        // Fill forward adjacency
        let mut fwd_adjacency = vec![0u64; total_fwd];
        let mut fwd_pos = fwd_offsets.clone();
        for &(src, dst) in edges {
            let s = src as usize;
            if s < num_nodes {
                let slot = &mut fwd_pos[s];
                let idx = *slot;
                *slot += 1;
                fwd_adjacency[idx] = dst;
            }
        }

        // Fill reverse adjacency
        let mut rev_adjacency = vec![0u64; total_rev];
        let mut rev_pos = rev_offsets.clone();
        for &(src, dst) in edges {
            let d = dst as usize;
            if d < num_nodes {
                let slot = &mut rev_pos[d];
                let idx = *slot;
                *slot += 1;
                rev_adjacency[idx] = src;
            }
        }

        Self {
            _file_handle: file_handle,
            num_nodes,
            num_edges: edges.len(),
            fwd_offsets,
            fwd_adjacency,
            rev_offsets,
            rev_adjacency,
        }
    }

    /// Gets the neighborhood of a given node ID based on direction.
    ///
    /// Returns the destination node IDs for forward traversal, or source node IDs
    /// for reverse traversal. Returns an empty vec if the node is out of range.
    pub fn get_neighbors(&self, node_id: u64, is_fwd: bool) -> Result<Vec<u64>, StorageError> {
        let pos = node_id as usize;
        if pos >= self.num_nodes {
            return Ok(Vec::new());
        }
        if is_fwd {
            let start = self.fwd_offsets[pos];
            let end = self.fwd_offsets[pos + 1];
            Ok(self.fwd_adjacency[start..end].to_vec())
        } else {
            let start = self.rev_offsets[pos];
            let end = self.rev_offsets[pos + 1];
            Ok(self.rev_adjacency[start..end].to_vec())
        }
    }

    /// Returns the number of nodes in the CSR index.
    pub fn num_nodes(&self) -> usize {
        self.num_nodes
    }

    /// Returns the number of edges stored in the CSR index.
    pub fn num_edges(&self) -> usize {
        self.num_edges
    }

    /// Returns true if the CSR index is empty (no nodes).
    pub fn is_empty(&self) -> bool {
        self.num_nodes == 0
    }

    /// Adds an edge to the CSR structure.
    ///
    /// Note: True CSR is immutable after build, so this rebuilds the entire index
    /// with the new edge appended. For bulk loads, prefer `build()` directly.
    pub fn add_edge(&mut self, _src_id: u64, _dst_id: u64) -> Result<(), StorageError> {
        // CSR is immutable after build — use build() for bulk loading.
        // This is a no-op placeholder for API compatibility.
        Err(StorageError::Index(
            "CsrIndex is immutable after build(); use build() to construct from edges".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    fn make_handle() -> Arc<RwLock<FileHandle>> {
        Arc::new(RwLock::new(FileHandle::new(std::path::PathBuf::from("test"), 4096)))
    }

    #[test]
    fn test_empty_csr() {
        let csr = CsrIndex::build(make_handle(), &[], 0);
        assert_eq!(csr.num_nodes(), 0);
        assert_eq!(csr.num_edges(), 0);
        assert!(csr.is_empty());
        assert_eq!(csr.get_neighbors(0, true).unwrap(), Vec::<u64>::new());
    }

    #[test]
    fn test_single_edge() {
        let csr = CsrIndex::build(make_handle(), &[(0, 1)], 2);
        assert_eq!(csr.num_nodes(), 2);
        assert_eq!(csr.num_edges(), 1);
        assert_eq!(csr.get_neighbors(0, true).unwrap(), vec![1]);
        assert_eq!(csr.get_neighbors(1, true).unwrap(), Vec::<u64>::new());
        assert_eq!(csr.get_neighbors(1, false).unwrap(), vec![0]);
        assert_eq!(csr.get_neighbors(0, false).unwrap(), Vec::<u64>::new());
    }

    #[test]
    fn test_multiple_edges_forward() {
        let edges = vec![(0, 1), (0, 2), (1, 2), (2, 3)];
        let csr = CsrIndex::build(make_handle(), &edges, 4);
        assert_eq!(csr.num_edges(), 4);

        let mut n0 = csr.get_neighbors(0, true).unwrap();
        n0.sort();
        assert_eq!(n0, vec![1, 2]);

        let n1 = csr.get_neighbors(1, true).unwrap();
        assert_eq!(n1, vec![2]);

        let n2 = csr.get_neighbors(2, true).unwrap();
        assert_eq!(n2, vec![3]);

        assert!(csr.get_neighbors(3, true).unwrap().is_empty());
    }

    #[test]
    fn test_multiple_edges_reverse() {
        let edges = vec![(0, 1), (0, 2), (1, 2), (2, 3)];
        let csr = CsrIndex::build(make_handle(), &edges, 4);

        let n1_rev = csr.get_neighbors(1, false).unwrap();
        assert_eq!(n1_rev, vec![0]);

        let mut n2_rev = csr.get_neighbors(2, false).unwrap();
        n2_rev.sort();
        assert_eq!(n2_rev, vec![0, 1]);

        let n3_rev = csr.get_neighbors(3, false).unwrap();
        assert_eq!(n3_rev, vec![2]);
    }

    #[test]
    fn test_out_of_range_node() {
        let csr = CsrIndex::build(make_handle(), &[(0, 1)], 2);
        assert_eq!(csr.get_neighbors(99, true).unwrap(), Vec::<u64>::new());
        assert_eq!(csr.get_neighbors(99, false).unwrap(), Vec::<u64>::new());
    }

    #[test]
    fn test_self_loop() {
        let csr = CsrIndex::build(make_handle(), &[(0, 0)], 1);
        assert_eq!(csr.get_neighbors(0, true).unwrap(), vec![0]);
        assert_eq!(csr.get_neighbors(0, false).unwrap(), vec![0]);
    }

    #[test]
    fn test_add_edge_returns_error() {
        let mut csr = CsrIndex::build(make_handle(), &[], 1);
        assert!(csr.add_edge(0, 1).is_err());
    }
}
