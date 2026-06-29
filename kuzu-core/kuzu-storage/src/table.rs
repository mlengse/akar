//! Table storage — columnar node/rel tables with NodeGroup-based storage.

use crate::node_group::NodeGroup;
use kuzu_common::types::{LogicalTypeID, Value};
use std::collections::HashMap;

/// A column definition within a table.
#[derive(Debug, Clone)]
pub struct ColumnDefinition {
    pub name: String,
    pub logical_type: LogicalTypeID,
    pub is_primary_key: bool,
}

/// A node table stores properties for a node label using NodeGroup-based
/// columnar storage. Data is held in-memory as `NodeGroup`s; when a group
/// reaches `NODE_GROUP_SIZE` rows a new group is automatically created.
#[derive(Debug, Clone)]
pub struct NodeTable {
    pub table_id: u64,
    pub name: String,
    pub columns: Vec<ColumnDefinition>,
    pub primary_key_column: usize,
    pub num_rows: u64,
    /// NodeGroup-based columnar storage. Each group holds up to
    /// `NODE_GROUP_SIZE` rows across all columns.
    pub node_groups: Vec<NodeGroup>,
}

impl NodeTable {
    pub fn new(table_id: u64, name: String, columns: Vec<ColumnDefinition>) -> Self {
        let primary_key_column = columns
            .iter()
            .position(|c| c.is_primary_key)
            .unwrap_or(0);
        Self {
            table_id,
            name,
            columns,
            primary_key_column,
            num_rows: 0,
            node_groups: Vec::new(),
        }
    }

    /// Insert a row of values into the table.
    ///
    /// Appends to the current `NodeGroup`; auto-creates a new group when the
    /// current one is full (reaches `NODE_GROUP_SIZE` rows).
    ///
    /// Returns an error if the number of values doesn't match the number of columns.
    pub fn insert_row(&mut self, values: Vec<Value>) -> Result<(), String> {
        if values.len() != self.columns.len() {
            return Err(format!(
                "Column count mismatch: expected {} values, got {}",
                self.columns.len(),
                values.len()
            ));
        }

        // Get or create the current node group.
        let num_cols = self.columns.len();
        if self.node_groups.is_empty() || self.node_groups.last().unwrap().is_full() {
            let start_offset = self.num_rows;
            self.node_groups
                .push(NodeGroup::new(num_cols, start_offset));
        }

        let current = self.node_groups.last_mut().unwrap();
        current.append_row(values)?;
        self.num_rows += 1;
        Ok(())
    }

    /// Scan all values for a given column across all node groups.
    ///
    /// Returns a flat `Vec<Value>` containing values from `start` to
    /// `start + count` (or fewer if the end of the table is reached).
    pub fn scan_column(&self, col_idx: usize, start: u64, count: u64) -> Vec<Value> {
        if col_idx >= self.columns.len() || start >= self.num_rows {
            return Vec::new();
        }
        let end = (start + count).min(self.num_rows);
        let mut result = Vec::with_capacity((end - start) as usize);

        // Find the first node group containing `start`.
        let group_start = self.find_group(start);
        let mut remaining = end - start;

        for g_idx in group_start..self.node_groups.len() {
            if remaining == 0 {
                break;
            }
            let group = &self.node_groups[g_idx];
            let local_start = if g_idx == group_start {
                (start - group.start_offset) as usize
            } else {
                0
            };
            let available = (group.num_nodes as usize).saturating_sub(local_start);
            let take = available.min(remaining as usize);

            for row in local_start..local_start + take {
                match group.get_value(row, col_idx) {
                    Some(v) => result.push(v.clone()),
                    None => result.push(Value::Null),
                }
            }
            remaining -= take as u64;
        }

        result
    }

    /// Get a single value at (row, col) by locating the correct `NodeGroup`
    /// and `ColumnChunk`.
    pub fn get_value(&self, row: usize, col: usize) -> Option<&Value> {
        if col >= self.columns.len() || row as u64 >= self.num_rows {
            return None;
        }
        let group_idx = self.find_group(row as u64);
        let group = self.node_groups.get(group_idx)?;
        let local_row = row as u64 - group.start_offset;
        group.get_value(local_row as usize, col)
    }

    /// Reconstruct column-major data (`Vec<Vec<Value>>`) from all node groups.
    ///
    /// Used by the processor (`resolve_scan_data`) for backward compatibility.
    pub fn to_column_major_data(&self) -> Vec<Vec<Value>> {
        let num_cols = self.columns.len();
        let mut result = vec![Vec::with_capacity(self.num_rows as usize); num_cols];

        for group in &self.node_groups {
            for row in 0..group.num_nodes as usize {
                for col in 0..num_cols {
                    match group.get_value(row, col) {
                        Some(v) => result[col].push(v.clone()),
                        None => result[col].push(Value::Null),
                    }
                }
            }
        }

        result
    }

    /// Binary-search for the node group that contains `row`.
    fn find_group(&self, row: u64) -> usize {
        match self
            .node_groups
            .binary_search_by_key(&row, |g| g.start_offset)
        {
            Ok(i) => i,
            Err(i) => {
                if i == 0 {
                    0
                } else {
                    i - 1
                }
            }
        }
    }
}

/// A relationship (edge) table with CSR (Compressed Sparse Row) adjacency storage.
///
/// Each edge connects a source node to a destination node and may carry
/// a set of property values (one per column in `columns`).
///
/// # Storage layout
///
/// - `edges` — flat edge list: `edge_idx → (src_offset, dst_offset)`
/// - `fwd_adj` — forward index: `src_offset → Vec<(dst_offset, edge_idx)>`
/// - `rev_adj` — reverse index: `dst_offset → Vec<(src_offset, edge_idx)>`
/// - `properties` — column-major property storage: `properties[col_idx][edge_idx]`
#[derive(Debug, Clone)]
pub struct RelTable {
    pub table_id: u64,
    pub name: String,
    pub src_table_id: u64,
    pub dst_table_id: u64,
    pub columns: Vec<ColumnDefinition>,
    pub num_rows: u64,
    /// Flat edge list: edge_idx → (src_offset, dst_offset).
    pub edges: Vec<(u64, u64)>,
    /// Forward CSR adjacency: src_offset → [(dst_offset, edge_idx), ...].
    pub fwd_adj: HashMap<u64, Vec<(u64, usize)>>,
    /// Reverse CSR adjacency: dst_offset → [(src_offset, edge_idx), ...].
    pub rev_adj: HashMap<u64, Vec<(u64, usize)>>,
    /// Column-major property storage: properties[col_idx][edge_idx].
    pub properties: Vec<Vec<Value>>,
}

impl RelTable {
    pub fn new(
        table_id: u64,
        name: String,
        src_table_id: u64,
        dst_table_id: u64,
        columns: Vec<ColumnDefinition>,
    ) -> Self {
        let num_cols = columns.len();
        Self {
            table_id,
            name,
            src_table_id,
            dst_table_id,
            columns,
            num_rows: 0,
            edges: Vec::new(),
            fwd_adj: HashMap::new(),
            rev_adj: HashMap::new(),
            properties: vec![Vec::new(); num_cols],
        }
    }

    /// Insert a relationship (edge) between two nodes with property values.
    ///
    /// `from` and `to` are the node offsets of the source and destination
    /// nodes within their respective tables.
    ///
    /// Returns an error if the number of values doesn't match the number
    /// of property columns.
    pub fn insert_rel(
        &mut self,
        from: u64,
        to: u64,
        values: Vec<Value>,
    ) -> Result<(), String> {
        if values.len() != self.columns.len() {
            return Err(format!(
                "Column count mismatch: expected {} values, got {}",
                self.columns.len(),
                values.len()
            ));
        }

        let edge_idx = self.edges.len();
        self.edges.push((from, to));

        // Update forward adjacency.
        self.fwd_adj
            .entry(from)
            .or_default()
            .push((to, edge_idx));

        // Update reverse adjacency.
        self.rev_adj
            .entry(to)
            .or_default()
            .push((from, edge_idx));

        // Store property values.
        for (col_idx, val) in values.into_iter().enumerate() {
            self.properties[col_idx].push(val);
        }

        self.num_rows += 1;
        Ok(())
    }

    /// Insert a row of values (legacy alias that treats all columns as properties).
    /// Only the first two values are treated as (from, to) if the table has
    /// at least 2 columns; otherwise they are stored as pure properties.
    pub fn insert_row(&mut self, values: Vec<Value>) -> Result<(), String> {
        // If there are at least 2 "structural" columns (src_id, dst_id) plus
        // property columns, we assume the first two values are the node offsets.
        // This preserves backward compatibility with the old flat API.
        let num_prop_cols = self.columns.len();
        if values.len() != num_prop_cols {
            return Err(format!(
                "Column count mismatch: expected {} values, got {}",
                num_prop_cols,
                values.len()
            ));
        }

        // We treat the values as plain properties and use sequential edge IDs
        // as (from, to) placeholders. Real callers should use `insert_rel`.
        let from = self.num_rows;
        let to = self.num_rows;
        self.insert_rel(from, to, values)
    }

    /// Scan the forward adjacency list for a given source node.
    ///
    /// Returns a list of `(dst_offset, edge_idx)` pairs, or an empty vec
    /// if the node has no outgoing edges.
    pub fn scan_adj_list(&self, src_offset: u64) -> &[(u64, usize)] {
        self.fwd_adj
            .get(&src_offset)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Scan the reverse adjacency list for a given destination node.
    ///
    /// Returns a list of `(src_offset, edge_idx)` pairs, or an empty vec
    /// if the node has no incoming edges.
    pub fn scan_rev_adj_list(&self, dst_offset: u64) -> &[(u64, usize)] {
        self.rev_adj
            .get(&dst_offset)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Get all outgoing edges from a source node as `(dst_offset, property_values)`.
    pub fn get_outgoing_edges(&self, src_offset: u64) -> Vec<(u64, Vec<Value>)> {
        self.scan_adj_list(src_offset)
            .iter()
            .map(|&(dst, edge_idx)| {
                let props = self.get_edge_properties(edge_idx);
                (dst, props)
            })
            .collect()
    }

    /// Get all incoming edges to a destination node as `(src_offset, property_values)`.
    pub fn get_incoming_edges(&self, dst_offset: u64) -> Vec<(u64, Vec<Value>)> {
        self.scan_rev_adj_list(dst_offset)
            .iter()
            .map(|&(src, edge_idx)| {
                let props = self.get_edge_properties(edge_idx);
                (src, props)
            })
            .collect()
    }

    /// Get the property values for a specific edge by index.
    pub fn get_edge_properties(&self, edge_idx: usize) -> Vec<Value> {
        let mut props = Vec::with_capacity(self.columns.len());
        for col in &self.properties {
            match col.get(edge_idx) {
                Some(v) => props.push(v.clone()),
                None => props.push(Value::Null),
            }
        }
        props
    }

    /// Get all values for a given property column (by index) as a slice.
    pub fn get_column(&self, col_idx: usize) -> Option<&[Value]> {
        self.properties.get(col_idx).map(|v| v.as_slice())
    }

    /// Reconstruct column-major data from properties for backward compatibility.
    pub fn to_column_major_data(&self) -> Vec<Vec<Value>> {
        self.properties.clone()
    }
}

/// A collection of tables managed by the storage engine.
#[derive(Debug, Default)]
pub struct TableCatalog {
    node_tables: HashMap<u64, NodeTable>,
    rel_tables: HashMap<u64, RelTable>,
    /// Map from table name to table ID for node tables
    node_name_to_id: HashMap<String, u64>,
    /// Map from table name to table ID for rel tables
    rel_name_to_id: HashMap<String, u64>,
    next_table_id: u64,
}

impl TableCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_node_table(&mut self, name: String, columns: Vec<ColumnDefinition>) -> NodeTable {
        let table_id = self.next_table_id;
        self.next_table_id += 1;
        let table = NodeTable::new(table_id, name.clone(), columns);
        self.node_name_to_id.insert(name, table_id);
        self.node_tables.insert(table_id, table.clone());
        table
    }

    pub fn create_rel_table(
        &mut self,
        name: String,
        src_table_id: u64,
        dst_table_id: u64,
        columns: Vec<ColumnDefinition>,
    ) -> RelTable {
        let table_id = self.next_table_id;
        self.next_table_id += 1;
        let table = RelTable::new(table_id, name.clone(), src_table_id, dst_table_id, columns);
        self.rel_name_to_id.insert(name, table_id);
        self.rel_tables.insert(table_id, table.clone());
        table
    }

    pub fn get_node_table(&self, table_id: u64) -> Option<&NodeTable> {
        self.node_tables.get(&table_id)
    }

    pub fn get_node_table_mut(&mut self, table_id: u64) -> Option<&mut NodeTable> {
        self.node_tables.get_mut(&table_id)
    }

    pub fn get_node_table_by_name(&self, name: &str) -> Option<&NodeTable> {
        self.node_name_to_id
            .get(name)
            .and_then(|id| self.node_tables.get(id))
    }

    pub fn get_node_table_by_name_mut(&mut self, name: &str) -> Option<&mut NodeTable> {
        let id = self.node_name_to_id.get(name).copied()?;
        self.node_tables.get_mut(&id)
    }

    pub fn get_rel_table(&self, table_id: u64) -> Option<&RelTable> {
        self.rel_tables.get(&table_id)
    }

    pub fn get_rel_table_mut(&mut self, table_id: u64) -> Option<&mut RelTable> {
        self.rel_tables.get_mut(&table_id)
    }

    pub fn get_rel_table_by_name(&self, name: &str) -> Option<&RelTable> {
        self.rel_name_to_id
            .get(name)
            .and_then(|id| self.rel_tables.get(id))
    }

    pub fn all_node_tables(&self) -> impl Iterator<Item = &NodeTable> {
        self.node_tables.values()
    }

    pub fn all_rel_tables(&self) -> impl Iterator<Item = &RelTable> {
        self.rel_tables.values()
    }

    /// Get the number of rows in a node table by name.
    pub fn node_table_num_rows(&self, name: &str) -> u64 {
        self.get_node_table_by_name(name)
            .map(|t| t.num_rows)
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column_chunk::NODE_GROUP_SIZE;

    // ==================== NodeTable tests ====================

    #[test]
    fn test_node_table_empty() {
        let table = NodeTable::new(1, "Person".into(), vec![
            ColumnDefinition { name: "name".into(), logical_type: LogicalTypeID::String, is_primary_key: true },
            ColumnDefinition { name: "age".into(), logical_type: LogicalTypeID::Int64, is_primary_key: false },
        ]);
        assert_eq!(table.num_rows, 0);
        assert!(table.node_groups.is_empty());
    }

    #[test]
    fn test_node_table_insert_and_get() {
        let mut table = NodeTable::new(1, "Person".into(), vec![
            ColumnDefinition { name: "name".into(), logical_type: LogicalTypeID::String, is_primary_key: true },
            ColumnDefinition { name: "age".into(), logical_type: LogicalTypeID::Int64, is_primary_key: false },
        ]);
        table.insert_row(vec![Value::String("Alice".into()), Value::Int64(30)]).unwrap();
        table.insert_row(vec![Value::String("Bob".into()), Value::Int64(25)]).unwrap();

        assert_eq!(table.num_rows, 2);
        assert_eq!(table.get_value(0, 0), Some(&Value::String("Alice".into())));
        assert_eq!(table.get_value(1, 1), Some(&Value::Int64(25)));
    }

    #[test]
    fn test_node_table_scan_column() {
        let mut table = NodeTable::new(1, "T".into(), vec![
            ColumnDefinition { name: "val".into(), logical_type: LogicalTypeID::Int64, is_primary_key: false },
        ]);
        for i in 0..100 {
            table.insert_row(vec![Value::Int64(i)]).unwrap();
        }
        let scanned = table.scan_column(0, 10, 5);
        assert_eq!(scanned.len(), 5);
        assert_eq!(scanned[0], Value::Int64(10));
        assert_eq!(scanned[4], Value::Int64(14));
    }

    #[test]
    fn test_node_table_to_column_major() {
        let mut table = NodeTable::new(1, "T".into(), vec![
            ColumnDefinition { name: "x".into(), logical_type: LogicalTypeID::Int64, is_primary_key: false },
            ColumnDefinition { name: "y".into(), logical_type: LogicalTypeID::Int64, is_primary_key: false },
        ]);
        table.insert_row(vec![Value::Int64(1), Value::Int64(10)]).unwrap();
        table.insert_row(vec![Value::Int64(2), Value::Int64(20)]).unwrap();

        let data = table.to_column_major_data();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0], vec![Value::Int64(1), Value::Int64(2)]);
        assert_eq!(data[1], vec![Value::Int64(10), Value::Int64(20)]);
    }

    #[test]
    fn test_node_table_auto_node_group() {
        let mut table = NodeTable::new(1, "T".into(), vec![
            ColumnDefinition { name: "v".into(), logical_type: LogicalTypeID::Int64, is_primary_key: false },
        ]);
        // Insert NODE_GROUP_SIZE + 1 rows to force a second node group
        for i in 0..NODE_GROUP_SIZE as u64 + 1 {
            table.insert_row(vec![Value::Int64(i as i64)]).unwrap();
        }
        assert_eq!(table.num_rows, NODE_GROUP_SIZE as u64 + 1);
        assert_eq!(table.node_groups.len(), 2);
        assert_eq!(table.node_groups[0].num_nodes, NODE_GROUP_SIZE as u64);
        assert_eq!(table.node_groups[1].num_nodes, 1);
        // Scan should still return all values
        assert_eq!(table.get_value(0, 0), Some(&Value::Int64(0)));
        assert_eq!(table.get_value(NODE_GROUP_SIZE, 0), Some(&Value::Int64(NODE_GROUP_SIZE as i64)));
    }

    // ==================== RelTable (CSR) tests ====================

    fn make_rel_table() -> RelTable {
        RelTable::new(1, "Knows".into(), 0, 1, vec![
            ColumnDefinition { name: "since".into(), logical_type: LogicalTypeID::Int64, is_primary_key: false },
            ColumnDefinition { name: "weight".into(), logical_type: LogicalTypeID::Double, is_primary_key: false },
        ])
    }

    #[test]
    fn test_rel_table_empty() {
        let rel = make_rel_table();
        assert_eq!(rel.num_rows, 0);
        assert!(rel.edges.is_empty());
        assert!(rel.fwd_adj.is_empty());
        assert!(rel.rev_adj.is_empty());
    }

    #[test]
    fn test_rel_insert_basic() {
        let mut rel = make_rel_table();
        rel.insert_rel(0, 1, vec![Value::Int64(2020), Value::Double(0.5)]).unwrap();
        rel.insert_rel(0, 2, vec![Value::Int64(2021), Value::Double(0.8)]).unwrap();
        rel.insert_rel(1, 0, vec![Value::Int64(2020), Value::Double(0.3)]).unwrap();

        assert_eq!(rel.num_rows, 3);
        assert_eq!(rel.edges.len(), 3);

        // Forward adjacency from node 0
        let fwd = rel.scan_adj_list(0);
        assert_eq!(fwd.len(), 2);
        assert_eq!(fwd[0], (1, 0)); // (dst=1, edge_idx=0)
        assert_eq!(fwd[1], (2, 1)); // (dst=2, edge_idx=1)

        // Forward from node 1
        let fwd1 = rel.scan_adj_list(1);
        assert_eq!(fwd1.len(), 1);
        assert_eq!(fwd1[0], (0, 2));
    }

    #[test]
    fn test_rel_reverse_adjacency() {
        let mut rel = make_rel_table();
        rel.insert_rel(0, 5, vec![Value::Int64(2022), Value::Double(1.0)]).unwrap();
        rel.insert_rel(3, 5, vec![Value::Int64(2023), Value::Double(1.5)]).unwrap();

        // Node 5 has two incoming edges
        let rev = rel.scan_rev_adj_list(5);
        assert_eq!(rev.len(), 2);
        assert_eq!(rev[0], (0, 0));
        assert_eq!(rev[1], (3, 1));
    }

    #[test]
    fn test_rel_get_edge_properties() {
        let mut rel = make_rel_table();
        rel.insert_rel(0, 1, vec![Value::Int64(2020), Value::Double(0.5)]).unwrap();
        rel.insert_rel(2, 3, vec![Value::Int64(2021), Value::Double(0.9)]).unwrap();

        let props0 = rel.get_edge_properties(0);
        assert_eq!(props0, vec![Value::Int64(2020), Value::Double(0.5)]);

        let props1 = rel.get_edge_properties(1);
        assert_eq!(props1, vec![Value::Int64(2021), Value::Double(0.9)]);
    }

    #[test]
    fn test_rel_get_outgoing_edges() {
        let mut rel = make_rel_table();
        rel.insert_rel(0, 10, vec![Value::Int64(2020), Value::Double(1.0)]).unwrap();
        rel.insert_rel(0, 20, vec![Value::Int64(2021), Value::Double(2.0)]).unwrap();

        let outgoing = rel.get_outgoing_edges(0);
        assert_eq!(outgoing.len(), 2);
        assert_eq!(outgoing[0].0, 10);
        assert_eq!(outgoing[0].1, vec![Value::Int64(2020), Value::Double(1.0)]);
        assert_eq!(outgoing[1].0, 20);
    }

    #[test]
    fn test_rel_get_incoming_edges() {
        let mut rel = make_rel_table();
        rel.insert_rel(10, 5, vec![Value::Int64(2020), Value::Double(1.0)]).unwrap();
        rel.insert_rel(20, 5, vec![Value::Int64(2021), Value::Double(2.0)]).unwrap();

        let incoming = rel.get_incoming_edges(5);
        assert_eq!(incoming.len(), 2);
        assert_eq!(incoming[0].0, 10);
        assert_eq!(incoming[1].0, 20);
    }

    #[test]
    fn test_rel_no_edges() {
        let rel = make_rel_table();
        assert!(rel.scan_adj_list(0).is_empty());
        assert!(rel.scan_rev_adj_list(0).is_empty());
        assert!(rel.get_outgoing_edges(0).is_empty());
        assert!(rel.get_incoming_edges(0).is_empty());
    }

    #[test]
    fn test_rel_insert_row_legacy() {
        let mut rel = make_rel_table();
        // insert_row treats values as properties with sequential edge IDs
        rel.insert_row(vec![Value::Int64(2022), Value::Double(3.0)]).unwrap();
        assert_eq!(rel.num_rows, 1);
        assert_eq!(rel.edges[0], (0, 0)); // sequential from=0, to=0
        assert_eq!(rel.get_edge_properties(0), vec![Value::Int64(2022), Value::Double(3.0)]);
    }

    #[test]
    fn test_rel_wrong_column_count() {
        let mut rel = make_rel_table();
        let result = rel.insert_rel(0, 1, vec![Value::Int64(42)]); // 1 value, expected 2
        assert!(result.is_err());
    }

    #[test]
    fn test_rel_get_column() {
        let mut rel = make_rel_table();
        rel.insert_rel(0, 1, vec![Value::Int64(2020), Value::Double(1.5)]).unwrap();
        rel.insert_rel(1, 2, vec![Value::Int64(2021), Value::Double(2.5)]).unwrap();

        let since_col = rel.get_column(0).unwrap();
        assert_eq!(since_col, &[Value::Int64(2020), Value::Int64(2021)]);

        let weight_col = rel.get_column(1).unwrap();
        assert_eq!(weight_col, &[Value::Double(1.5), Value::Double(2.5)]);
    }

    #[test]
    fn test_rel_to_column_major() {
        let mut rel = make_rel_table();
        rel.insert_rel(0, 1, vec![Value::Int64(2020), Value::Double(0.5)]).unwrap();
        rel.insert_rel(2, 3, vec![Value::Int64(2021), Value::Double(0.9)]).unwrap();

        let data = rel.to_column_major_data();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0], vec![Value::Int64(2020), Value::Int64(2021)]);
        assert_eq!(data[1], vec![Value::Double(0.5), Value::Double(0.9)]);
    }

    // ==================== TableCatalog tests ====================

    #[test]
    fn test_catalog_create_and_lookup() {
        let mut cat = TableCatalog::new();
        let node_table = cat.create_node_table("Person".into(), vec![
            ColumnDefinition { name: "id".into(), logical_type: LogicalTypeID::Int64, is_primary_key: true },
        ]);
        assert_eq!(node_table.table_id, 0);

        let rel_table = cat.create_rel_table("Knows".into(), 0, 1, vec![
            ColumnDefinition { name: "since".into(), logical_type: LogicalTypeID::Int64, is_primary_key: false },
        ]);
        assert_eq!(rel_table.table_id, 1);

        assert!(cat.get_node_table(0).is_some());
        assert!(cat.get_rel_table(1).is_some());
        assert_eq!(cat.node_table_num_rows("Person"), 0);
    }
}
