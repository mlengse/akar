//! GDS (Graph Data Science) framework for Kuzu.
//!
//! Provides the core infrastructure for graph algorithm execution:
//! - Frontier management (sparse/dense frontier tracking)
//! - EdgeCompute and VertexCompute abstractions
//! - BFS graph for path tracking (ParentList)
//! - Output writers for algorithm results
//! - Execution utilities (GDSUtils)
//!
//! This module is the Rust port of the C++ GDS framework
//! originally in `src/function/gds/`.

pub mod bfs_graph;
pub mod compute;
pub mod frontier;
pub mod output_writer;
pub mod utils;

pub use bfs_graph::{BFSGraphManager, BaseBFSGraph, DenseBFSGraph, ParentList, SparseBFSGraph};
pub use compute::{EdgeCompute, VertexCompute};
pub use frontier::{
    DenseFrontier, DenseFrontierPair, DenseFrontierReference, DenseSparseDynamicFrontierPair, FRONTIER_INITIAL_VISITED,
    FRONTIER_UNVISITED, Frontier, FrontierPair, GDSDensityState, Iteration, SPFrontierPair, SparseFrontier,
    SparseFrontierReference,
};
pub use output_writer::{PathsOutputWriter, PathsOutputWriterInfo, RJOutputWriter, SPPathsOutputWriter};
pub use utils::GDSUtils;
