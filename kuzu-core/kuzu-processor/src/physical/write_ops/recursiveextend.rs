use crate::physical::write_ops::evaluate_expression_for_row;
use kuzu_common::types::{PhysicalTypeID, Value};
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_storage::table::TableCatalog;
use std::sync::Arc;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::physical::scan_filter::PhysicalScan;
use crate::physical::common::store_value_in_vector;

// ==================== RecursiveExtend ====================

/// Physical operator for variable-length path matching (BFS traversal).
///
/// For each source node, performs BFS up to `upper_bound` depth and emits
/// result rows for all nodes reachable at depths between `lower_bound` and
/// `upper_bound`.
///
/// Uses GDS-style path tracking to record actual paths (node IDs + edge IDs)
/// and enforces path semantics (WALK/TRAIL/ACYCLIC).
///
/// Produces a DataChunk with columns:
///   (src_offset, dst_offset, length, path_node_ids, path_edge_ids[, cost])
///
/// When `weight_property` is `Some`, uses Dijkstra's algorithm for weighted
/// shortest path traversal (port of C++ `WeightedSPPathsFunction`).
/// The `cost` column is appended to the output.
pub struct PhysicalRecursiveExtend {
    pub source_table_id: u64,
    pub rel_table_ids: Vec<u64>,
    pub lower_bound: u64,
    pub upper_bound: u64,
    pub direction: kuzu_common::enums::ExtendDirection,
    pub semantic: kuzu_common::enums::PathSemantic,
    pub table_catalog: Option<Arc<TableCatalog>>,
    /// Optional edge weight property name for weighted shortest path.
    /// When set, Dijkstra traversal is used instead of BFS.
    pub weight_property: Option<String>,
    /// Optional name for the cost output column.
    pub cost_output_name: Option<String>,
}

impl PhysicalOperatorExec for PhysicalRecursiveExtend {
    fn operator_type(&self) -> &str {
        "recursive_extend"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        use kuzu_common::enums::ExtendDirection;
        use kuzu_common::enums::PathSemantic;
        use kuzu_common::types::Value;
        use kuzu_common::vector::ValueVector;
        use std::collections::{HashMap, VecDeque};

        let catalog = self
            .table_catalog
            .as_ref()
            .ok_or_else(|| "No table catalog available for RecursiveExtend".to_string())?;

        // Build adjacency with edge IDs: neighbor_offset -> (neighbor_offset, edge_id)
        let mut fwd_adj: HashMap<u64, Vec<(u64, u64)>> = HashMap::new();
        let mut rev_adj: HashMap<u64, Vec<(u64, u64)>> = HashMap::new();
        // Edge weight lookup: edge_id -> weight (for weighted shortest path)
        let mut edge_weights: HashMap<u64, f64> = HashMap::new();
        // Whether we're doing weighted shortest path
        let is_weighted = self.weight_property.is_some();
        // Resolve weight column index for each rel table
        let mut weight_col_idx: HashMap<u64, Option<usize>> = HashMap::new();

        for &rel_table_id in &self.rel_table_ids {
            if let Some(rel_table) = catalog.get_rel_table(rel_table_id) {
                // Resolve weight column index
                if let Some(ref wp) = self.weight_property {
                    let idx = rel_table.columns.iter().position(|c| c.name == *wp);
                    weight_col_idx.insert(rel_table_id, idx);
                }

                for (&src, neighbors) in rel_table.fwd_adj.iter() {
                    fwd_adj
                        .entry(src)
                        .or_default()
                        .extend(neighbors.iter().map(|(dst, edge_idx)| (*dst, *edge_idx as u64)));
                    // Pre-compute edge weights
                    if is_weighted && let Some(col_idx) = weight_col_idx.get(&rel_table_id).and_then(|&c| c) {
                        for &(_dst, edge_idx) in neighbors {
                            if let Some(weight_val) =
                                rel_table.properties.get(col_idx).and_then(|col| col.get(edge_idx))
                            {
                                let w = match weight_val {
                                    Value::Int64(i) => *i as f64,
                                    Value::Double(d) => *d,
                                    Value::Float(f) => *f as f64,
                                    Value::Int32(i) => *i as f64,
                                    _ => 1.0, // default weight for unrecognized types
                                };
                                edge_weights.insert(edge_idx as u64, w);
                            }
                        }
                    }
                }
                for (&dst, neighbors) in rel_table.rev_adj.iter() {
                    rev_adj
                        .entry(dst)
                        .or_default()
                        .extend(neighbors.iter().map(|(src, edge_idx)| (*src, *edge_idx as u64)));
                }
            }
        }

        // Collect source node offsets from input
        let source_offsets: Vec<i64> = if input.is_empty() || input[0].fields.is_empty() {
            let mut all: Vec<i64> = fwd_adj
                .keys()
                .chain(rev_adj.keys())
                .copied()
                .map(|k| k as i64)
                .collect();
            all.sort();
            all.dedup();
            all
        } else {
            let field = &input[0].fields[0];
            let num_rows = input[0].size;
            let mut offsets = Vec::with_capacity(num_rows);
            for i in 0..num_rows {
                if !field.is_null(i) {
                    let offset = i64::from_le_bytes(field.data()[i * 8..i * 8 + 8].try_into().unwrap());
                    offsets.push(offset);
                }
            }
            offsets
        };

        if source_offsets.is_empty() {
            return Ok(vec![DataChunk::new(vec![])]);
        }

        // Result columns
        let mut result_src: Vec<i64> = Vec::new();
        let mut result_dst: Vec<i64> = Vec::new();
        let mut result_len: Vec<i64> = Vec::new();
        let mut result_cost: Vec<f64> = Vec::new(); // only used for weighted
        // Path tracking: for each result, store the sequence of (node_id, edge_id) pairs
        let mut result_path_nodes: Vec<Vec<i64>> = Vec::new();
        let mut result_path_edges: Vec<Vec<i64>> = Vec::new();

        for &src in &source_offsets {
            let src_u = src as u64;

            if is_weighted {
                // === Weighted Shortest Path: Dijkstra ===
                use std::cmp::Reverse;
                use std::collections::BinaryHeap;

                // Use i64 for priority queue (cost * PRECISION) since f64 doesn't implement Ord.
                // PRECISION = 1000 captures 3 decimal places.
                const COST_PRECISION: i64 = 1000;

                // Helper to convert f64 cost to i64 for the pq
                let cost_to_i64 = |c: f64| -> i64 { (c * COST_PRECISION as f64).round() as i64 };

                // Parent map: child -> (parent, edge_id, depth, cumulative_cost)
                let mut parents: HashMap<u64, (u64, u64, u64, f64)> = HashMap::new();
                let mut pq: BinaryHeap<Reverse<(i64, u64)>> = BinaryHeap::new();

                pq.push(Reverse((cost_to_i64(0.0), src_u)));
                parents.insert(src_u, (u64::MAX, u64::MAX, 0, 0.0));

                while let Some(Reverse((cur_cost_i64, node))) = pq.pop() {
                    let cur_cost = cur_cost_i64 as f64 / COST_PRECISION as f64;
                    let cur_depth = parents.get(&node).map(|&(_, _, d, _)| d).unwrap_or(0);

                    // If we already found a better path to this node, skip
                    if let Some(&(_, _, _, best_cost)) = parents.get(&node)
                        && cur_cost > best_cost + 1e-9
                    {
                        continue;
                    }

                    if cur_depth >= self.upper_bound {
                        continue;
                    }

                    // Get neighbors
                    let neighbors: Vec<(u64, u64)> = match self.direction {
                        ExtendDirection::Fwd => fwd_adj.get(&node).cloned().unwrap_or_default(),
                        ExtendDirection::Bwd => rev_adj.get(&node).cloned().unwrap_or_default(),
                        ExtendDirection::Both => {
                            let mut nbrs = fwd_adj.get(&node).cloned().unwrap_or_default();
                            if let Some(bwd) = rev_adj.get(&node) {
                                nbrs.extend(bwd.iter().copied());
                            }
                            nbrs
                        }
                    };

                    for (nbr, edge_id) in neighbors {
                        let edge_w = edge_weights.get(&edge_id).copied().unwrap_or(1.0);
                        let new_cost = cur_cost + edge_w;
                        let new_depth = cur_depth + 1;

                        let should_visit = match parents.get(&nbr) {
                            Some(&(_, _, _, existing_cost)) => new_cost < existing_cost - 1e-9,
                            None => true,
                        };

                        if should_visit {
                            parents.insert(nbr, (node, edge_id, new_depth, new_cost));
                            pq.push(Reverse((cost_to_i64(new_cost), nbr)));
                        }
                    }
                }

                // Emit results
                for (&node, &(_parent, _eid, depth, cost)) in &parents {
                    if depth < self.lower_bound || depth > self.upper_bound {
                        continue;
                    }
                    if depth == 0 && self.lower_bound > 0 {
                        continue;
                    }

                    result_src.push(src);
                    result_dst.push(node as i64);
                    result_len.push(depth as i64);
                    result_cost.push(cost);

                    // Reconstruct path
                    let mut cur = node;
                    let mut temp_nodes = vec![node as i64];
                    let mut temp_edges = Vec::new();

                    while cur != src_u {
                        if let Some(&(parent, eid, _, _)) = parents.get(&cur) {
                            if parent == u64::MAX {
                                break;
                            }
                            temp_edges.push(eid as i64);
                            temp_nodes.push(parent as i64);
                            cur = parent;
                        } else {
                            break;
                        }
                    }

                    temp_nodes.reverse();
                    temp_edges.reverse();
                    let mut path_nodes = vec![src];
                    path_nodes.extend(temp_nodes);
                    result_path_nodes.push(path_nodes);
                    result_path_edges.push(temp_edges);
                }
            } else {
                // === Unweighted: BFS ===
                let mut queue = VecDeque::new();
                // Parent map: child -> (parent, edge_id, depth)
                let mut parents: HashMap<u64, (u64, u64, u64)> = HashMap::new();
                queue.push_back((src_u, 0u64));
                parents.insert(src_u, (u64::MAX, u64::MAX, 0));

                let semantic = self.semantic;

                while let Some((node, depth)) = queue.pop_front() {
                    if depth >= self.upper_bound {
                        continue;
                    }

                    let neighbors: Vec<(u64, u64)> = match self.direction {
                        ExtendDirection::Fwd => fwd_adj.get(&node).cloned().unwrap_or_default(),
                        ExtendDirection::Bwd => rev_adj.get(&node).cloned().unwrap_or_default(),
                        ExtendDirection::Both => {
                            let mut nbrs = fwd_adj.get(&node).cloned().unwrap_or_default();
                            if let Some(bwd) = rev_adj.get(&node) {
                                nbrs.extend(bwd.iter().copied());
                            }
                            nbrs
                        }
                    };

                    'neighbors: for (nbr, edge_id) in neighbors {
                        if parents.contains_key(&nbr) {
                            match semantic {
                                PathSemantic::Walk | PathSemantic::Acyclic => continue 'neighbors,
                                PathSemantic::Trail => {
                                    let mut cur = node;
                                    while let Some(&(p, eid, _)) = parents.get(&cur) {
                                        if eid == edge_id {
                                            continue 'neighbors;
                                        }
                                        if p == u64::MAX {
                                            break;
                                        }
                                        cur = p;
                                    }
                                }
                            }
                        }

                        let new_depth = depth + 1;
                        parents.insert(nbr, (node, edge_id, new_depth));
                        queue.push_back((nbr, new_depth));
                    }
                }

                // Emit results for nodes at valid depths
                for (&node, &(_parent_node, _edge_id, depth)) in &parents {
                    if depth < self.lower_bound || depth > self.upper_bound {
                        continue;
                    }
                    if depth == 0 && self.lower_bound > 0 {
                        continue;
                    }

                    result_src.push(src);
                    result_dst.push(node as i64);
                    result_len.push(depth as i64);

                    // Reconstruct path
                    let mut cur = node;
                    let mut temp_nodes = vec![node as i64];
                    let mut temp_edges = Vec::new();

                    while cur != src_u {
                        if let Some(&(parent, eid, _)) = parents.get(&cur) {
                            if parent == u64::MAX {
                                break;
                            }
                            temp_edges.push(eid as i64);
                            temp_nodes.push(parent as i64);
                            cur = parent;
                        } else {
                            break;
                        }
                    }

                    temp_nodes.reverse();
                    temp_edges.reverse();
                    let mut path_nodes = vec![src];
                    path_nodes.extend(temp_nodes);
                    result_path_nodes.push(path_nodes);
                    result_path_edges.push(temp_edges);
                }
            }
        }

        // Build output DataChunk
        let num_results = result_src.len();
        if num_results == 0 {
            return Ok(vec![DataChunk::new(vec![])]);
        }

        // Column 0-2: primitive Int64 vectors
        let mut src_v = ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, num_results);
        let mut dst_v = ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, num_results);
        let mut len_v = ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, num_results);

        for i in 0..num_results {
            let offset = i * 8;
            src_v.data_mut()[offset..offset + 8].copy_from_slice(&result_src[i].to_le_bytes());
            src_v.set_null(i, false);
            dst_v.data_mut()[offset..offset + 8].copy_from_slice(&result_dst[i].to_le_bytes());
            dst_v.set_null(i, false);
            len_v.data_mut()[offset..offset + 8].copy_from_slice(&result_len[i].to_le_bytes());
            len_v.set_null(i, false);
        }
        src_v.resize(num_results);
        dst_v.resize(num_results);
        len_v.resize(num_results);

        // Column 3-4: create Value vectors for path lists, then convert to columns
        // Use Vec<Option<Value>> to store per-row path data
        let mut path_nodes_col: Vec<Value> = Vec::with_capacity(num_results);
        let mut path_edges_col: Vec<Value> = Vec::with_capacity(num_results);

        for i in 0..num_results {
            // Path nodes as List(Int64)
            let node_vals: Vec<Value> = result_path_nodes[i].iter().map(|&n| Value::Int64(n)).collect();
            path_nodes_col.push(Value::List(node_vals));
            // Path edges as List(Int64)
            let edge_vals: Vec<Value> = result_path_edges[i].iter().map(|&e| Value::Int64(e)).collect();
            path_edges_col.push(Value::List(edge_vals));
        }

        // Store List values in ValueVector via set_value
        let mut path_nodes_v = ValueVector::new(kuzu_common::types::PhysicalTypeID::List, num_results);
        let mut path_edges_v = ValueVector::new(kuzu_common::types::PhysicalTypeID::List, num_results);

        for (i, val) in path_nodes_col.iter().enumerate() {
            path_nodes_v.set_value(i, val).ok();
        }
        for (i, val) in path_edges_col.iter().enumerate() {
            path_edges_v.set_value(i, val).ok();
        }

        // When weighted, include cost column
        let has_cost = is_weighted;

        if has_cost {
            let mut cost_v = ValueVector::new(kuzu_common::types::PhysicalTypeID::Double, num_results);
            for (i, cost) in result_cost.iter().enumerate().take(num_results) {
                let offset = i * 8;
                cost_v.data_mut()[offset..offset + 8].copy_from_slice(&cost.to_le_bytes());
                cost_v.set_null(i, false);
            }
            cost_v.resize(num_results);

            Ok(vec![DataChunk {
                fields: vec![src_v, dst_v, len_v, path_nodes_v, path_edges_v, cost_v],
                size: num_results,
                field_names: vec![],
                sel_vector: None,
            }])
        } else {
            Ok(vec![DataChunk {
                fields: vec![src_v, dst_v, len_v, path_nodes_v, path_edges_v],
                size: num_results,
                field_names: vec![],
                sel_vector: None,
            }])
        }
    }
}

pub struct PhysicalCreateNode {
    pub table_name: String,
    pub table_id: u64,
    pub out_var_name: String,
    pub properties: Vec<(String, kuzu_parser::ast::Expression)>,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalCreateNode {
    pub fn execute(&self, input: Vec<DataChunk>) -> Result<Vec<DataChunk>, String> {
        if input.is_empty() {
            return Ok(input);
        }

        let mut table = self
            .table_catalog
            .get_node_table_by_name_mut(&self.table_name)
            .ok_or_else(|| format!("Node table {} not found", self.table_name))?;

        // For each input chunk, we create nodes and attach the new node IDs
        let mut output = Vec::with_capacity(input.len());

        for mut chunk in input {
            let mut node_ids = ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, chunk.size);

            for i in 0..chunk.size {
                let mut values = vec![kuzu_common::types::Value::Null; table.columns.len()];
                for (prop_name, prop_expr) in &self.properties {
                    if let Some(col_idx) = table.columns.iter().position(|c| c.name == *prop_name) {
                        values[col_idx] = evaluate_expression_for_row(prop_expr, &chunk, i);
                    }
                }

                let row_offset = table.insert_row(values)?;
                node_ids.data_mut()[i * 8..(i + 1) * 8].copy_from_slice(&(row_offset as i64).to_le_bytes());
                node_ids.set_null(i, false);
            }
            node_ids.resize(chunk.size);

            chunk.fields.push(node_ids);
            chunk.field_names.push(self.out_var_name.clone());
            output.push(chunk);
        }

        Ok(output)
    }
}

pub struct PhysicalCreateRel {
    pub table_name: String,
    pub table_id: u64,
    pub src_node_name: String,
    pub dst_node_name: String,
    pub properties: Vec<(String, kuzu_parser::ast::Expression)>,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalCreateRel {
    pub fn execute(&self, input: Vec<DataChunk>) -> Result<Vec<DataChunk>, String> {
        if input.is_empty() {
            return Ok(input);
        }

        let mut table = self
            .table_catalog
            .get_rel_table_by_name_mut(&self.table_name)
            .ok_or_else(|| format!("Rel table {} not found", self.table_name))?;

        let mut output = Vec::with_capacity(input.len());

        for chunk in input {
            let src_name_id = format!("{}.{}", self.src_node_name, "_id");
            let src_name_pk = format!("{}.{}", self.src_node_name, "id");
            let src_idx = chunk
                .field_names
                .iter()
                .position(|name| name == &src_name_id)
                .or_else(|| chunk.field_names.iter().position(|name| name == &self.src_node_name))
                .or_else(|| chunk.field_names.iter().position(|name| name == &src_name_pk))
                .ok_or_else(|| format!("Source node variable {} not found", self.src_node_name))?;

            let dst_name_id = format!("{}.{}", self.dst_node_name, "_id");
            let dst_name_pk = format!("{}.{}", self.dst_node_name, "id");
            let dst_idx = chunk
                .field_names
                .iter()
                .position(|name| name == &dst_name_id)
                .or_else(|| chunk.field_names.iter().position(|name| name == &self.dst_node_name))
                .or_else(|| chunk.field_names.iter().position(|name| name == &dst_name_pk))
                .ok_or_else(|| format!("Destination node variable {} not found", self.dst_node_name))?;

            let src_vec = &chunk.fields[src_idx];
            let dst_vec = &chunk.fields[dst_idx];

            let mut inserted = 0;
            for i in 0..chunk.size {
                if src_vec.is_null(i) || dst_vec.is_null(i) {
                    continue; // Skip creating relationships involving NULL nodes
                }

                let mut src_bytes = [0u8; 8];
                src_bytes.copy_from_slice(&src_vec.data()[i * 8..(i + 1) * 8]);
                let src_id = i64::from_le_bytes(src_bytes) as u64;

                let mut dst_bytes = [0u8; 8];
                dst_bytes.copy_from_slice(&dst_vec.data()[i * 8..(i + 1) * 8]);
                let dst_id = i64::from_le_bytes(dst_bytes) as u64;

                let mut values = vec![kuzu_common::types::Value::Null; table.columns.len()];
                for (prop_name, prop_expr) in &self.properties {
                    if let Some(col_idx) = table.columns.iter().position(|c| c.name == *prop_name) {
                        values[col_idx] = evaluate_expression_for_row(prop_expr, &chunk, i);
                    }
                }

                table.insert_rel(src_id, dst_id, values)?;
                inserted += 1;
            }
            println!(
                "PhysicalCreateRel inserted {} relationships from chunk of size {}",
                inserted, chunk.size
            );

            output.push(chunk);
        }

        Ok(output)
    }
}

/// Physical operator for extending from a source node through a relationship.
///
/// Takes input chunks from the source node scan, and for each source row,
/// looks up adjacency list entries in the relationship table, producing
/// output rows that include the source fields, relationship properties,
/// and destination node properties.
///
/// Ported from C++ `ScanRelTable` (the physical extend operator).
pub struct PhysicalExtend {
    /// Name of the relationship table.
    pub rel_table_name: String,
    /// ID of the relationship table.
    pub rel_table_id: u64,
    /// Variable name of the bound (source) node.
    pub bound_node_var: String,
    /// Direction of the extend.
    pub direction: kuzu_parser::ast::EdgeDirection,
    /// Variable name of the destination node.
    pub dst_node_var: String,
    /// Table name of the destination node.
    pub dst_table_name: String,
    /// Table ID of the destination node.
    pub dst_table_id: u64,
    /// Table catalog for data access.
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalExtend {
    pub fn execute(&self, input: Vec<DataChunk>) -> Result<Vec<DataChunk>, String> {
        if input.is_empty() || input.iter().all(|c| c.size == 0) {
            return Ok(input);
        }

        // Collect rel table data upfront (owned)
        let (fwd_adj, rev_adj, rel_props, rel_cols) = {
            let rel_table = self
                .table_catalog
                .get_rel_table_by_name(&self.rel_table_name)
                .ok_or_else(|| format!("Rel table {} not found", self.rel_table_name))?;
            let fwd = rel_table.fwd_adj.clone();
            let rev = rel_table.rev_adj.clone();
            let props = rel_table.properties.clone();
            let cols = rel_table.columns.clone();
            (fwd, rev, props, cols)
        };

        // Collect dest node table data upfront (owned)
        let (dest_data, dest_cols, dest_pk_col) = {
            let dest_table = self
                .table_catalog
                .get_node_table_by_name(&self.dst_table_name)
                .ok_or_else(|| format!("Node table {} not found", self.dst_table_name))?;
            let data = dest_table.to_column_major_data();
            let cols = dest_table.columns.clone();
            let pk = dest_table.primary_key_column;
            (data, cols, pk)
        };

        // Build PK → row offset map for destination lookups
        let pk_to_row: std::collections::HashMap<u64, usize> = if dest_pk_col < dest_data.len() {
            dest_data[dest_pk_col]
                .iter()
                .enumerate()
                .filter_map(|(row, val)| {
                    if let Value::Int64(id) = val {
                        Some((*id as u64, row))
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            std::collections::HashMap::new()
        };

        let mut output = Vec::with_capacity(input.len());

        for chunk in input {
            // Find the bound node column in the chunk
            let bound_name_id = format!("{}.{}", self.bound_node_var, "_id");
            let bound_name_pk = format!("{}.{}", self.bound_node_var, "id");
            let bound_idx = chunk
                .field_names
                .iter()
                .position(|name| name == &bound_name_id)
                .or_else(|| chunk.field_names.iter().position(|name| name == &self.bound_node_var))
                .or_else(|| chunk.field_names.iter().position(|name| name == &bound_name_pk))
                .ok_or_else(|| {
                    format!(
                        "Bound node variable {} not found in Extend input. Available fields: {:?}",
                        self.bound_node_var, chunk.field_names
                    )
                })?;

            // Calculate total output rows and build row mapping
            let mut total_rows = 0;
            let mut row_mappings: Vec<(usize, u64, usize)> = Vec::new(); // (input_row, dst_offset, edge_idx)

            for i in 0..chunk.size {
                if chunk.fields[bound_idx].is_null(i) {
                    continue;
                }
                let offset = i * 8;
                let src_vec_data = chunk.fields[bound_idx].data();
                if offset + 8 > src_vec_data.len() {
                    continue;
                }
                let mut src_bytes = [0u8; 8];
                src_bytes.copy_from_slice(&src_vec_data[offset..offset + 8]);
                let src_id = i64::from_le_bytes(src_bytes) as u64;

                let edges: Vec<(u64, usize)> = match self.direction {
                    kuzu_parser::ast::EdgeDirection::LeftToRight => fwd_adj.get(&src_id).cloned().unwrap_or_default(),
                    kuzu_parser::ast::EdgeDirection::RightToLeft => rev_adj.get(&src_id).cloned().unwrap_or_default(),
                    kuzu_parser::ast::EdgeDirection::Both => {
                        let mut all = fwd_adj.get(&src_id).cloned().unwrap_or_default();
                        if let Some(rev) = rev_adj.get(&src_id) {
                            all.extend(rev.iter().cloned());
                        }
                        all
                    }
                };

                for &(dst_offset, edge_idx) in &edges {
                    if !pk_to_row.contains_key(&dst_offset)
                        && dst_offset as usize >= dest_data.first().map(|c| c.len()).unwrap_or(0)
                    {
                        continue;
                    }
                    total_rows += 1;
                    row_mappings.push((i, dst_offset, edge_idx));
                }
            }

            if total_rows == 0 {
                output.push(DataChunk::new(vec![]));
                continue;
            }

            // Build output:
            // Column layout: [input_fields | rel_properties | dest_node_fields]
            let num_input_fields = chunk.fields.len();
            let num_rel_cols = rel_cols.len();
            let num_dest_cols = dest_cols.len();
            let num_out_cols = num_input_fields + num_rel_cols + num_dest_cols;

            // Build column-major data
            let mut out_data: Vec<Vec<Value>> = vec![Vec::with_capacity(total_rows); num_out_cols];

            for &(input_row, dst_offset, edge_idx) in &row_mappings {
                // Copy input fields
                for col in 0..num_input_fields {
                    let val = chunk.fields[col].get_value(input_row).unwrap_or(Value::Null);
                    out_data[col].push(val);
                }
                // Copy rel properties
                for col in 0..num_rel_cols {
                    let val = rel_props
                        .get(col)
                        .and_then(|c| c.get(edge_idx))
                        .cloned()
                        .unwrap_or(Value::Null);
                    out_data[num_input_fields + col].push(val);
                }
                // Copy dest node properties
                let dest_row = pk_to_row.get(&dst_offset).copied();
                for col in 0..num_dest_cols {
                    let val = dest_row
                        .and_then(|r| dest_data.get(col).and_then(|c| c.get(r)))
                        .cloned()
                        .unwrap_or_else(|| {
                            dest_data
                                .get(col)
                                .and_then(|c| c.get(dst_offset as usize))
                                .cloned()
                                .unwrap_or(Value::Null)
                        });
                    out_data[num_input_fields + num_rel_cols + col].push(val);
                }
            }

            // Convert column-major data to ValueVectors
            let mut fields = Vec::with_capacity(num_out_cols);
            let mut field_names = Vec::with_capacity(num_out_cols);

            // Input field names (already prefixed)
            for col in 0..num_input_fields {
                let phys_type = chunk.fields[col].physical_type();
                let mut v = ValueVector::new(phys_type, total_rows);
                v.resize(total_rows);
                for row in 0..total_rows {
                    store_value_in_vector(&mut v, row, &out_data[col][row]);
                }
                fields.push(v);
                if col < chunk.field_names.len() {
                    field_names.push(chunk.field_names[col].clone());
                } else {
                    field_names.push(format!("field_{}", col));
                }
            }

            // Rel field names (prefixed with rel table name)
            for col in 0..num_rel_cols {
                let phys_type = if col < rel_cols.len() {
                    PhysicalScan::logical_to_physical(&rel_cols[col].logical_type)
                } else {
                    PhysicalTypeID::Int64
                };
                let mut v = ValueVector::new(phys_type, total_rows);
                v.resize(total_rows);
                for row in 0..total_rows {
                    store_value_in_vector(&mut v, row, &out_data[num_input_fields + col][row]);
                }
                fields.push(v);
                let rel_prefix = &self.rel_table_name;
                let col_name = rel_cols.get(col).map(|c| c.name.as_str()).unwrap_or("");
                field_names.push(format!("{}.{}", rel_prefix, col_name));
            }

            // Dest field names (prefixed with dest variable)
            for col in 0..num_dest_cols {
                let phys_type = if col < dest_cols.len() {
                    PhysicalScan::logical_to_physical(&dest_cols[col].logical_type)
                } else {
                    PhysicalTypeID::Int64
                };
                let mut v = ValueVector::new(phys_type, total_rows);
                v.resize(total_rows);
                for row in 0..total_rows {
                    store_value_in_vector(&mut v, row, &out_data[num_input_fields + num_rel_cols + col][row]);
                }
                fields.push(v);
                let prefix = &self.dst_node_var;
                let col_name = dest_cols.get(col).map(|c| c.name.as_str()).unwrap_or("");
                field_names.push(format!("{}.{}", prefix, col_name));
            }

            output.push(DataChunk {
                fields,
                size: total_rows,
                field_names,
                sel_vector: None,
            });
        }

        Ok(output)
    }
}


