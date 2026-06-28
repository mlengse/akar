//! Graph data structures for graph traversal and storage.

use kuzu_common::types::InternalID;
use std::collections::HashMap;

/// An entry in the graph representing a node label's adjacency.
#[derive(Debug, Clone)]
pub struct GraphEntry {
    pub node_label: String,
    pub node_table_id: u64,
    pub rel_label: String,
    pub rel_table_id: u64,
    pub is_directed: bool,
}

/// In-memory graph representation for traversal.
#[derive(Debug, Default)]
pub struct Graph {
    entries: Vec<GraphEntry>,
    adjacency: HashMap<u64, Vec<(u64, InternalID)>>, // src_node_offset → [(rel_id, dst_node_id)]
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_entry(&mut self, entry: GraphEntry) {
        self.entries.push(entry);
    }

    pub fn entries(&self) -> &[GraphEntry] {
        &self.entries
    }

    pub fn add_edge(&mut self, src: u64, rel_id: u64, dst: InternalID) {
        self.adjacency.entry(src).or_default().push((rel_id, dst));
    }

    pub fn get_neighbors(&self, node_offset: u64) -> Option<&[(u64, InternalID)]> {
        self.adjacency.get(&node_offset).map(|v| v.as_slice())
    }
}
