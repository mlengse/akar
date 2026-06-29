//! Graph data structures and traversal algorithms.
//!
//! Features CSR adjacency format, BFS, PageRank, WCC,
//! shortest path, and degree centrality.

pub mod algorithms;
pub mod graph;

pub use algorithms::{
    AlgorithmResult, bfs, degree_centrality, page_rank, reachable_within, shortest_path, weakly_connected_components,
};
pub use graph::{CSRAdjacency, Edge, Graph, GraphEntry, OnDiskGraph};
