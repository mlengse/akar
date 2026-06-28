//! Graph data structures and traversal algorithms.
//!
//! Features CSR adjacency format, BFS, PageRank, WCC,
//! shortest path, and degree centrality.

pub mod graph;
pub mod algorithms;

pub use graph::{Graph, GraphEntry, Edge, CSRAdjacency, OnDiskGraph};
pub use algorithms::{
    bfs, page_rank, weakly_connected_components,
    shortest_path, reachable_within, degree_centrality,
    AlgorithmResult,
};
