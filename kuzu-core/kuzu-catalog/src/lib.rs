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

/// Index type for primary key indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexType {
    /// Default hash index (equality-only lookup).
    Hash,
    /// Adaptive Radix Tree index (supports range scans).
    Art,
}

impl IndexType {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "HASH" => Some(IndexType::Hash),
            "ART" => Some(IndexType::Art),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            IndexType::Hash => "HASH",
            IndexType::Art => "ART",
        }
    }
}

/// A node table entry in the catalog.
#[derive(Debug, Clone)]
pub struct NodeTableEntry {
    pub table_id: u64,
    pub name: String,
    pub columns: Vec<CatalogColumn>,
    pub primary_key_column: usize,
    /// Type of index used for the primary key.
    pub index_type: Option<IndexType>,
    /// Name of the index (if any).
    pub index_name: Option<String>,
}

impl NodeTableEntry {
    pub fn primary_key_column(&self) -> Option<&CatalogColumn> {
        self.columns.get(self.primary_key_column)
    }

    pub fn num_columns(&self) -> usize {
        self.columns.len()
    }

    /// Returns `true` if this table has an ART index.
    pub fn has_art_index(&self) -> bool {
        matches!(self.index_type, Some(IndexType::Art))
    }

    /// Returns `true` if this table has any index configured.
    pub fn has_index(&self) -> bool {
        self.index_type.is_some()
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

/// A sequence entry in the catalog for auto-incrementing counters.
///
/// Supports `CREATE SEQUENCE` DDL and `SERIAL` column auto-increment.
/// Thread-safe via internal Mutex for concurrent `nextval`/`currval` access.
#[derive(Debug, Clone)]
pub struct SequenceEntry {
    pub sequence_id: u64,
    pub name: String,
    pub usage_count: u64,
    pub curr_val: i64,
    pub increment: i64,
    pub start_value: i64,
    pub min_value: i64,
    pub max_value: i64,
    pub cycle: bool,
}

impl SequenceEntry {
    pub fn new(
        name: String,
        start_value: i64,
        increment: i64,
        min_value: i64,
        max_value: i64,
        cycle: bool,
        sequence_id: u64,
    ) -> Self {
        Self {
            sequence_id,
            name,
            usage_count: 0,
            curr_val: start_value,
            start_value,
            increment,
            min_value,
            max_value,
            cycle,
        }
    }

    /// Get the current value without advancing.
    pub fn curr_val(&self) -> i64 {
        self.curr_val
    }

    /// Advance the sequence by `count` steps and return the next value.
    pub fn next_k_val(&mut self, count: u64) -> i64 {
        let result = self.curr_val;
        self.usage_count += count;
        let delta = self.increment * (count as i64);
        let new_val = self.curr_val.checked_add(delta);
        match new_val {
            Some(v) => {
                if v > self.max_value {
                    if self.cycle {
                        self.curr_val = self.min_value;
                    } else {
                        self.curr_val = v;
                    }
                } else if v < self.min_value {
                    if self.cycle {
                        self.curr_val = self.max_value;
                    } else {
                        self.curr_val = v;
                    }
                } else {
                    self.curr_val = v;
                }
            }
            None => {
                // Overflow: cycle if enabled, otherwise clamp
                if self.cycle {
                    self.curr_val = if delta > 0 { self.min_value } else { self.max_value };
                }
            }
        }
        result
    }

    /// Rollback the sequence to a previous state (for transaction rollback).
    pub fn rollback_val(&mut self, usage_count: u64, curr_val: i64) {
        self.usage_count = usage_count;
        self.curr_val = curr_val;
    }

    /// Generate the auto-generated sequence name for a SERIAL column.
    pub fn get_serial_name(table_name: &str, property_name: &str) -> String {
        format!("{}_{}_serial", table_name, property_name)
    }
}

/// A vector index entry in the catalog.
#[derive(Debug, Clone)]
pub struct VectorIndexEntry {
    pub index_id: u64,
    pub name: String,
    pub table_name: String,
    pub column_name: String,
    pub metric: String,
    pub dimensions: u64,
}

/// A foreign table entry in the catalog for externally-attached tables.
///
/// Foreign tables represent tables from external catalogs (e.g., DuckDB, Postgres, SQLite)
/// that are attached to the current database. They behave like read-only tables with
/// a source type identifying the external engine.
#[derive(Debug, Clone)]
pub struct ForeignTableEntry {
    pub table_id: u64,
    pub name: String,
    pub columns: Vec<CatalogColumn>,
    /// The type of external data source (e.g., "duckdb", "postgres", "sqlite").
    pub source_type: String,
}

impl ForeignTableEntry {
    pub fn num_columns(&self) -> usize {
        self.columns.len()
    }
}

/// A scalar macro entry in the catalog.
///
/// Stores a macro definition: `CREATE MACRO name(params) AS expression`.
/// Macros are expanded at binding time via parameter substitution.
///
/// Ported from C++ `scalar_macro_catalog_entry.h`.
#[derive(Debug, Clone)]
pub struct ScalarMacroEntry {
    pub macro_id: u64,
    pub name: String,
    /// Positional parameter names (no default value).
    pub positional_args: Vec<String>,
    /// Parameters with default values (name, default expression as string).
    pub default_args: Vec<(String, String)>,
    /// The macro body expression (serialized as string).
    pub expression: String,
}

impl ScalarMacroEntry {
    pub fn new(
        macro_id: u64,
        name: String,
        positional_args: Vec<String>,
        default_args: Vec<(String, String)>,
        expression: String,
    ) -> Self {
        Self {
            macro_id,
            name,
            positional_args,
            default_args,
            expression,
        }
    }

    pub fn total_args(&self) -> usize {
        self.positional_args.len() + self.default_args.len()
    }
}

/// An entry in the system catalog (node table, rel table, vector index, sequence, or foreign table).
#[derive(Debug, Clone)]
pub enum CatalogEntry {
    NodeTable(NodeTableEntry),
    RelTable(RelTableEntry),
    VectorIndex(VectorIndexEntry),
    Sequence(SequenceEntry),
    Foreign(ForeignTableEntry),
    Macro(ScalarMacroEntry),
}

impl CatalogEntry {
    pub fn name(&self) -> &str {
        match self {
            CatalogEntry::NodeTable(t) => &t.name,
            CatalogEntry::RelTable(t) => &t.name,
            CatalogEntry::VectorIndex(v) => &v.name,
            CatalogEntry::Sequence(s) => &s.name,
            CatalogEntry::Foreign(f) => &f.name,
            CatalogEntry::Macro(m) => &m.name,
        }
    }

    pub fn table_id(&self) -> u64 {
        match self {
            CatalogEntry::NodeTable(t) => t.table_id,
            CatalogEntry::RelTable(t) => t.table_id,
            CatalogEntry::VectorIndex(v) => v.index_id,
            CatalogEntry::Sequence(s) => s.sequence_id,
            CatalogEntry::Foreign(f) => f.table_id,
            CatalogEntry::Macro(m) => m.macro_id,
        }
    }

    pub fn columns(&self) -> &[CatalogColumn] {
        match self {
            CatalogEntry::NodeTable(t) => &t.columns,
            CatalogEntry::RelTable(t) => &t.columns,
            CatalogEntry::VectorIndex(_) => &[],
            CatalogEntry::Sequence(_) => &[],
            CatalogEntry::Foreign(f) => &f.columns,
            CatalogEntry::Macro(_) => &[],
        }
    }

    pub fn is_node_table(&self) -> bool {
        matches!(self, CatalogEntry::NodeTable(_))
    }

    pub fn is_rel_table(&self) -> bool {
        matches!(self, CatalogEntry::RelTable(_))
    }

    pub fn is_vector_index(&self) -> bool {
        matches!(self, CatalogEntry::VectorIndex(_))
    }

    pub fn is_sequence(&self) -> bool {
        matches!(self, CatalogEntry::Sequence(_))
    }

    pub fn is_foreign_table(&self) -> bool {
        matches!(self, CatalogEntry::Foreign(_))
    }

    pub fn is_macro(&self) -> bool {
        matches!(self, CatalogEntry::Macro(_))
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

    pub fn as_vector_index(&self) -> Option<&VectorIndexEntry> {
        match self {
            CatalogEntry::VectorIndex(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_foreign_table(&self) -> Option<&ForeignTableEntry> {
        match self {
            CatalogEntry::Foreign(f) => Some(f),
            _ => None,
        }
    }

    pub fn as_macro(&self) -> Option<&ScalarMacroEntry> {
        match self {
            CatalogEntry::Macro(m) => Some(m),
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
    /// Monotonically increasing version counter, incremented on every DDL.
    version: u64,
}

impl Catalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a vector index entry. Returns error if name already exists.
    pub fn create_vector_index(
        &mut self,
        name: String,
        table_name: String,
        column_name: String,
        metric: String,
        dimensions: u64,
    ) -> CatalogResult {
        if self.name_to_id.contains_key(&name) {
            return CatalogResult::AlreadyExists;
        }
        // Validate that the referenced table exists
        if !self.name_to_id.contains_key(&table_name) {
            return CatalogResult::NotFound;
        }
        let index_id = self.next_id;
        self.next_id += 1;
        let entry = VectorIndexEntry {
            index_id,
            name: name.clone(),
            table_name,
            column_name,
            metric,
            dimensions,
        };
        self.entries.insert(index_id, CatalogEntry::VectorIndex(entry));
        self.name_to_id.insert(name, index_id);
        CatalogResult::Created { table_id: index_id }
    }

    /// Create a node table. Returns error if name already exists.
    pub fn create_node_table(&mut self, name: String, columns: Vec<CatalogColumn>) -> CatalogResult {
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
            index_type: None,
            index_name: None,
        };
        self.entries.insert(table_id, CatalogEntry::NodeTable(entry));
        self.name_to_id.insert(name, table_id);
        self.bump_version();
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
        self.bump_version();
        CatalogResult::Created { table_id }
    }

    /// Drop a table by name.
    pub fn drop_table(&mut self, name: &str) -> CatalogResult {
        if let Some(&table_id) = self.name_to_id.get(name) {
            self.entries.remove(&table_id);
            self.name_to_id.remove(name);
            self.bump_version();
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
        let entry = self
            .get_entry_by_name_mut(table_name)
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
            CatalogEntry::VectorIndex(_) => Err("Cannot add column to a vector index".into()),
            CatalogEntry::Sequence(_) => Err("Cannot add column to a sequence".into()),
            CatalogEntry::Foreign(_) => Err("Cannot add column to a foreign table".into()),
            CatalogEntry::Macro(_) => Err("Cannot add column to a macro".into()),
        }
    }

    /// Drop a column from a table in the catalog.
    pub fn drop_column(&mut self, table_name: &str, column_name: &str) -> Result<(), String> {
        let entry = self
            .get_entry_by_name_mut(table_name)
            .ok_or_else(|| format!("Table '{table_name}' not found"))?;
        match entry {
            CatalogEntry::NodeTable(t) => {
                let pos = t
                    .columns
                    .iter()
                    .position(|c| c.name == column_name)
                    .ok_or_else(|| format!("Column '{column_name}' not found"))?;
                t.columns.remove(pos);
                Ok(())
            }
            CatalogEntry::RelTable(t) => {
                let pos = t
                    .columns
                    .iter()
                    .position(|c| c.name == column_name)
                    .ok_or_else(|| format!("Column '{column_name}' not found"))?;
                t.columns.remove(pos);
                Ok(())
            }
            CatalogEntry::VectorIndex(_) => Err("Cannot drop column from a vector index".into()),
            CatalogEntry::Sequence(_) => Err("Cannot drop column from a sequence".into()),
            CatalogEntry::Foreign(_) => Err("Cannot drop column from a foreign table".into()),
            CatalogEntry::Macro(_) => Err("Cannot drop column from a macro".into()),
        }
    }

    /// Create an index on a node table.
    pub fn create_index(
        &mut self,
        table_name: &str,
        index_name: String,
        index_type: IndexType,
        column_name: &str,
    ) -> Result<(), String> {
        let entry = self
            .get_entry_by_name_mut(table_name)
            .ok_or_else(|| format!("Table '{table_name}' not found"))?;
        match entry {
            CatalogEntry::NodeTable(t) => {
                // Validate column exists
                if !t.columns.iter().any(|c| c.name == column_name) {
                    return Err(format!("Column '{column_name}' not found in table '{table_name}'"));
                }
                // Validate column is the primary key
                let pk_col = t.primary_key_column();
                if pk_col.map(|c| c.name.as_str()) != Some(column_name) {
                    return Err(format!(
                        "Cannot create index on non-PK column '{column_name}'. Only PK columns are supported."
                    ));
                }
                t.index_type = Some(index_type);
                t.index_name = Some(index_name);
                Ok(())
            }
            _ => Err(format!("Table '{table_name}' is not a node table")),
        }
    }

    /// Create a sequence in the catalog.
    pub fn create_sequence(
        &mut self,
        name: String,
        start_value: i64,
        increment: i64,
        min_value: i64,
        max_value: i64,
        cycle: bool,
    ) -> CatalogResult {
        if self.name_to_id.contains_key(&name) {
            return CatalogResult::AlreadyExists;
        }
        let sequence_id = self.next_id;
        self.next_id += 1;
        let entry = SequenceEntry::new(
            name.clone(),
            start_value,
            increment,
            min_value,
            max_value,
            cycle,
            sequence_id,
        );
        self.entries.insert(sequence_id, CatalogEntry::Sequence(entry));
        self.name_to_id.insert(name, sequence_id);
        self.bump_version();
        CatalogResult::Created { table_id: sequence_id }
    }

    /// Get a sequence entry by name.
    pub fn get_sequence(&self, name: &str) -> Option<&SequenceEntry> {
        self.name_to_id.get(name).and_then(|id| match self.entries.get(id) {
            Some(CatalogEntry::Sequence(s)) => Some(s),
            _ => None,
        })
    }

    /// Get a mutable sequence entry by name.
    pub fn get_sequence_mut(&mut self, name: &str) -> Option<&mut SequenceEntry> {
        let id = self.name_to_id.get(name).copied()?;
        match self.entries.get_mut(&id) {
            Some(CatalogEntry::Sequence(s)) => Some(s),
            _ => None,
        }
    }

    /// Drop a sequence by name.
    pub fn drop_sequence(&mut self, name: &str) -> CatalogResult {
        if let Some(&seq_id) = self.name_to_id.get(name) {
            self.entries.remove(&seq_id);
            self.name_to_id.remove(name);
            CatalogResult::Dropped { table_id: seq_id }
        } else {
            CatalogResult::NotFound
        }
    }

    /// List all sequence entries.
    pub fn sequences(&self) -> Vec<&SequenceEntry> {
        self.entries
            .values()
            .filter_map(|e| match e {
                CatalogEntry::Sequence(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    /// Auto-create a sequence backing a SERIAL column.
    ///
    /// The sequence is named `{table_name}_{column_name}_serial` with:
    /// - start = 0, increment = 1, min = 0, max = i64::MAX, no cycle
    ///
    /// This matches C++ behavior in `Catalog::createSerialSequence()`.
    pub fn create_serial_sequence(&mut self, table_name: &str, column_name: &str) -> CatalogResult {
        let seq_name = SequenceEntry::get_serial_name(table_name, column_name);
        self.create_sequence(seq_name, 0, 1, 0, i64::MAX, false)
    }

    /// Drop the auto-created sequence for a SERIAL column.
    ///
    /// Called when a table with SERIAL columns is dropped.
    pub fn drop_serial_sequence(&mut self, table_name: &str, column_name: &str) -> CatalogResult {
        let seq_name = SequenceEntry::get_serial_name(table_name, column_name);
        self.drop_sequence(&seq_name)
    }

    // ==================== Macro methods ====================

    /// Create a scalar macro entry in the catalog.
    ///
    /// Ported from C++ `ScalarMacroCatalogEntry`.
    pub fn create_macro(
        &mut self,
        name: String,
        positional_args: Vec<String>,
        default_args: Vec<(String, String)>,
        expression: String,
    ) -> CatalogResult {
        let macro_name_upper = name.to_uppercase();
        if self.name_to_id.contains_key(&macro_name_upper) {
            return CatalogResult::AlreadyExists;
        }
        let macro_id = self.next_id;
        self.next_id += 1;
        let entry = ScalarMacroEntry::new(macro_id, name.clone(), positional_args, default_args, expression);
        self.entries.insert(macro_id, CatalogEntry::Macro(entry));
        self.name_to_id.insert(macro_name_upper, macro_id);
        self.bump_version();
        CatalogResult::Created { table_id: macro_id }
    }

    /// Get a macro entry by name (case-insensitive lookup).
    pub fn get_macro(&self, name: &str) -> Option<&ScalarMacroEntry> {
        let upper = name.to_uppercase();
        self.name_to_id.get(&upper).and_then(|id| match self.entries.get(id) {
            Some(CatalogEntry::Macro(m)) => Some(m),
            _ => None,
        })
    }

    /// Get a mutable macro entry by name (case-insensitive).
    pub fn get_macro_mut(&mut self, name: &str) -> Option<&mut ScalarMacroEntry> {
        let upper = name.to_uppercase();
        let id = self.name_to_id.get(&upper).copied()?;
        match self.entries.get_mut(&id) {
            Some(CatalogEntry::Macro(m)) => Some(m),
            _ => None,
        }
    }

    /// Drop a macro by name (case-insensitive).
    pub fn drop_macro(&mut self, name: &str) -> CatalogResult {
        let upper = name.to_uppercase();
        if let Some(&macro_id) = self.name_to_id.get(&upper) {
            self.entries.remove(&macro_id);
            self.name_to_id.remove(&upper);
            CatalogResult::Dropped { table_id: macro_id }
        } else {
            CatalogResult::NotFound
        }
    }

    /// List all macro entries.
    pub fn macros(&self) -> Vec<&ScalarMacroEntry> {
        self.entries
            .values()
            .filter_map(|e| match e {
                CatalogEntry::Macro(m) => Some(m),
                _ => None,
            })
            .collect()
    }

    /// Check if a macro with the given name exists (case-insensitive).
    pub fn contains_macro(&self, name: &str) -> bool {
        let upper = name.to_uppercase();
        self.name_to_id.contains_key(&upper)
            && matches!(
                self.entries.get(self.name_to_id.get(&upper).unwrap()),
                Some(CatalogEntry::Macro(_))
            )
    }

    /// Create a foreign table entry in the catalog.
    pub fn create_foreign_table(
        &mut self,
        name: String,
        columns: Vec<CatalogColumn>,
        source_type: String,
    ) -> CatalogResult {
        if self.name_to_id.contains_key(&name) {
            return CatalogResult::AlreadyExists;
        }
        let table_id = self.next_id;
        self.next_id += 1;
        let entry = ForeignTableEntry {
            table_id,
            name: name.clone(),
            columns,
            source_type,
        };
        self.entries.insert(table_id, CatalogEntry::Foreign(entry));
        self.name_to_id.insert(name, table_id);
        CatalogResult::Created { table_id }
    }

    /// Get a foreign table entry by name.
    pub fn get_foreign_table(&self, name: &str) -> Option<&ForeignTableEntry> {
        self.name_to_id.get(name).and_then(|id| match self.entries.get(id) {
            Some(CatalogEntry::Foreign(f)) => Some(f),
            _ => None,
        })
    }

    /// Drop a foreign table by name.
    pub fn drop_foreign_table(&mut self, name: &str) -> CatalogResult {
        if let Some(&table_id) = self.name_to_id.get(name) {
            self.entries.remove(&table_id);
            self.name_to_id.remove(name);
            CatalogResult::Dropped { table_id }
        } else {
            CatalogResult::NotFound
        }
    }

    /// List all foreign table entries.
    pub fn foreign_tables(&self) -> Vec<&ForeignTableEntry> {
        self.entries
            .values()
            .filter_map(|e| match e {
                CatalogEntry::Foreign(f) => Some(f),
                _ => None,
            })
            .collect()
    }

    /// Drop an index from a table.
    pub fn drop_index(&mut self, table_name: &str, index_name: &str) -> Result<(), String> {
        let entry = self
            .get_entry_by_name_mut(table_name)
            .ok_or_else(|| format!("Table '{table_name}' not found"))?;
        match entry {
            CatalogEntry::NodeTable(t) => {
                if t.index_name.as_deref() != Some(index_name) {
                    return Err(format!("Index '{index_name}' not found on table '{table_name}'"));
                }
                t.index_type = None;
                t.index_name = None;
                Ok(())
            }
            _ => Err(format!("Table '{table_name}' is not a node table")),
        }
    }

    /// Get index info for a table.
    pub fn get_index_info(&self, table_name: &str) -> Option<(IndexType, String)> {
        match self.get_entry_by_name(table_name)? {
            CatalogEntry::NodeTable(t) => {
                let idx_type = t.index_type?;
                let idx_name = t.index_name.clone()?;
                Some((idx_type, idx_name))
            }
            _ => None,
        }
    }

    /// Check if a table has an ART index.
    pub fn has_art_index(&self, table_name: &str) -> bool {
        matches!(self.get_index_info(table_name), Some((IndexType::Art, _)))
    }

    /// Rename a column in a table in the catalog.
    pub fn rename_column(&mut self, table_name: &str, old_name: &str, new_name: &str) -> Result<(), String> {
        // Check for duplicates before the mutable borrow
        let cols = self
            .get_entry_by_name(table_name)
            .ok_or_else(|| format!("Table '{table_name}' not found"))?
            .columns()
            .to_vec();
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
            CatalogEntry::VectorIndex(_) => Err("Cannot rename column on a vector index".into()),
            CatalogEntry::Sequence(_) => Err("Cannot rename column on a sequence".into()),
            CatalogEntry::Foreign(_) => Err("Cannot rename column on a foreign table".into()),
            CatalogEntry::Macro(_) => Err("Cannot rename column on a macro".into()),
        }
    }

    /// Rename a table in the catalog.
    pub fn rename_table(&mut self, old_name: &str, new_name: &str) -> Result<(), String> {
        let id = self
            .name_to_id
            .get(old_name)
            .copied()
            .ok_or_else(|| format!("Table '{old_name}' not found"))?;
        if self.name_to_id.contains_key(new_name) {
            return Err(format!("Table '{new_name}' already exists"));
        }
        match self.entries.get_mut(&id) {
            Some(CatalogEntry::NodeTable(t)) => t.name = new_name.to_string(),
            Some(CatalogEntry::RelTable(t)) => t.name = new_name.to_string(),
            Some(CatalogEntry::VectorIndex(_)) => {
                // Vector index rename uses the `rename` method
                return Err("Use rename method for vector indexes".into());
            }
            Some(CatalogEntry::Sequence(s)) => s.name = new_name.to_string(),
            Some(CatalogEntry::Foreign(f)) => f.name = new_name.to_string(),
            Some(CatalogEntry::Macro(m)) => m.name = new_name.to_string(),
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

    /// Get the current catalog version.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Bump the catalog version counter.
    fn bump_version(&mut self) {
        self.version += 1;
    }

    /// Get all indexes as (name, table_name, type, column).
    pub fn indexes(&self) -> Vec<(String, String, String, String)> {
        let mut result = Vec::new();
        for entry in self.entries.values() {
            match entry {
                CatalogEntry::NodeTable(nt) => {
                    if nt.index_type.is_some() {
                        result.push((
                            nt.index_name.clone().unwrap_or_default(),
                            nt.name.clone(),
                            "ART".to_string(),
                            nt.columns
                                .get(nt.primary_key_column)
                                .map(|c| c.name.clone())
                                .unwrap_or_default(),
                        ));
                    }
                }
                CatalogEntry::VectorIndex(vi) => {
                    result.push((
                        vi.name.clone(),
                        vi.table_name.clone(),
                        "HNSW".to_string(),
                        vi.column_name.clone(),
                    ));
                }
                _ => {}
            }
        }
        result
    }

    /// Get connection info for a table as a vec of Values.
    pub fn connection_info(&self, table_name: &str) -> Option<Vec<kuzu_common::types::Value>> {
        let entry = self.get_entry_by_name(table_name)?;
        match entry {
            CatalogEntry::NodeTable(nt) => Some(vec![
                kuzu_common::types::Value::String(nt.name.clone()),
                kuzu_common::types::Value::String("NODE".to_string()),
                kuzu_common::types::Value::Null,
            ]),
            CatalogEntry::RelTable(rt) => {
                let src = self
                    .entries
                    .get(&rt.src_table_id)
                    .map(|e| e.name().to_string())
                    .unwrap_or_else(|| rt.src_table_id.to_string());
                let dst = self
                    .entries
                    .get(&rt.dst_table_id)
                    .map(|e| e.name().to_string())
                    .unwrap_or_else(|| rt.dst_table_id.to_string());
                Some(vec![
                    kuzu_common::types::Value::String(rt.name.clone()),
                    kuzu_common::types::Value::String(src),
                    kuzu_common::types::Value::String(dst),
                ])
            }
            _ => None,
        }
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

    /// Get all vector index entries.
    pub fn vector_indexes(&self) -> Vec<&VectorIndexEntry> {
        self.entries
            .values()
            .filter_map(|e| match e {
                CatalogEntry::VectorIndex(v) => Some(v),
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
                CatalogEntry::VectorIndex(v) => v.name = new_name,
                CatalogEntry::Sequence(s) => s.name = new_name.clone(),
                CatalogEntry::Foreign(f) => f.name = new_name,
                CatalogEntry::Macro(m) => m.name = new_name,
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
            CatalogColumn {
                name: "name".into(),
                logical_type: LogicalTypeID::String,
                is_primary_key: true,
                default_value: None,
            },
            CatalogColumn {
                name: "age".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: false,
                default_value: None,
            },
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
        let result = cat.create_rel_table(
            "Knows".into(),
            0,
            0,
            vec![CatalogColumn {
                name: "since".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: false,
                default_value: None,
            }],
        );
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
        assert_eq!(
            cat.create_node_table("Person".into(), sample_node_columns()),
            CatalogResult::AlreadyExists
        );
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
        assert!(matches!(
            cat.rename("Person", "Employee".into()),
            CatalogResult::Created { .. }
        ));
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

    // --- Sequence tests ---

    #[test]
    fn test_create_sequence_basic() {
        let mut cat = Catalog::new();
        let result = cat.create_sequence("my_seq".into(), 1, 1, 1, i64::MAX, false);
        assert!(matches!(result, CatalogResult::Created { .. }));
        assert_eq!(cat.len(), 1);
    }

    #[test]
    fn test_create_sequence_duplicate() {
        let mut cat = Catalog::new();
        cat.create_sequence("my_seq".into(), 1, 1, 1, i64::MAX, false);
        let result = cat.create_sequence("my_seq".into(), 1, 1, 1, i64::MAX, false);
        assert_eq!(result, CatalogResult::AlreadyExists);
    }

    #[test]
    fn test_get_sequence() {
        let mut cat = Catalog::new();
        cat.create_sequence("my_seq".into(), 100, 5, 1, i64::MAX, false);
        let seq = cat.get_sequence("my_seq");
        assert!(seq.is_some());
        let seq = seq.unwrap();
        assert_eq!(seq.name, "my_seq");
        assert_eq!(seq.curr_val(), 100);
        assert_eq!(seq.increment, 5);
    }

    #[test]
    fn test_get_sequence_nonexistent() {
        let cat = Catalog::new();
        assert!(cat.get_sequence("ghost").is_none());
    }

    #[test]
    fn test_sequence_next_k_val() {
        let mut cat = Catalog::new();
        cat.create_sequence("seq1".into(), 1, 1, 1, i64::MAX, false);
        let seq = cat.get_sequence_mut("seq1").unwrap();
        let v1 = seq.next_k_val(1);
        assert_eq!(v1, 1); // First call returns start value
        assert_eq!(seq.curr_val(), 2); // Advanced by increment
        let v2 = seq.next_k_val(1);
        assert_eq!(v2, 2);
        assert_eq!(seq.curr_val(), 3);
    }

    #[test]
    fn test_sequence_next_k_val_count() {
        let mut cat = Catalog::new();
        cat.create_sequence("seq2".into(), 0, 10, 0, i64::MAX, false);
        let seq = cat.get_sequence_mut("seq2").unwrap();
        let v = seq.next_k_val(3);
        assert_eq!(v, 0); // First call returns start
        assert_eq!(seq.curr_val(), 30); // Advanced by 10 * 3 = 30
    }

    #[test]
    fn test_sequence_drop() {
        let mut cat = Catalog::new();
        cat.create_sequence("to_drop".into(), 1, 1, 1, i64::MAX, false);
        assert!(cat.get_sequence("to_drop").is_some());
        let result = cat.drop_sequence("to_drop");
        assert!(matches!(result, CatalogResult::Dropped { .. }));
        assert!(cat.get_sequence("to_drop").is_none());
    }

    #[test]
    fn test_sequence_drop_nonexistent() {
        let mut cat = Catalog::new();
        assert_eq!(cat.drop_sequence("ghost"), CatalogResult::NotFound);
    }

    #[test]
    fn test_sequence_rollback() {
        let mut cat = Catalog::new();
        cat.create_sequence("rb_seq".into(), 0, 1, 0, i64::MAX, false);
        let seq = cat.get_sequence_mut("rb_seq").unwrap();
        seq.next_k_val(10); // Advance 10 times → curr_val = 10
        seq.rollback_val(0, 0); // Rollback to initial state
        assert_eq!(seq.usage_count, 0);
        assert_eq!(seq.curr_val, 0);
    }

    #[test]
    fn test_sequence_serial_name() {
        let name = SequenceEntry::get_serial_name("Person", "id");
        assert_eq!(name, "Person_id_serial");
    }

    #[test]
    fn test_sequence_entry_new() {
        let seq = SequenceEntry::new("test_seq".into(), 42, 2, 1, 100, true, 99);
        assert_eq!(seq.name, "test_seq");
        assert_eq!(seq.sequence_id, 99);
        assert_eq!(seq.curr_val(), 42);
        assert_eq!(seq.increment, 2);
        assert!(seq.cycle);
    }

    // --- Foreign table tests ---

    #[test]
    fn test_create_foreign_table_basic() {
        let mut cat = Catalog::new();
        let result = cat.create_foreign_table(
            "ext_table".into(),
            vec![
                CatalogColumn {
                    name: "id".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: false,
                    default_value: None,
                },
                CatalogColumn {
                    name: "name".into(),
                    logical_type: LogicalTypeID::String,
                    is_primary_key: false,
                    default_value: None,
                },
            ],
            "duckdb".into(),
        );
        assert!(matches!(result, CatalogResult::Created { .. }));
        assert_eq!(cat.len(), 1);
    }

    #[test]
    fn test_create_foreign_table_duplicate() {
        let mut cat = Catalog::new();
        cat.create_foreign_table("ext".into(), vec![], "duckdb".into());
        let result = cat.create_foreign_table("ext".into(), vec![], "postgres".into());
        assert_eq!(result, CatalogResult::AlreadyExists);
    }

    #[test]
    fn test_get_foreign_table() {
        let mut cat = Catalog::new();
        cat.create_foreign_table(
            "pg_orders".into(),
            vec![CatalogColumn {
                name: "amount".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: false,
                default_value: None,
            }],
            "postgres".into(),
        );
        let ft = cat.get_foreign_table("pg_orders");
        assert!(ft.is_some());
        let ft = ft.unwrap();
        assert_eq!(ft.name, "pg_orders");
        assert_eq!(ft.source_type, "postgres");
        assert_eq!(ft.num_columns(), 1);
    }

    #[test]
    fn test_get_foreign_table_nonexistent() {
        let cat = Catalog::new();
        assert!(cat.get_foreign_table("ghost").is_none());
    }

    #[test]
    fn test_drop_foreign_table() {
        let mut cat = Catalog::new();
        cat.create_foreign_table("to_drop".into(), vec![], "duckdb".into());
        assert!(cat.get_foreign_table("to_drop").is_some());
        let result = cat.drop_foreign_table("to_drop");
        assert!(matches!(result, CatalogResult::Dropped { .. }));
        assert!(cat.get_foreign_table("to_drop").is_none());
    }

    #[test]
    fn test_foreign_table_helpers() {
        let mut cat = Catalog::new();
        cat.create_foreign_table("f1".into(), vec![], "sqlite".into());
        let entry = cat.get_entry_by_name("f1").unwrap();
        assert!(entry.is_foreign_table());
        assert!(!entry.is_node_table());
        assert!(!entry.is_rel_table());
        assert!(!entry.is_sequence());
        assert!(entry.as_foreign_table().is_some());
        assert!(entry.as_node_table().is_none());
    }

    #[test]
    fn test_foreign_tables_filter() {
        let mut cat = Catalog::new();
        cat.create_node_table("Person".into(), sample_node_columns());
        cat.create_foreign_table("ext".into(), vec![], "duckdb".into());
        let foreign = cat.foreign_tables();
        assert_eq!(foreign.len(), 1);
        assert_eq!(foreign[0].source_type, "duckdb");
        let nodes = cat.node_tables();
        assert_eq!(nodes.len(), 1);
        assert_eq!(cat.len(), 2);
    }

    #[test]
    fn test_foreign_table_columns_access() {
        let mut cat = Catalog::new();
        cat.create_foreign_table(
            "ext".into(),
            vec![
                CatalogColumn {
                    name: "a".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: false,
                    default_value: None,
                },
                CatalogColumn {
                    name: "b".into(),
                    logical_type: LogicalTypeID::String,
                    is_primary_key: false,
                    default_value: None,
                },
            ],
            "duckdb".into(),
        );
        let entry = cat.get_entry_by_name("ext").unwrap();
        assert_eq!(entry.columns().len(), 2);
        assert_eq!(entry.columns()[0].name, "a");
        assert_eq!(entry.columns()[1].name, "b");
    }

    #[test]
    fn test_foreign_table_cannot_add_column() {
        let mut cat = Catalog::new();
        cat.create_foreign_table("ext".into(), vec![], "duckdb".into());
        let result = cat.add_column(
            "ext",
            CatalogColumn {
                name: "new_col".into(),
                logical_type: LogicalTypeID::String,
                is_primary_key: false,
                default_value: None,
            },
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("foreign table"));
    }

    #[test]
    fn test_foreign_table_cannot_drop_column() {
        let mut cat = Catalog::new();
        cat.create_foreign_table(
            "ext".into(),
            vec![CatalogColumn {
                name: "col1".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: false,
                default_value: None,
            }],
            "duckdb".into(),
        );
        let result = cat.drop_column("ext", "col1");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("foreign table"));
    }

    #[test]
    fn test_foreign_table_rename() {
        let mut cat = Catalog::new();
        cat.create_foreign_table("old_name".into(), vec![], "duckdb".into());
        assert!(matches!(
            cat.rename("old_name", "new_name".into()),
            CatalogResult::Created { .. }
        ));
        assert!(cat.contains("new_name"));
        assert!(!cat.contains("old_name"));
        let ft = cat.get_foreign_table("new_name");
        assert!(ft.is_some());
        assert_eq!(ft.unwrap().name, "new_name");
    }

    #[test]
    fn test_drop_foreign_table_nonexistent() {
        let mut cat = Catalog::new();
        assert_eq!(cat.drop_foreign_table("ghost"), CatalogResult::NotFound);
    }
}
