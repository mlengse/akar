//! System catalog — manages schemas, tables, and type definitions.

use hashbrown::HashMap;
use kuzu_common::types::LogicalTypeID;

/// A table column definition in the catalog.
#[derive(Debug, Clone)]
pub struct CatalogColumn {
    pub name: String,
    pub logical_type: LogicalTypeID,
    pub is_primary_key: bool,
    pub default_value: Option<Vec<u8>>,
}

/// A node table entry in the catalog.
#[derive(Debug, Clone)]
pub struct NodeTableEntry {
    pub table_id: u64,
    pub name: String,
    pub columns: Vec<CatalogColumn>,
    pub primary_key_column: usize,
}

/// A relationship table entry in the catalog.
#[derive(Debug, Clone)]
pub struct RelTableEntry {
    pub table_id: u64,
    pub name: String,
    pub src_table_id: u64,
    pub dst_table_id: u64,
    pub columns: Vec<CatalogColumn>,
}

/// An entry in the system catalog (either a node table or rel table).
#[derive(Debug, Clone)]
pub enum CatalogEntry {
    NodeTable(NodeTableEntry),
    RelTable(RelTableEntry),
}

/// The system catalog manages all schema definitions.
#[derive(Debug, Default, Clone)]
pub struct Catalog {
    entries: HashMap<u64, CatalogEntry>,
    name_to_id: HashMap<String, u64>,
    next_id: u64,
}

impl Catalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_node_table(
        &mut self,
        name: String,
        columns: Vec<CatalogColumn>,
    ) -> u64 {
        let table_id = self.next_id;
        self.next_id += 1;
        let pk_col = columns.iter().position(|c| c.is_primary_key).unwrap_or(0);
        let entry = NodeTableEntry {
            table_id,
            name: name.clone(),
            columns,
            primary_key_column: pk_col,
        };
        self.entries
            .insert(table_id, CatalogEntry::NodeTable(entry));
        self.name_to_id.insert(name, table_id);
        table_id
    }

    pub fn create_rel_table(
        &mut self,
        name: String,
        src_table_id: u64,
        dst_table_id: u64,
        columns: Vec<CatalogColumn>,
    ) -> u64 {
        let table_id = self.next_id;
        self.next_id += 1;
        let entry = RelTableEntry {
            table_id,
            name: name.clone(),
            src_table_id,
            dst_table_id,
            columns,
        };
        self.entries
            .insert(table_id, CatalogEntry::RelTable(entry));
        self.name_to_id.insert(name, table_id);
        table_id
    }

    pub fn get_entry(&self, table_id: u64) -> Option<&CatalogEntry> {
        self.entries.get(&table_id)
    }

    pub fn get_entry_by_name(&self, name: &str) -> Option<&CatalogEntry> {
        self.name_to_id
            .get(name)
            .and_then(|id| self.entries.get(id))
    }

    pub fn get_table_id(&self, name: &str) -> Option<u64> {
        self.name_to_id.get(name).copied()
    }

    pub fn all_entries(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.entries.values()
    }
}
