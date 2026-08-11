//! Builds a `GraphDataSource` snapshot from the storage `TableCatalog`.
//!
//! The query processor and the connection layer own the storage catalog, so
//! they construct this adapter and hand it to GDS table functions through the
//! `TableFunction::CustomTableWithGraph` variant.

use akar_function::graph::{GraphDataSource, GraphEdge};
use akar_storage::table::TableCatalog;
use std::sync::Arc;

/// A `GraphDataSource` snapshot over the live node/rel tables of a catalog.
#[derive(Debug, Default)]
pub struct CatalogGraphSource {
    num_nodes: usize,
    edges: Vec<GraphEdge>,
}

impl CatalogGraphSource {
    /// Snapshot the graph topology. `None` (no catalog) yields an empty graph.
    pub fn new(catalog: Option<&Arc<TableCatalog>>) -> Self {
        let Some(catalog) = catalog else {
            return Self::default();
        };

        let mut num_nodes = 0usize;
        let mut edges = Vec::new();

        for node_table in catalog.all_node_tables() {
            num_nodes = num_nodes.max(node_table.num_rows as usize);
        }
        for rel_table in catalog.all_rel_tables() {
            for (edge_idx, &(src, dst)) in rel_table.edges.iter().enumerate() {
                if src == u64::MAX || dst == u64::MAX {
                    continue;
                }
                edges.push(GraphEdge {
                    src_offset: src,
                    dst_offset: dst,
                    rel_id: edge_idx as u64,
                    rel_table_id: rel_table.table_id,
                });
                num_nodes = num_nodes.max(src as usize + 1).max(dst as usize + 1);
            }
        }

        Self { num_nodes, edges }
    }
}

impl GraphDataSource for CatalogGraphSource {
    fn num_nodes(&self) -> usize {
        self.num_nodes
    }

    fn edges(&self) -> Vec<GraphEdge> {
        self.edges.clone()
    }
}
