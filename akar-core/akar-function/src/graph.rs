//! Graph data source abstraction for table functions that need graph access.
//!
//! `akar-function` cannot depend on `akar-graph` (the dependency graph runs
//! `akar-graph -> akar-storage -> akar-vector -> akar-function`), so graph
//! algorithms and table-function closures receive graph data through this
//! trait instead. The query processor (which owns the storage `TableCatalog`)
//! builds a concrete `GraphDataSource` from the catalog's node/rel tables.

/// A directed or undirected edge in the database graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEdge {
    /// Offset of the source node within its node table.
    pub src_offset: u64,
    /// Offset of the destination node within its node table.
    pub dst_offset: u64,
    /// Row index of the edge within its relationship table.
    pub rel_id: u64,
    /// ID of the relationship table that owns this edge.
    pub rel_table_id: u64,
}

/// Provides the node/edge topology backing the graph for GDS table functions.
///
/// Implementors snapshot the graph at call time. Edges whose source or
/// destination is `u64::MAX` (soft-deleted rows) are excluded.
pub trait GraphDataSource {
    /// Total number of nodes in the graph (at least `max_offset + 1`).
    fn num_nodes(&self) -> usize;

    /// All live edges of the graph.
    fn edges(&self) -> Vec<GraphEdge>;
}
