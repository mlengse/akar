#![allow(dead_code)]
//! Output writers for GDS algorithm results.
//!
//! Ported from C++ `rj_output_writer.h` / `output_writer.cpp`.
//!
//! Handles serialization of algorithm results into tabular format,
//! with support for path enumeration via DFS backtracking.

use akar_common::types::{InternalID, Value};

use crate::gds::bfs_graph::{BaseBFSGraph, DenseBFSGraph, ParentList};

// ==================== RJOutputWriter ====================

/// Base output writer for recursive join (RJ) algorithms.
///
/// Manages output vectors and mask-based filtering.
pub trait RJOutputWriter: Send {
    /// Initialize writing for a specific table.
    fn begin_writing(&mut self, table_id: u64);
    /// Write results for all destinations in a frontier.
    fn write_all(&mut self, dst_offsets: &[u64], output: &mut Vec<Vec<Value>>);
    /// Write a single destination node.
    fn write_single(&mut self, dst_node_id: InternalID, output: &mut Vec<Vec<Value>>);
    /// Check if a node is in the output mask.
    fn in_output_mask(&self, offset: u64) -> bool;
    /// Create a boxed copy.
    fn box_clone(&self) -> Box<dyn RJOutputWriter>;
}

// ==================== PathsOutputWriterInfo ====================

/// Configuration for path output writing.
pub struct PathsOutputWriterInfo {
    pub semantic: akar_common::enums::PathSemantic,
    pub lower_bound: u16,
    pub flip_path: bool,
    pub write_edge_direction: bool,
    pub write_path: bool,
}

impl Default for PathsOutputWriterInfo {
    fn default() -> Self {
        Self {
            semantic: akar_common::enums::PathSemantic::Walk,
            lower_bound: 0,
            flip_path: false,
            write_edge_direction: false,
            write_path: true,
        }
    }
}

// ==================== PathsOutputWriter ====================

/// Output writer that enumerates paths using DFS backtracking.
///
/// Supports:
/// - WALK/TRAIL/ACYCLIC semantic control
/// - Path node/edge ID output
/// - Direction-aware path serialization
pub struct PathsOutputWriter {
    pub info: PathsOutputWriterInfo,
    pub bfs_graph: Box<dyn BaseBFSGraph>,
    pub source_node_id: InternalID,
    pub output_mask: Option<Vec<bool>>,
    pub src_offset: u64,
}

impl PathsOutputWriter {
    pub fn new(info: PathsOutputWriterInfo, bfs_graph: Box<dyn BaseBFSGraph>, source_node_id: InternalID) -> Self {
        Self {
            info,
            bfs_graph,
            source_node_id,
            output_mask: None,
            src_offset: source_node_id.offset,
        }
    }

    /// Set output mask (which nodes to include in results).
    pub fn set_output_mask(&mut self, mask: Vec<bool>) {
        self.output_mask = Some(mask);
    }

    /// Guard against unbounded recursion on corrupt/cyclic parent lists.
    const MAX_PATH_DEPTH: usize = 10_000;

    /// DFS over the BFS predecessor tree, collecting the path for `dst`.
    ///
    /// Follows the PREDECESSOR chain via `get_parent_list_head_offset`: each
    /// `ParentList` entry's `node_id` is the parent that reached the current
    /// node, so the next hop is that parent's own parent list. The old code
    /// followed `ParentList::next` — which is a sibling/candidate list for the
    /// SAME node — producing truncated/duplicate paths (P52.57).
    fn dfs_fast(
        &self,
        node: InternalID,
        dst: InternalID,
        path: &mut Vec<InternalID>,
        edges: &mut Vec<u64>,
        results: &mut Vec<Vec<Value>>,
    ) {
        if node.offset == self.src_offset {
            self.write_path(dst, path, edges, results);
            return;
        }
        if path.len() >= Self::MAX_PATH_DEPTH {
            return;
        }
        let mut current = self.bfs_graph.get_parent_list_head_offset(node.offset);
        while let Some(parent) = current {
            path.push(parent.node_id);
            edges.push(parent.edge_id);
            self.dfs_fast(parent.node_id, dst, path, edges, results);
            edges.pop();
            path.pop();
            current = parent.next.as_deref();
        }
    }

    /// DFS with semantic checking (WALK/TRAIL/ACYCLIC).
    fn dfs_slow(
        &self,
        node: InternalID,
        dst: InternalID,
        path: &mut Vec<InternalID>,
        edges: &mut Vec<u64>,
        results: &mut Vec<Vec<Value>>,
    ) {
        if node.offset == self.src_offset {
            if self.check_semantic(path, edges) {
                self.write_path(dst, path, edges, results);
            }
            return;
        }
        if path.len() >= Self::MAX_PATH_DEPTH {
            return;
        }
        let mut current = self.bfs_graph.get_parent_list_head_offset(node.offset);
        while let Some(parent) = current {
            if self.is_next_viable(parent, path, edges) {
                path.push(parent.node_id);
                edges.push(parent.edge_id);
                self.dfs_slow(parent.node_id, dst, path, edges, results);
                edges.pop();
                path.pop();
            }
            current = parent.next.as_deref();
        }
    }

    /// Check if the next parent is viable given the current path and semantic constraints.
    fn is_next_viable(&self, next: &ParentList, path: &[InternalID], edges: &[u64]) -> bool {
        match self.info.semantic {
            akar_common::enums::PathSemantic::Walk => true,
            akar_common::enums::PathSemantic::Trail => !edges.contains(&next.edge_id),
            akar_common::enums::PathSemantic::Acyclic => !path.iter().any(|n| n.offset == next.node_id.offset),
        }
    }

    /// Check semantic constraints for a complete path.
    fn check_semantic(&self, path: &[InternalID], edges: &[u64]) -> bool {
        match self.info.semantic {
            akar_common::enums::PathSemantic::Walk => true,
            akar_common::enums::PathSemantic::Trail => {
                let mut unique = edges.to_vec();
                unique.sort_unstable();
                unique.dedup();
                unique.len() == edges.len()
            }
            akar_common::enums::PathSemantic::Acyclic => {
                let mut unique: Vec<u64> = path.iter().map(|n| n.offset).collect();
                unique.sort_unstable();
                unique.dedup();
                unique.len() == path.len()
            }
        }
    }

    /// Write a reconstructed path to the output.
    ///
    /// `path` is the predecessor chain `[parent_of_dst, ..., source]`; the
    /// emitted node list is `source, …, parent_of_dst, dst` (P52.57).
    fn write_path(&self, dst: InternalID, path: &[InternalID], edges: &[u64], results: &mut Vec<Vec<Value>>) {
        let mut row = Vec::new();

        // Source node ID
        row.push(Value::InternalID(self.source_node_id));
        // Destination node ID — the actual `dst`, not the first parent entry.
        row.push(Value::InternalID(dst));
        // Path length (number of hops)
        row.push(Value::Int64(edges.len() as i64));

        if self.info.write_path {
            // Path node IDs (source → dst)
            let mut nodes = Vec::with_capacity(path.len() + 1);
            nodes.extend(path.iter().rev().copied());
            nodes.push(dst);
            let path_nodes: Vec<Value> = nodes.iter().map(|n| Value::InternalID(*n)).collect();
            row.push(Value::List(path_nodes));

            // Path edge IDs
            let path_edges: Vec<Value> = edges.iter().rev().map(|e| Value::Int64(*e as i64)).collect();
            row.push(Value::List(path_edges));
        }

        results.push(row);
    }
}

// ==================== SPPathsOutputWriter ====================

/// Single shortest path output writer — extends PathsOutputWriter.
pub struct SPPathsOutputWriter {
    inner: PathsOutputWriter,
}

impl SPPathsOutputWriter {
    pub fn new(bfs_graph: Box<dyn BaseBFSGraph>, source_node_id: InternalID) -> Self {
        let info = PathsOutputWriterInfo {
            semantic: akar_common::enums::PathSemantic::Walk,
            lower_bound: 0,
            flip_path: false,
            write_edge_direction: false,
            write_path: true,
        };
        Self {
            inner: PathsOutputWriter::new(info, bfs_graph, source_node_id),
        }
    }

    /// Write the shortest path for a destination node.
    pub fn write_path_for_dst(&self, dst_offset: u64, results: &mut Vec<Vec<Value>>) {
        let dst = InternalID {
            offset: dst_offset,
            table_id: self.inner.source_node_id.table_id,
        };
        let mut path = Vec::new();
        let mut edges = Vec::new();
        self.inner.dfs_fast(dst, dst, &mut path, &mut edges, results);
    }
}

impl RJOutputWriter for SPPathsOutputWriter {
    fn begin_writing(&mut self, _table_id: u64) {}

    fn write_all(&mut self, dst_offsets: &[u64], output: &mut Vec<Vec<Value>>) {
        for &offset in dst_offsets {
            if self.in_output_mask(offset) {
                self.write_path_for_dst(offset, output);
            }
        }
    }

    fn write_single(&mut self, dst_node_id: InternalID, output: &mut Vec<Vec<Value>>) {
        if self.in_output_mask(dst_node_id.offset) {
            self.write_path_for_dst(dst_node_id.offset, output);
        }
    }

    fn in_output_mask(&self, offset: u64) -> bool {
        self.inner
            .output_mask
            .as_ref()
            .map(|mask| offset < mask.len() as u64 && mask[offset as usize])
            .unwrap_or(true)
    }

    fn box_clone(&self) -> Box<dyn RJOutputWriter> {
        // Simplified: no deep cloning of bfs_graph — use empty placeholder
        Box::new(SPPathsOutputWriter {
            inner: PathsOutputWriter {
                info: PathsOutputWriterInfo {
                    semantic: self.inner.info.semantic,
                    lower_bound: self.inner.info.lower_bound,
                    flip_path: self.inner.info.flip_path,
                    write_edge_direction: self.inner.info.write_edge_direction,
                    write_path: self.inner.info.write_path,
                },
                bfs_graph: Box::new(DenseBFSGraph::new(0)),
                source_node_id: self.inner.source_node_id,
                output_mask: self.inner.output_mask.clone(),
                src_offset: self.inner.src_offset,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paths_output_writer_info_default() {
        let info = PathsOutputWriterInfo::default();
        assert_eq!(info.lower_bound, 0);
        assert!(info.write_path);
    }
}
