//! Table storage — columnar node/rel tables with CSR adjacency index.

use kuzu_common::types::{LogicalTypeID, Value};
use std::collections::HashMap;

/// A column definition within a table.
#[derive(Debug, Clone)]
pub struct ColumnDefinition {
    pub name: String,
    pub logical_type: LogicalTypeID,
    pub is_primary_key: bool,
}

/// A node table stores properties for a node label.
#[derive(Debug, Clone)]
pub struct NodeTable {
    pub table_id: u64,
    pub name: String,
    pub columns: Vec<ColumnDefinition>,
    pub primary_key_column: usize,
    pub num_rows: u64,
    /// Column-major in-memory data storage: data[col_idx][row_idx]
    pub data: Vec<Vec<Value>>,
}

impl NodeTable {
    pub fn new(table_id: u64, name: String, columns: Vec<ColumnDefinition>) -> Self {
        let primary_key_column = columns
            .iter()
            .position(|c| c.is_primary_key)
            .unwrap_or(0);
        let num_cols = columns.len();
        Self {
            table_id,
            name,
            columns,
            primary_key_column,
            num_rows: 0,
            data: vec![Vec::new(); num_cols],
        }
    }

    /// Insert a row of values into the table (column-major storage).
    /// Returns an error if the number of values doesn't match the number of columns.
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

    /// Get a single value at (row, col).
    pub fn get_value(&self, row: usize, col: usize) -> Option<&Value> {
        self.data.get(col).and_then(|col_data| col_data.get(row))
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
