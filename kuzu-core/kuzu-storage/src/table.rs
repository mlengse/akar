//! Table storage — columnar node/rel tables with CSR adjacency index.

use kuzu_common::types::LogicalTypeID;
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
}

impl RelTable {
    pub fn new(
        table_id: u64,
        name: String,
        src_table_id: u64,
        dst_table_id: u64,
        columns: Vec<ColumnDefinition>,
    ) -> Self {
        Self {
            table_id,
            name,
            src_table_id,
            dst_table_id,
            columns,
            num_rows: 0,
        }
    }
}

/// A collection of tables managed by the storage engine.
#[derive(Debug, Default)]
pub struct TableCatalog {
    node_tables: HashMap<u64, NodeTable>,
    rel_tables: HashMap<u64, RelTable>,
    next_table_id: u64,
}

impl TableCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_node_table(&mut self, name: String, columns: Vec<ColumnDefinition>) -> NodeTable {
        let table_id = self.next_table_id;
        self.next_table_id += 1;
        let table = NodeTable::new(table_id, name, columns);
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
        let table = RelTable::new(table_id, name, src_table_id, dst_table_id, columns);
        self.rel_tables.insert(table_id, table.clone());
        table
    }

    pub fn get_node_table(&self, table_id: u64) -> Option<&NodeTable> {
        self.node_tables.get(&table_id)
    }

    pub fn get_rel_table(&self, table_id: u64) -> Option<&RelTable> {
        self.rel_tables.get(&table_id)
    }

    pub fn all_node_tables(&self) -> impl Iterator<Item = &NodeTable> {
        self.node_tables.values()
    }

    pub fn all_rel_tables(&self) -> impl Iterator<Item = &RelTable> {
        self.rel_tables.values()
    }
}
