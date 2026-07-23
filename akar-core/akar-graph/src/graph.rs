//! Graph data structures for graph traversal and storage.
//!
//! Uses CSR (Compressed Sparse Row) format for adjacency storage,
//! supporting both in-memory and on-disk graph representations.

use akar_common::types::InternalID;
use akar_storage::table::RelTable;

/// An entry in the graph representing a node label's adjacency.
#[derive(Debug, Clone)]
pub struct GraphEntry {
    pub node_label: String,
    pub node_table_id: u64,
    pub rel_label: String,
    pub rel_table_id: u64,
    pub is_directed: bool,
}

/// A directed or undirected edge in the graph.
#[derive(Debug, Clone)]
pub struct Edge {
    pub src_offset: u64,
    pub dst_offset: u64,
    pub rel_id: u64,
    pub rel_table_id: u64,
}

/// CSR (Compressed Sparse Row) adjacency representation.
///
/// Efficient for traversal: for a node at position `i`, its neighbors
/// are at `adjacency[offsets[i]..offsets[i+1]]`.
#[derive(Debug, Clone)]
pub struct CSRAdjacency {
    /// For each node, the start offset in `adjacency`.
    pub offsets: Vec<usize>,
    /// Flat adjacency list — all neighbors concatenated.
    pub adjacency: Vec<(u64, InternalID)>, // (rel_id, dst_node_id)
}

impl CSRAdjacency {
    pub fn new(num_nodes: usize) -> Self {
        Self {
            offsets: vec![0; num_nodes + 1],
            adjacency: Vec::new(),
        }
    }

    /// Get neighbors for a node by its position.
    pub fn neighbors(&self, node_pos: usize) -> &[(u64, InternalID)] {
        let start = self.offsets[node_pos];
        let end = self.offsets[node_pos + 1];
        &self.adjacency[start..end]
    }

    pub fn num_nodes(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    pub fn num_edges(&self) -> usize {
        self.adjacency.len()
    }

    /// Build from a list of edges.
    pub fn build(edges: &[Edge], num_nodes: usize) -> Self {
        let mut offsets = vec![0; num_nodes + 1];
        let mut adj = Vec::new();

        // Count degree per node (undirected: count both directions)
        let mut degree = vec![0u32; num_nodes];
        for edge in edges {
            degree[edge.src_offset as usize] += 1;
            degree[edge.dst_offset as usize] += 1;
        }

        // Build offsets
        let mut current = 0;
        for (i, d) in degree.iter().enumerate() {
            offsets[i] = current;
            current += *d as usize;
        }
        offsets[num_nodes] = current;
        adj.resize(current, (0, InternalID { table_id: 0, offset: 0 }));

        // Fill adjacency (temporary position tracking)
        let mut pos = offsets.clone();
        for edge in edges {
            let src = edge.src_offset as usize;
            let dst = edge.dst_offset as usize;
            adj[pos[src]] = (
                edge.rel_id,
                InternalID {
                    table_id: edge.rel_table_id,
                    offset: edge.dst_offset,
                },
            );
            pos[src] += 1;
            adj[pos[dst]] = (
                edge.rel_id,
                InternalID {
                    table_id: edge.rel_table_id,
                    offset: edge.src_offset,
                },
            );
            pos[dst] += 1;
        }

        Self {
            offsets,
            adjacency: adj,
        }
    }
}

/// In-memory graph representation for traversal and algorithms.
#[derive(Debug, Default)]
pub struct Graph {
    /// Graph metadata entries.
    entries: Vec<GraphEntry>,
    /// CSR adjacency (node_offset → neighbors).
    csr: Option<CSRAdjacency>,
    /// Number of nodes in the graph.
    num_nodes: u64,
}

impl Graph {
    /// Create an empty graph (no nodes or edges).
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a labeled graph entry (node table or edge table) to the graph.
    pub fn add_entry(&mut self, entry: GraphEntry) {
        self.entries.push(entry);
    }

    /// Return all graph entries (node/edge tables) registered in this graph.
    pub fn entries(&self) -> &[GraphEntry] {
        &self.entries
    }

    /// Build the CSR adjacency from a list of edges.
    pub fn build_adjacency(&mut self, edges: &[Edge]) {
        let num_nodes = edges
            .iter()
            .flat_map(|e| [e.src_offset, e.dst_offset])
            .max()
            .map(|m| m as usize + 1)
            .unwrap_or(0);
        self.num_nodes = num_nodes as u64;
        self.csr = Some(CSRAdjacency::build(edges, num_nodes));
    }

    /// Return the neighbors of `node_offset` as `(edge_weight, target_internal_id)` pairs.
    /// Returns `None` if CSR is not built or `node_offset` is out of range.
    pub fn get_neighbors(&self, node_offset: u64) -> Option<&[(u64, InternalID)]> {
        self.csr.as_ref().and_then(|csr| {
            let pos = node_offset as usize;
            if pos < csr.num_nodes() {
                Some(csr.neighbors(pos))
            } else {
                None
            }
        })
    }

    /// Return the number of nodes in the graph.
    pub fn num_nodes(&self) -> u64 {
        self.num_nodes
    }

    /// Return the number of edges in the graph.
    pub fn num_edges(&self) -> usize {
        self.csr.as_ref().map(|c| c.num_edges()).unwrap_or(0)
    }

    /// Return `true` if the CSR adjacency index has been built.
    pub fn has_csr(&self) -> bool {
        self.csr.is_some()
    }
}

/// On-disk graph that reads adjacency from storage tables.
pub struct OnDiskGraph;

impl Default for OnDiskGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl OnDiskGraph {
    pub fn new() -> Self {
        Self
    }

    /// Scan a relationship table and build an in-memory graph.
    pub fn build_from_storage(&self, rel_table: &RelTable) -> Result<Graph, String> {
        let mut graph = Graph::new();

        graph.add_entry(GraphEntry {
            node_label: String::new(),
            node_table_id: rel_table.src_table_id,
            rel_label: rel_table.name.clone(),
            rel_table_id: rel_table.table_id,
            is_directed: false,
        });

        let edges: Vec<Edge> = rel_table
            .edges
            .iter()
            .enumerate()
            .filter(|&(_, &(src, dst))| src != u64::MAX && dst != u64::MAX)
            .map(|(edge_idx, &(src, dst))| Edge {
                src_offset: src,
                dst_offset: dst,
                rel_id: edge_idx as u64,
                rel_table_id: rel_table.table_id,
            })
            .collect();

        graph.build_adjacency(&edges);
        Ok(graph)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_storage::table::RelTable;

    fn sample_edges() -> Vec<Edge> {
        vec![
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
        ]
    }

    fn sample_rel_table() -> RelTable {
        RelTable::new(1, "knows".into(), 10, 20, vec![])
    }

    #[test]
    fn test_create_graph_entry() {
        let mut g = Graph::new();
        g.add_entry(GraphEntry {
            node_label: "Person".into(),
            node_table_id: 0,
            rel_label: "Knows".into(),
            rel_table_id: 1,
            is_directed: false,
        });
        assert_eq!(g.entries().len(), 1);
        assert_eq!(g.entries()[0].node_label, "Person");
    }

    #[test]
    fn test_csr_build() {
        let edges = sample_edges();
        let csr = CSRAdjacency::build(&edges, 4);
        assert_eq!(csr.num_nodes(), 4);
        assert_eq!(csr.num_edges(), 10); // 5 undirected edges = 10 directed entries
    }

    #[test]
    fn test_csr_neighbors() {
        let edges = sample_edges();
        let csr = CSRAdjacency::build(&edges, 4);
        let n0 = csr.neighbors(0);
        assert!(!n0.is_empty());
        // Node 0 is connected to nodes 1, 2, 3
        let dsts: Vec<u64> = n0.iter().map(|(_, id)| id.offset).collect();
        assert!(dsts.contains(&1));
        assert!(dsts.contains(&2));
        assert!(dsts.contains(&3));
    }

    #[test]
    fn test_graph_build_adjacency() {
        let edges = sample_edges();
        let mut g = Graph::new();
        g.add_entry(GraphEntry {
            node_label: "Test".into(),
            node_table_id: 0,
            rel_label: "R".into(),
            rel_table_id: 0,
            is_directed: false,
        });
        g.build_adjacency(&edges);
        assert!(g.has_csr());
        assert_eq!(g.num_nodes(), 4);
        assert_eq!(g.num_edges(), 10);
    }

    #[test]
    fn test_get_neighbors() {
        let edges = sample_edges();
        let mut g = Graph::new();
        g.build_adjacency(&edges);

        let n0 = g.get_neighbors(0).unwrap();
        assert!(!n0.is_empty());

        let n4 = g.get_neighbors(4);
        assert!(n4.is_none()); // Node 4 doesn't exist
    }

    #[test]
    fn test_empty_graph() {
        let g = Graph::new();
        assert_eq!(g.num_nodes(), 0);
        assert_eq!(g.num_edges(), 0);
        assert!(!g.has_csr());
    }

    #[test]
    fn test_csr_single_node() {
        let edges = vec![Edge {
            src_offset: 0,
            dst_offset: 0,
            rel_id: 0,
            rel_table_id: 0,
        }];
        let csr = CSRAdjacency::build(&edges, 1);
        assert_eq!(csr.num_nodes(), 1);
        assert_eq!(csr.num_edges(), 2); // Self-loop counted twice in undirected
    }

    #[test]
    fn test_build_from_storage_empty() {
        let rel_table = sample_rel_table();
        let disk = OnDiskGraph::new();
        let graph = disk.build_from_storage(&rel_table).unwrap();
        assert_eq!(graph.num_nodes(), 0);
        assert_eq!(graph.num_edges(), 0);
        assert_eq!(graph.entries().len(), 1);
        assert_eq!(graph.entries()[0].rel_table_id, 1);
        assert_eq!(graph.entries()[0].rel_label, "knows");
    }

    #[test]
    fn test_build_from_storage_with_edges() {
        let mut rel_table = sample_rel_table();
        rel_table.insert_rel(0, 1, vec![]).unwrap();
        rel_table.insert_rel(0, 2, vec![]).unwrap();
        rel_table.insert_rel(1, 2, vec![]).unwrap();

        let disk = OnDiskGraph::new();
        let graph = disk.build_from_storage(&rel_table).unwrap();
        assert_eq!(graph.num_nodes(), 3);
        assert_eq!(graph.num_edges(), 6); // 3 undirected = 6 directed
        let n0 = graph.get_neighbors(0).unwrap();
        assert_eq!(n0.len(), 2); // connected to 1 and 2
    }

    #[test]
    fn test_build_from_storage_skips_tombstones() {
        let mut rel_table = sample_rel_table();
        rel_table.insert_rel(0, 1, vec![]).unwrap();
        rel_table.insert_rel(0, 2, vec![]).unwrap();
        // tombstones are edges with u64::MAX
        rel_table.edges[1] = (u64::MAX, u64::MAX);

        let disk = OnDiskGraph::new();
        let graph = disk.build_from_storage(&rel_table).unwrap();
        assert_eq!(graph.num_edges(), 2); // only 1 valid edge = 2 directed
    }
}
