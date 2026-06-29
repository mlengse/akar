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

impl NodeTableEntry {
    pub fn primary_key_column(&self) -> Option<&CatalogColumn> {
        self.columns.get(self.primary_key_column)
    }

    pub fn num_columns(&self) -> usize {
        self.columns.len()
    }
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

impl RelTableEntry {
    pub fn num_columns(&self) -> usize {
        self.columns.len()
    }
}

/// An entry in the system catalog (either a node table or rel table).
#[derive(Debug, Clone)]
pub enum CatalogEntry {
    NodeTable(NodeTableEntry),
    RelTable(RelTableEntry),
}

impl CatalogEntry {
    pub fn name(&self) -> &str {
        match self {
            CatalogEntry::NodeTable(t) => &t.name,
            CatalogEntry::RelTable(t) => &t.name,
        }
    }

    pub fn table_id(&self) -> u64 {
        match self {
            CatalogEntry::NodeTable(t) => t.table_id,
            CatalogEntry::RelTable(t) => t.table_id,
        }
    }

    pub fn columns(&self) -> &[CatalogColumn] {
        match self {
            CatalogEntry::NodeTable(t) => &t.columns,
            CatalogEntry::RelTable(t) => &t.columns,
        }
    }

    pub fn is_node_table(&self) -> bool {
        matches!(self, CatalogEntry::NodeTable(_))
    }

    pub fn is_rel_table(&self) -> bool {
        matches!(self, CatalogEntry::RelTable(_))
    }

    pub fn as_node_table(&self) -> Option<&NodeTableEntry> {
        match self {
            CatalogEntry::NodeTable(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_rel_table(&self) -> Option<&RelTableEntry> {
        match self {
            CatalogEntry::RelTable(t) => Some(t),
            _ => None,
        }
    }
}

/// Result of a catalog operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogResult {
    Created { table_id: u64 },
    Dropped { table_id: u64 },
    NotFound,
    AlreadyExists,
}

/// The system catalog manages all schema definitions.
///
/// Provides CRUD operations for tables (node and relationship tables),
/// schema validation, and lookup by name or ID.
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

    /// Create a node table. Returns error if name already exists.
    pub fn create_node_table(
        &mut self,
        name: String,
        columns: Vec<CatalogColumn>,
    ) -> CatalogResult {
        if self.name_to_id.contains_key(&name) {
            return CatalogResult::AlreadyExists;
        }
        let table_id = self.next_id;
        self.next_id += 1;
        let pk_col = columns.iter().position(|c| c.is_primary_key).unwrap_or(0);
        let entry = NodeTableEntry {
            table_id,
            name: name.clone(),
            columns,
            primary_key_column: pk_col,
        };
        self.entries.insert(table_id, CatalogEntry::NodeTable(entry));
        self.name_to_id.insert(name, table_id);
        CatalogResult::Created { table_id }
    }

    /// Create a rel table. Returns error if name already exists.
    pub fn create_rel_table(
        &mut self,
        name: String,
        src_table_id: u64,
        dst_table_id: u64,
        columns: Vec<CatalogColumn>,
    ) -> CatalogResult {
        if self.name_to_id.contains_key(&name) {
            return CatalogResult::AlreadyExists;
        }
        let table_id = self.next_id;
        self.next_id += 1;
        let entry = RelTableEntry {
            table_id,
            name: name.clone(),
            src_table_id,
            dst_table_id,
            columns,
        };
        self.entries.insert(table_id, CatalogEntry::RelTable(entry));
        self.name_to_id.insert(name, table_id);
        CatalogResult::Created { table_id }
    }

    /// Drop a table by name.
    pub fn drop_table(&mut self, name: &str) -> CatalogResult {
        if let Some(&table_id) = self.name_to_id.get(name) {
            self.entries.remove(&table_id);
            self.name_to_id.remove(name);
            CatalogResult::Dropped { table_id }
        } else {
            CatalogResult::NotFound
        }
    }

    /// Get a catalog entry by table ID.
    pub fn get_entry(&self, table_id: u64) -> Option<&CatalogEntry> {
        self.entries.get(&table_id)
    }

    /// Get a catalog entry by table name.
    pub fn get_entry_by_name(&self, name: &str) -> Option<&CatalogEntry> {
        self.name_to_id.get(name).and_then(|id| self.entries.get(id))
    }

    /// Get a mutable catalog entry by table name.
    pub fn get_entry_by_name_mut(&mut self, name: &str) -> Option<&mut CatalogEntry> {
        let id = self.name_to_id.get(name).copied()?;
        self.entries.get_mut(&id)
    }

    /// Add a column to a table in the catalog.
    pub fn add_column(&mut self, table_name: &str, column: CatalogColumn) -> Result<(), String> {
        let entry = self.get_entry_by_name_mut(table_name)
            .ok_or_else(|| format!("Table '{table_name}' not found"))?;
        match entry {
            CatalogEntry::NodeTable(t) => {
                if t.columns.iter().any(|c| c.name.eq_ignore_ascii_case(&column.name)) {
                    return Err(format!("Column '{}' already exists", column.name));
                }
                t.columns.push(column);
                Ok(())
            }
            CatalogEntry::RelTable(t) => {
                if t.columns.iter().any(|c| c.name.eq_ignore_ascii_case(&column.name)) {
                    return Err(format!("Column '{}' already exists", column.name));
                }
                t.columns.push(column);
                Ok(())
            }
        }
    }

    /// Drop a column from a table in the catalog.
    pub fn drop_column(&mut self, table_name: &str, column_name: &str) -> Result<(), String> {
        let entry = self.get_entry_by_name_mut(table_name)
            .ok_or_else(|| format!("Table '{table_name}' not found"))?;
        match entry {
            CatalogEntry::NodeTable(t) => {
                let pos = t.columns.iter().position(|c| c.name == column_name)
                    .ok_or_else(|| format!("Column '{column_name}' not found"))?;
                t.columns.remove(pos);
                Ok(())
            }
            CatalogEntry::RelTable(t) => {
                let pos = t.columns.iter().position(|c| c.name == column_name)
                    .ok_or_else(|| format!("Column '{column_name}' not found"))?;
                t.columns.remove(pos);
                Ok(())
            }
        }
    }

    /// Rename a column in a table in the catalog.
    pub fn rename_column(&mut self, table_name: &str, old_name: &str, new_name: &str) -> Result<(), String> {
        // Check for duplicates before the mutable borrow
        let cols = self.get_entry_by_name(table_name)
            .ok_or_else(|| format!("Table '{table_name}' not found"))?
            .columns().to_vec();
        if !cols.iter().any(|c| c.name == old_name) {
            return Err(format!("Column '{old_name}' not found"));
        }
        if cols.iter().any(|c| c.name == new_name) {
            return Err(format!("Column '{new_name}' already exists"));
        }
        drop(cols);

        let entry = self.get_entry_by_name_mut(table_name).unwrap();
        match entry {
            CatalogEntry::NodeTable(t) => {
                let col = t.columns.iter_mut().find(|c| c.name == old_name).unwrap();
                col.name = new_name.to_string();
                Ok(())
            }
            CatalogEntry::RelTable(t) => {
                let col = t.columns.iter_mut().find(|c| c.name == old_name).unwrap();
                col.name = new_name.to_string();
                Ok(())
            }
        }
    }

    /// Rename a table in the catalog.
    pub fn rename_table(&mut self, old_name: &str, new_name: &str) -> Result<(), String> {
        let id = self.name_to_id.get(old_name).copied()
            .ok_or_else(|| format!("Table '{old_name}' not found"))?;
        if self.name_to_id.contains_key(new_name) {
            return Err(format!("Table '{new_name}' already exists"));
        }
        match self.entries.get_mut(&id) {
            Some(CatalogEntry::NodeTable(t)) => t.name = new_name.to_string(),
            Some(CatalogEntry::RelTable(t)) => t.name = new_name.to_string(),
            None => return Err("Table not found".into()),
        }
        self.name_to_id.remove(old_name);
        self.name_to_id.insert(new_name.to_string(), id);
        Ok(())
    }

    /// Get table ID by name.
    pub fn get_table_id(&self, name: &str) -> Option<u64> {
        self.name_to_id.get(name).copied()
    }

    /// Check if a table exists.
    pub fn contains(&self, name: &str) -> bool {
        self.name_to_id.contains_key(name)
    }

    /// Get the number of tables in the catalog.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over all catalog entries.
    pub fn all_entries(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.entries.values()
    }

    /// Get all node table entries.
    pub fn node_tables(&self) -> Vec<&NodeTableEntry> {
        self.entries
            .values()
            .filter_map(|e| match e {
                CatalogEntry::NodeTable(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    /// Get all rel table entries.
    pub fn rel_tables(&self) -> Vec<&RelTableEntry> {
        self.entries
            .values()
            .filter_map(|e| match e {
                CatalogEntry::RelTable(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    /// Rename a table.
    pub fn rename(&mut self, old_name: &str, new_name: String) -> CatalogResult {
        if !self.name_to_id.contains_key(old_name) {
            return CatalogResult::NotFound;
        }
        if self.name_to_id.contains_key(&new_name) {
            return CatalogResult::AlreadyExists;
        }
        let table_id = self.name_to_id.remove(old_name).unwrap();
        self.name_to_id.insert(new_name.clone(), table_id);

        // Update the entry's name
        if let Some(entry) = self.entries.get_mut(&table_id) {
            match entry {
                CatalogEntry::NodeTable(t) => t.name = new_name.clone(),
                CatalogEntry::RelTable(t) => t.name = new_name,
            }
        }
        CatalogResult::Created { table_id }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_node_columns() -> Vec<CatalogColumn> {
        vec![
            CatalogColumn { name: "name".into(), logical_type: LogicalTypeID::String, is_primary_key: true, default_value: None },
            CatalogColumn { name: "age".into(), logical_type: LogicalTypeID::Int64, is_primary_key: false, default_value: None },
        ]
    }

    #[test]
    fn test_create_node_table() {
        let mut cat = Catalog::new();
        let result = cat.create_node_table("Person".into(), sample_node_columns());
        assert!(matches!(result, CatalogResult::Created { .. }));
        assert_eq!(cat.len(), 1);
    }

    #[test]
    fn test_create_rel_table() {
        let mut cat = Catalog::new();
        cat.create_node_table("Person".into(), sample_node_columns());
        let result = cat.create_rel_table("Knows".into(), 0, 0, vec![
            CatalogColumn { name: "since".into(), logical_type: LogicalTypeID::Int64, is_primary_key: false, default_value: None },
        ]);
        assert!(matches!(result, CatalogResult::Created { .. }));
        assert_eq!(cat.len(), 2);
    }

    #[test]
    fn test_drop_table() {
        let mut cat = Catalog::new();
        cat.create_node_table("Person".into(), sample_node_columns());
        let result = cat.drop_table("Person");
        assert!(matches!(result, CatalogResult::Dropped { .. }));
        assert!(cat.is_empty());
    }

    #[test]
    fn test_drop_nonexistent() {
        let mut cat = Catalog::new();
        assert_eq!(cat.drop_table("Ghost"), CatalogResult::NotFound);
    }

    #[test]
    fn test_duplicate_name() {
        let mut cat = Catalog::new();
        cat.create_node_table("Person".into(), sample_node_columns());
        assert_eq!(cat.create_node_table("Person".into(), sample_node_columns()), CatalogResult::AlreadyExists);
    }

    #[test]
    fn test_lookup_by_name() {
        let mut cat = Catalog::new();
        cat.create_node_table("Person".into(), sample_node_columns());
        let entry = cat.get_entry_by_name("Person");
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().name(), "Person");
    }

    #[test]
    fn test_lookup_by_id() {
        let mut cat = Catalog::new();
        let id = match cat.create_node_table("Person".into(), sample_node_columns()) {
            CatalogResult::Created { table_id } => table_id,
            _ => panic!("Failed to create"),
        };
        let entry = cat.get_entry(id);
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().table_id(), id);
    }

    #[test]
    fn test_get_table_id() {
        let mut cat = Catalog::new();
        cat.create_node_table("Person".into(), sample_node_columns());
        assert!(cat.get_table_id("Person").is_some());
        assert!(cat.get_table_id("Ghost").is_none());
    }

    #[test]
    fn test_rename() {
        let mut cat = Catalog::new();
        cat.create_node_table("Person".into(), sample_node_columns());
        assert!(matches!(cat.rename("Person", "Employee".into()), CatalogResult::Created { .. }));
        assert!(cat.contains("Employee"));
        assert!(!cat.contains("Person"));
    }

    #[test]
    fn test_rename_nonexistent() {
        let mut cat = Catalog::new();
        assert_eq!(cat.rename("Ghost", "NewName".into()), CatalogResult::NotFound);
    }

    #[test]
    fn test_node_tables_filter() {
        let mut cat = Catalog::new();
        cat.create_node_table("Person".into(), sample_node_columns());
        cat.create_rel_table("Knows".into(), 0, 0, vec![]);
        let nodes = cat.node_tables();
        let rels = cat.rel_tables();
        assert_eq!(nodes.len(), 1);
        assert_eq!(rels.len(), 1);
    }

    #[test]
    fn test_entry_helpers() {
        let mut cat = Catalog::new();
        cat.create_node_table("Person".into(), sample_node_columns());
        let entry = cat.get_entry_by_name("Person").unwrap();
        assert!(entry.is_node_table());
        assert!(!entry.is_rel_table());
        assert!(entry.as_node_table().is_some());
        assert!(entry.as_rel_table().is_none());
        assert_eq!(entry.columns().len(), 2);
    }

    #[test]
    fn test_catalog_empty() {
        let cat = Catalog::new();
        assert!(cat.is_empty());
        assert_eq!(cat.len(), 0);
    }

    #[test]
    fn test_all_entries() {
        let mut cat = Catalog::new();
        cat.create_node_table("A".into(), vec![]);
        cat.create_node_table("B".into(), vec![]);
        assert_eq!(cat.all_entries().count(), 2);
    }
}
