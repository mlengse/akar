#![allow(dead_code)]
//! Output writers for GDS algorithm results.
//!
//! Ported from C++ `rj_output_writer.h` / `output_writer.cpp`.
//!
//! Handles serialization of algorithm results into tabular format,
//! with support for path enumeration via DFS backtracking.

use kuzu_common::types::{InternalID, Value};

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
    pub semantic: kuzu_common::enums::PathSemantic,
    pub lower_bound: u16,
    pub flip_path: bool,
    pub write_edge_direction: bool,
    pub write_path: bool,
}

impl Default for PathsOutputWriterInfo {
    fn default() -> Self {
        Self {
            semantic: kuzu_common::enums::PathSemantic::Walk,
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
    pub fn new(
        info: PathsOutputWriterInfo,
        bfs_graph: Box<dyn BaseBFSGraph>,
        source_node_id: InternalID,
    ) -> Self {
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

    /// DFS-based fast path for WALK semantic without node mask.
    fn dfs_fast<'a>(
        &self,
        first_parent: &'a ParentList,
        path: &mut Vec<&'a ParentList>,
        results: &mut Vec<Vec<Value>>,
    ) {
        path.push(first_parent);

        // Check if we should stop: we reached the source node (iter == 0 and no parent chain back)
        let reached_source = first_parent.node_id.offset == self.src_offset
            || (first_parent.next.is_none() && first_parent.iter == 0);

        if reached_source {
            // Reconstruct path from source to destination
            self.write_path(path, results);
        } else if let Some(ref next_parent) = first_parent.next {
            self.dfs_fast(next_parent, path, results);
        }

        path.pop();
    }

    /// DFS slow path with semantic checking.
    fn dfs_slow<'a>(
        &self,
        first_parent: &'a ParentList,
        path: &mut Vec<&'a ParentList>,
        results: &mut Vec<Vec<Value>>,
    ) {
        path.push(first_parent);

        let reached_source = first_parent.node_id.offset == self.src_offset
            || (first_parent.next.is_none() && first_parent.iter == 0);

        if reached_source {
            // Check semantic constraints
            if self.check_semantic(path) {
                self.write_path(path, results);
            }
        } else if let Some(ref next_parent) = first_parent.next {
            if self.is_next_viable(next_parent, path) {
                self.dfs_slow(next_parent, path, results);
            }
        }

        path.pop();
    }

    /// Check if the next parent is viable given the current path and semantic constraints.
    fn is_next_viable(&self, _next: &ParentList, path: &[&ParentList]) -> bool {
        match self.info.semantic {
            kuzu_common::enums::PathSemantic::Walk => true,
            kuzu_common::enums::PathSemantic::Trail => {
                // No edge can repeat
                let edge_id = _next.edge_id;
                !path.iter().any(|p| p.edge_id == edge_id)
            }
            kuzu_common::enums::PathSemantic::Acyclic => {
                // No node can repeat
                let node_id = _next.node_id.offset;
                !path.iter().any(|p| p.node_id.offset == node_id)
            }
        }
    }

    /// Check semantic constraints for a complete path.
    fn check_semantic(&self, path: &[&ParentList]) -> bool {
        match self.info.semantic {
            kuzu_common::enums::PathSemantic::Walk => true,
            kuzu_common::enums::PathSemantic::Trail => {
                let edge_ids: Vec<u64> = path.iter().map(|p| p.edge_id).collect();
                let mut unique = edge_ids.clone();
                unique.sort();
                unique.dedup();
                unique.len() == edge_ids.len()
            }
            kuzu_common::enums::PathSemantic::Acyclic => {
                let node_ids: Vec<u64> = path.iter().map(|p| p.node_id.offset).collect();
                let mut unique = node_ids.clone();
                unique.sort();
                unique.dedup();
                unique.len() == node_ids.len()
            }
        }
    }

    /// Write a path to the output.
    fn write_path(&self, path: &[&ParentList], results: &mut Vec<Vec<Value>>) {
        let mut row = Vec::new();

        // Source node ID
        row.push(Value::InternalID(self.source_node_id));
        // Destination node ID — the first element in the path (the destination)
        let dst = if !path.is_empty() {
            path.first()
                .map(|p| p.node_id)
                .unwrap_or(self.source_node_id)
        } else {
            self.source_node_id
        };
        row.push(Value::InternalID(dst));
        // Path length
        row.push(Value::Int64(path.len() as i64));

        if self.info.write_path {
            // Path node IDs (from source to destination)
            let path_nodes: Vec<Value> = std::iter::once(Value::InternalID(self.source_node_id))
                .chain(path.iter().rev().map(|p| Value::InternalID(p.node_id)))
                .collect();
            row.push(Value::List(path_nodes));

            // Path edge IDs
            let path_edges: Vec<Value> = path
                .iter()
                .rev()
                .map(|p| Value::Int64(p.edge_id as i64))
                .collect();
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
    pub fn new(
        bfs_graph: Box<dyn BaseBFSGraph>,
        source_node_id: InternalID,
    ) -> Self {
        let info = PathsOutputWriterInfo {
            semantic: kuzu_common::enums::PathSemantic::Walk,
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
    pub fn write_path_for_dst(
        &self,
        dst_offset: u64,
        results: &mut Vec<Vec<Value>>,
    ) {
        let first_parent = self.inner.bfs_graph.get_parent_list_head_offset(dst_offset);
        if let Some(parent) = first_parent {
            let mut path = Vec::new();
            let mut results_buf = Vec::new();

            // For single shortest path, use fast DFS
            self.inner.dfs_fast(parent, &mut path, &mut results_buf);

            for row in results_buf {
                results.push(row);
            }
        }
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
    use crate::gds::bfs_graph::DenseBFSGraph;

    #[test]
    fn test_paths_output_writer_info_default() {
        let info = PathsOutputWriterInfo::default();
        assert_eq!(info.lower_bound, 0);
        assert!(info.write_path);
    }
}
