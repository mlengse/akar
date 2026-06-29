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

/// A relationship (edge) table with CSR adjacency storage.
#[derive(Debug, Clone)]
pub struct RelTable {
    pub table_id: u64,
    pub name: String,
    pub src_table_id: u64,
    pub dst_table_id: u64,
    pub columns: Vec<ColumnDefinition>,
    pub num_rows: u64,
    /// Column-major in-memory data storage: data[col_idx][row_idx]
    pub data: Vec<Vec<Value>>,
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
            data: vec![Vec::new(); num_cols],
        }
    }

    /// Insert a row of values into the table (column-major storage).
    pub fn insert_row(&mut self, values: Vec<Value>) -> Result<(), String> {
        if values.len() != self.columns.len() {
            return Err(format!(
                "Column count mismatch: expected {} values, got {}",
                self.columns.len(),
                values.len()
            ));
        }
        for (col_idx, val) in values.into_iter().enumerate() {
            self.data[col_idx].push(val);
        }
        self.num_rows += 1;
        Ok(())
    }

    /// Get all values for a given column (by index) as a slice.
    pub fn get_column(&self, col_idx: usize) -> Option<&[Value]> {
        self.data.get(col_idx).map(|v| v.as_slice())
    }

    /// Return column-major data (clone of the internal Vec<Vec<Value>>).
    pub fn to_column_major_data(&self) -> Vec<Vec<Value>> {
        self.data.clone()
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
