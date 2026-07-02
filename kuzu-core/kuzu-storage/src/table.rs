//! Table storage — columnar node/rel tables with NodeGroup-based storage.

use crate::art_index::ArtPrimaryKeyIndex;
use crate::art_key::ArtKey;
use crate::index::HashIndex;
use crate::node_group::NodeGroup;
use crate::vector_index::VectorIndexTable;
use dashmap::DashMap;
use kuzu_common::types::{LogicalTypeID, Value};
use kuzu_vector::hnsw::DistanceMetric;
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
///
/// Primary key uniqueness is enforced via an in-memory `HashIndex` that maps
/// PK values to row offsets. For persistent indexes, use `OnDiskHashIndex`
/// alongside the L1 cache (see `index.rs`).
///
/// Optionally, an `ArtPrimaryKeyIndex` can be attached for range-scan
/// support on the primary key column (see `create_art_index`).
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
    /// In-memory hash index for primary key → row lookup and dedup.
    /// Stores the PK value (as a string representation) mapped to row offset.
    pub hash_index: HashIndex<String>,
    /// Optional ART (Adaptive Radix Tree) index for PK range scans.
    /// When present, `insert_row()` also updates this index automatically.
    pub art_index: Option<ArtPrimaryKeyIndex>,
}

impl NodeTable {
    pub fn new(table_id: u64, name: String, columns: Vec<ColumnDefinition>) -> Self {
        let primary_key_column = columns.iter().position(|c| c.is_primary_key).unwrap_or(0);
        Self {
            table_id,
            name,
            columns,
            primary_key_column,
            num_rows: 0,
            node_groups: Vec::new(),
            hash_index: HashIndex::new(),
            art_index: None,
        }
    }

    /// Insert a row of values into the table.
    ///
    /// Appends to the current `NodeGroup`; auto-creates a new group when the
    /// current one is full (reaches `NODE_GROUP_SIZE` rows).
    ///
    /// If the table has a primary key column, checks for duplicates and rejects
    /// rows with already-existing PK values. The hash index is updated after
    /// a successful insert.
    ///
    /// Returns an error if the number of values doesn't match the number of columns,
    /// or if a duplicate primary key value is detected.
    pub fn insert_row(&mut self, values: Vec<Value>) -> Result<(), String> {
        if values.len() != self.columns.len() {
            return Err(format!(
                "Column count mismatch: expected {} values, got {}",
                self.columns.len(),
                values.len()
            ));
        }

        // Check primary key uniqueness
        if self.primary_key_column < self.columns.len() {
            let pk_value = &values[self.primary_key_column];
            let pk_key = pk_value_to_string(pk_value);
            if self.hash_index.lookup(&pk_key).is_some() {
                return Err(format!(
                    "Duplicate primary key value: '{pk_key}' in table '{}'",
                    self.name
                ));
            }
        }

        // Get or create the current node group.
        let num_cols = self.columns.len();
        if self.node_groups.is_empty() || self.node_groups.last().unwrap().is_full() {
            let start_offset = self.num_rows;
            self.node_groups.push(NodeGroup::new(num_cols, start_offset));
        }

        let current = self.node_groups.last_mut().unwrap();
        current.append_row(values.clone())?;
        self.num_rows += 1;

        // Update hash index with the PK value for this row
        if self.primary_key_column < self.columns.len() {
            let pk_value = &values[self.primary_key_column];
            let pk_key = pk_value_to_string(pk_value);
            self.hash_index.insert(pk_key, self.num_rows - 1);

            // Also update ART index if present
            if let Some(ref mut art_idx) = self.art_index
                && let Some(art_key) = ArtKey::from_value(pk_value) {
                    art_idx.insert(&art_key, self.num_rows - 1);
                }
        }

        Ok(())
    }

    /// Look up a row offset by its primary key value.
    ///
    /// Returns `Some(row_offset)` if the PK exists, or `None` if not found.
    /// Uses the in-memory hash index for O(1) lookup.
    pub fn lookup_by_pk(&self, pk_value: &Value) -> Option<u64> {
        let pk_key = pk_value_to_string(pk_value);
        self.hash_index.lookup(&pk_key)
    }

    /// Perform a range scan on the primary key column using the ART index.
    ///
    /// Returns up to `max_results` row offsets for keys within `[lower, upper]`
    /// (respecting inclusivity flags). Returns an empty vec if no ART index
    /// exists or no keys match.
    ///
    /// This is the bridge function called by `PhysicalArtIndexRangeScan`.
    pub fn lookup_by_pk_range(
        &self,
        lower: Option<&Value>,
        lower_inclusive: bool,
        upper: Option<&Value>,
        upper_inclusive: bool,
        max_results: u64,
    ) -> Vec<u64> {
        match &self.art_index {
            Some(idx) => {
                let lower_key = lower.and_then(ArtKey::from_value);
                let upper_key = upper.and_then(ArtKey::from_value);
                idx.range_scan(
                    lower_key.as_ref(),
                    lower_inclusive,
                    upper_key.as_ref(),
                    upper_inclusive,
                    max_results,
                )
            }
            None => Vec::new(),
        }
    }

    /// Scan all values for a given column across all node groups.
    ///
    /// Returns a flat `Vec<Value>` containing values from `start` to
    /// `start + count` (or fewer if the end of the table is reached).
    ///
    /// If `snapshot_ts` is `Some(...)`, performs MVCC snapshot isolation:
    /// rows inserted/deleted by transactions committed after `snapshot_ts`
    /// are excluded, and versioned updates are resolved.
    pub fn scan_column(
        &self,
        col_idx: usize,
        start: u64,
        count: u64,
        snapshot_ts: Option<u64>,
        commit_history: &[(u64, u64)],
    ) -> Vec<Value> {
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
                let val = group.get_value_with_snapshot(row, col_idx, snapshot_ts, commit_history);
                match val {
                    Some(v) => result.push(v.clone()),
                    None => result.push(Value::Null),
                }
            }
            remaining -= take as u64;
        }

        result
    }

    /// Update a single cell (row, column) with a new value.
    pub fn update_cell(&mut self, row_idx: u64, col_idx: usize, value: Value) -> Result<(), String> {
        if col_idx >= self.columns.len() {
            return Err(format!("Column index {col_idx} out of range"));
        }
        if row_idx >= self.num_rows {
            return Err(format!("Row index {row_idx} out of range (num_rows={})", self.num_rows));
        }

        let mut offset = 0u64;
        for group in &mut self.node_groups {
            if row_idx < offset + group.num_nodes {
                let local_row = (row_idx - offset) as usize;
                if let Some(col_chunk) = group.columns.get_mut(col_idx) {
                    col_chunk.set_value(local_row, value)?;
                }
                return Ok(());
            }
            offset += group.num_nodes;
        }
        Err(format!("Row index {row_idx} not found in any node group"))
    }

    /// Delete a row by its index. Marks the row as null by setting all its column
    /// values to `Value::Null`. This is a soft delete — the row slot remains.
    pub fn delete_row(&mut self, row_idx: u64) -> Result<(), String> {
        if row_idx >= self.num_rows {
            return Err(format!("Row index {row_idx} out of range (num_rows={})", self.num_rows));
        }

        // Locate the node group containing this row
        let mut offset = 0u64;
        for group in &mut self.node_groups {
            if row_idx < offset + group.num_nodes {
                let local_row = (row_idx - offset) as usize;
                // Set all columns to Null for this row
                for col_chunk in &mut group.columns {
                    let _ = col_chunk.set_value(local_row, Value::Null);
                }
                return Ok(());
            }
            offset += group.num_nodes;
        }
        Err(format!("Row index {row_idx} not found in any node group"))
    }

    /// Get a single value at (row, col) by locating the correct `NodeGroup`
    /// and `ColumnChunk`.
    pub fn get_value(&self, row: usize, col: usize) -> Option<&Value> {
        self.get_value_with_snapshot(row, col, None, &[])
    }

    /// Get a single value with MVCC snapshot isolation.
    ///
    /// Checks `VersionInfo` for insert/delete visibility and `UpdateInfo`
    /// version chains when `snapshot_ts` is provided.
    pub fn get_value_with_snapshot(
        &self,
        row: usize,
        col: usize,
        snapshot_ts: Option<u64>,
        commit_history: &[(u64, u64)],
    ) -> Option<&Value> {
        if col >= self.columns.len() || row as u64 >= self.num_rows {
            return None;
        }
        let group_idx = self.find_group(row as u64);
        let group = self.node_groups.get(group_idx)?;
        let local_row = row as u64 - group.start_offset;
        group.get_value_with_snapshot(local_row as usize, col, snapshot_ts, commit_history)
    }

    /// Reconstruct column-major data (`Vec<Vec<Value>>`) from all node groups.
    ///
    /// Used by the processor (`resolve_scan_data`) for backward compatibility.
    pub fn to_column_major_data(&self) -> Vec<Vec<Value>> {
        self.to_column_major_data_with_predicate(None)
    }

    /// Like `to_column_major_data`, but applies an optional zone map predicate
    /// `(col_idx, op_string, val)` to skip entire node groups.
    pub fn to_column_major_data_with_predicate(
        &self,
        predicate: Option<(usize, &str, &Value)>,
    ) -> Vec<Vec<Value>> {
        let num_cols = self.columns.len();
        let mut result = vec![Vec::new(); num_cols]; // Avoid allocating self.num_rows if we skip chunks

        for group in &self.node_groups {
            if let Some((col_idx, op, val)) = predicate
                && let Some(col_chunk) = group.columns.get(col_idx) {
                    use crate::predicate::{check_zone_map, ZoneMapCheckResult};
                    if check_zone_map(&col_chunk.stats, op, val) == ZoneMapCheckResult::SkipScan {
                        continue; // Skip this entire node group
                    }
                }

            for row in 0..group.num_nodes as usize {
                for (col, res_col) in result.iter_mut().enumerate().take(num_cols) {
                    match group.get_value(row, col) {
                        Some(v) => res_col.push(v.clone()),
                        None => res_col.push(Value::Null),
                    }
                }
            }
        }

        result
    }

    /// Binary-search for the node group that contains `row`.
    fn find_group(&self, row: u64) -> usize {
        match self.node_groups.binary_search_by_key(&row, |g| g.start_offset) {
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

/// Convert a Value to its string representation for use as a hash index key.
fn pk_value_to_string(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int64(i) => i.to_string(),
        Value::Int32(i) => i.to_string(),
        Value::Int16(i) => i.to_string(),
        Value::Int8(i) => i.to_string(),
        Value::UInt64(u) => u.to_string(),
        Value::UInt32(u) => u.to_string(),
        Value::UInt16(u) => u.to_string(),
        Value::UInt8(u) => u.to_string(),
        Value::Double(f) => f.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => s.clone(),
        Value::Date(d) => format!("Date({})", d.0),
        Value::Timestamp(ts) => format!("Timestamp({})", ts.0),
        other => format!("{other:?}"),
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
    pub fn insert_rel(&mut self, from: u64, to: u64, values: Vec<Value>) -> Result<(), String> {
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
        self.fwd_adj.entry(from).or_default().push((to, edge_idx));

        // Update reverse adjacency.
        self.rev_adj.entry(to).or_default().push((from, edge_idx));

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
        self.fwd_adj.get(&src_offset).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Scan the reverse adjacency list for a given destination node.
    ///
    /// Returns a list of `(src_offset, edge_idx)` pairs, or an empty vec
    /// if the node has no incoming edges.
    pub fn scan_rev_adj_list(&self, dst_offset: u64) -> &[(u64, usize)] {
        self.rev_adj.get(&dst_offset).map(|v| v.as_slice()).unwrap_or(&[])
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
///
/// Uses `DashMap` internally for lock-free concurrent reads.
/// Write operations synchronize on individual entries rather than
/// the entire catalog, allowing concurrent writers to different
/// tables to proceed in parallel.
#[derive(Debug, Default)]
pub struct TableCatalog {
    node_tables: DashMap<u64, NodeTable>,
    rel_tables: DashMap<u64, RelTable>,
    vector_indexes: DashMap<u64, VectorIndexTable>,
    /// Map from table name to table ID for node tables.
    node_name_to_id: DashMap<String, u64>,
    /// Map from table name to table ID for rel tables.
    rel_name_to_id: DashMap<String, u64>,
    /// Map from index name to index ID for vector indexes.
    vector_index_name_to_id: DashMap<String, u64>,
    next_table_id: std::sync::atomic::AtomicU64,
}

impl TableCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_node_table(&self, name: String, columns: Vec<ColumnDefinition>) -> NodeTable {
        let table_id = self.next_table_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let table = NodeTable::new(table_id, name.clone(), columns);
        self.node_name_to_id.insert(name, table_id);
        self.node_tables.insert(table_id, table.clone());
        table
    }

    pub fn create_rel_table(
        &self,
        name: String,
        src_table_id: u64,
        dst_table_id: u64,
        columns: Vec<ColumnDefinition>,
    ) -> RelTable {
        let table_id = self.next_table_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let table = RelTable::new(table_id, name.clone(), src_table_id, dst_table_id, columns);
        self.rel_name_to_id.insert(name, table_id);
        self.rel_tables.insert(table_id, table.clone());
        table
    }

    pub fn get_node_table(&self, table_id: u64) -> Option<dashmap::mapref::one::Ref<'_, u64, NodeTable>> {
        self.node_tables.get(&table_id)
    }

    pub fn get_node_table_mut(&self, table_id: u64) -> Option<dashmap::mapref::one::RefMut<'_, u64, NodeTable>> {
        self.node_tables.get_mut(&table_id)
    }

    pub fn get_node_table_by_name(&self, name: &str) -> Option<dashmap::mapref::one::Ref<'_, u64, NodeTable>> {
        let id = self.node_name_to_id.get(name)?;
        self.node_tables.get(&*id)
    }

    pub fn get_node_table_by_name_mut(&self, name: &str) -> Option<dashmap::mapref::one::RefMut<'_, u64, NodeTable>> {
        let id = self.node_name_to_id.get(name)?;
        self.node_tables.get_mut(&*id)
    }

    pub fn get_rel_table(&self, table_id: u64) -> Option<dashmap::mapref::one::Ref<'_, u64, RelTable>> {
        self.rel_tables.get(&table_id)
    }

    pub fn get_rel_table_mut(&self, table_id: u64) -> Option<dashmap::mapref::one::RefMut<'_, u64, RelTable>> {
        self.rel_tables.get_mut(&table_id)
    }

    pub fn get_rel_table_by_name(&self, name: &str) -> Option<dashmap::mapref::one::Ref<'_, u64, RelTable>> {
        let id = self.rel_name_to_id.get(name)?;
        self.rel_tables.get(&*id)
    }

    pub fn get_rel_table_by_name_mut(&self, name: &str) -> Option<dashmap::mapref::one::RefMut<'_, u64, RelTable>> {
        let id = self.rel_name_to_id.get(name)?;
        self.rel_tables.get_mut(&*id)
    }

    pub fn all_node_tables(
        &self,
    ) -> Vec<dashmap::mapref::multiple::RefMulti<'_, u64, NodeTable>> {
        self.node_tables.iter().collect()
    }

    pub fn all_rel_tables(
        &self,
    ) -> Vec<dashmap::mapref::multiple::RefMulti<'_, u64, RelTable>> {
        self.rel_tables.iter().collect()
    }

    /// Get the number of rows in a node table by name.
    pub fn node_table_num_rows(&self, name: &str) -> u64 {
        self.get_node_table_by_name(name)
            .map(|t| t.num_rows)
            .unwrap_or(0)
    }

    /// Remove a node table by name. Returns true if the table existed.
    pub fn drop_node_table(&self, name: &str) -> bool {
        if let Some(id) = self.node_name_to_id.get(name) {
            let table_id = *id;
            drop(id);
            self.node_name_to_id.remove(name);
            self.node_tables.remove(&table_id).is_some()
        } else {
            false
        }
    }

    /// Create a new vector index in the catalog.
    pub fn create_vector_index(
        &self,
        name: String,
        table_name: String,
        column_name: String,
        metric: DistanceMetric,
        dimensions: u32,
    ) -> VectorIndexTable {
        let index_id = self.next_table_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let table = VectorIndexTable::new(index_id, name.clone(), table_name, column_name, metric, dimensions);
        self.vector_index_name_to_id.insert(name, index_id);
        self.vector_indexes.insert(index_id, table.clone());
        table
    }

    /// Get a vector index by its ID.
    pub fn get_vector_index(&self, index_id: u64) -> Option<dashmap::mapref::one::Ref<'_, u64, VectorIndexTable>> {
        self.vector_indexes.get(&index_id)
    }

    /// Get a vector index by name.
    pub fn get_vector_index_by_name(&self, name: &str) -> Option<dashmap::mapref::one::Ref<'_, u64, VectorIndexTable>> {
        let id = self.vector_index_name_to_id.get(name)?;
        self.vector_indexes.get(&*id)
    }

    /// Get a mutable vector index by name.
    pub fn get_vector_index_by_name_mut(&self, name: &str) -> Option<dashmap::mapref::one::RefMut<'_, u64, VectorIndexTable>> {
        let id = self.vector_index_name_to_id.get(name)?;
        self.vector_indexes.get_mut(&*id)
    }

    /// Get a mutable vector index by ID.
    pub fn get_vector_index_mut(&self, index_id: u64) -> Option<dashmap::mapref::one::RefMut<'_, u64, VectorIndexTable>> {
        self.vector_indexes.get_mut(&index_id)
    }

    /// Remove a vector index by name. Returns true if the index existed.
    pub fn drop_vector_index(&self, name: &str) -> bool {
        if let Some(id) = self.vector_index_name_to_id.get(name) {
            let index_id = *id;
            drop(id);
            self.vector_index_name_to_id.remove(name);
            self.vector_indexes.remove(&index_id).is_some()
        } else {
            false
        }
    }

    /// Get all vector indexes.
    pub fn all_vector_indexes(&self) -> Vec<dashmap::mapref::multiple::RefMulti<'_, u64, VectorIndexTable>> {
        self.vector_indexes.iter().collect()
    }

    /// Create an ART (Adaptive Radix Tree) index on a node table's PK column.
    ///
    /// Creates a new `ArtPrimaryKeyIndex`, backfills it with all existing rows,
    /// and attaches it to the `NodeTable`.
    ///
    /// The `index_name` is used as the BufferManager file name for persistence.
    pub fn create_art_index(&self, table_name: &str, index_name: &str) -> Result<(), String> {
        let mut table = self
            .get_node_table_by_name_mut(table_name)
            .ok_or_else(|| format!("Node table '{table_name}' not found"))?;

        if table.art_index.is_some() {
            return Err(format!("Table '{table_name}' already has an ART index"));
        }

        let mut art_idx = ArtPrimaryKeyIndex::new(index_name);

        // Backfill existing rows
        let pk_col = table.primary_key_column;
        // Scan all rows via to_column_major_data for backfill
        let col_major = table.to_column_major_data();
        if pk_col < col_major.len() {
            for (row_offset, pk_val) in col_major[pk_col].iter().enumerate() {
                if !matches!(pk_val, Value::Null)
                    && let Some(art_key) = ArtKey::from_value(pk_val) {
                        art_idx.insert(&art_key, row_offset as u64);
                    }
            }
        }

        table.art_index = Some(art_idx);
        Ok(())
    }

    /// Drop the ART index from a node table.
    pub fn drop_art_index(&self, table_name: &str) -> Result<(), String> {
        let mut table = self
            .get_node_table_by_name_mut(table_name)
            .ok_or_else(|| format!("Node table '{table_name}' not found"))?;

        table.art_index = None;
        Ok(())
    }

    /// Get a reference to the ART index for a node table (via table name).
    /// Returns `None` if the table has no ART index or doesn't exist.
    pub fn get_art_index(&self, table_name: &str) -> Option<ArtPrimaryKeyIndex> {
        let table = self.get_node_table_by_name(table_name)?;
        table.art_index.clone()
    }

    /// Check if a node table has an ART index.
    pub fn has_art_index(&self, table_name: &str) -> bool {
        self.get_node_table_by_name(table_name)
            .map(|t| t.art_index.is_some())
            .unwrap_or(false)
    }

    /// Remove a rel table by name. Returns true if the table existed.
    pub fn drop_rel_table(&self, name: &str) -> bool {
        if let Some(id) = self.rel_name_to_id.get(name) {
            let table_id = *id;
            drop(id);
            self.rel_name_to_id.remove(name);
            self.rel_tables.remove(&table_id).is_some()
        } else {
            false
        }
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
        let table = NodeTable::new(
            1,
            "Person".into(),
            vec![
                ColumnDefinition {
                    name: "name".into(),
                    logical_type: LogicalTypeID::String,
                    is_primary_key: true,
                },
                ColumnDefinition {
                    name: "age".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: false,
                },
            ],
        );
        assert_eq!(table.num_rows, 0);
        assert!(table.node_groups.is_empty());
    }

    #[test]
    fn test_node_table_insert_and_get() {
        let mut table = NodeTable::new(
            1,
            "Person".into(),
            vec![
                ColumnDefinition {
                    name: "name".into(),
                    logical_type: LogicalTypeID::String,
                    is_primary_key: true,
                },
                ColumnDefinition {
                    name: "age".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: false,
                },
            ],
        );
        table
            .insert_row(vec![Value::String("Alice".into()), Value::Int64(30)])
            .unwrap();
        table
            .insert_row(vec![Value::String("Bob".into()), Value::Int64(25)])
            .unwrap();

        assert_eq!(table.num_rows, 2);
        assert_eq!(table.get_value(0, 0), Some(&Value::String("Alice".into())));
        assert_eq!(table.get_value(1, 1), Some(&Value::Int64(25)));
    }

    #[test]
    fn test_node_table_scan_column() {
        let mut table = NodeTable::new(
            1,
            "T".into(),
            vec![ColumnDefinition {
                name: "val".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: false,
            }],
        );
        for i in 0..100 {
            table.insert_row(vec![Value::Int64(i)]).unwrap();
        }
        let scanned = table.scan_column(0, 10, 5, None, &[]);
        assert_eq!(scanned.len(), 5);
        assert_eq!(scanned[0], Value::Int64(10));
        assert_eq!(scanned[4], Value::Int64(14));
    }

    #[test]
    fn test_node_table_to_column_major() {
        let mut table = NodeTable::new(
            1,
            "T".into(),
            vec![
                ColumnDefinition {
                    name: "x".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: false,
                },
                ColumnDefinition {
                    name: "y".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: false,
                },
            ],
        );
        table.insert_row(vec![Value::Int64(1), Value::Int64(10)]).unwrap();
        table.insert_row(vec![Value::Int64(2), Value::Int64(20)]).unwrap();

        let data = table.to_column_major_data();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0], vec![Value::Int64(1), Value::Int64(2)]);
        assert_eq!(data[1], vec![Value::Int64(10), Value::Int64(20)]);
    }

    #[test]
    fn test_node_table_auto_node_group() {
        let mut table = NodeTable::new(
            1,
            "T".into(),
            vec![ColumnDefinition {
                name: "v".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: false,
            }],
        );
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
        assert_eq!(
            table.get_value(NODE_GROUP_SIZE, 0),
            Some(&Value::Int64(NODE_GROUP_SIZE as i64))
        );
    }

    // ==================== RelTable (CSR) tests ====================

    fn make_rel_table() -> RelTable {
        RelTable::new(
            1,
            "Knows".into(),
            0,
            1,
            vec![
                ColumnDefinition {
                    name: "since".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: false,
                },
                ColumnDefinition {
                    name: "weight".into(),
                    logical_type: LogicalTypeID::Double,
                    is_primary_key: false,
                },
            ],
        )
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
        rel.insert_rel(0, 1, vec![Value::Int64(2020), Value::Double(0.5)])
            .unwrap();
        rel.insert_rel(0, 2, vec![Value::Int64(2021), Value::Double(0.8)])
            .unwrap();
        rel.insert_rel(1, 0, vec![Value::Int64(2020), Value::Double(0.3)])
            .unwrap();

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
        rel.insert_rel(0, 5, vec![Value::Int64(2022), Value::Double(1.0)])
            .unwrap();
        rel.insert_rel(3, 5, vec![Value::Int64(2023), Value::Double(1.5)])
            .unwrap();

        // Node 5 has two incoming edges
        let rev = rel.scan_rev_adj_list(5);
        assert_eq!(rev.len(), 2);
        assert_eq!(rev[0], (0, 0));
        assert_eq!(rev[1], (3, 1));
    }

    #[test]
    fn test_rel_get_edge_properties() {
        let mut rel = make_rel_table();
        rel.insert_rel(0, 1, vec![Value::Int64(2020), Value::Double(0.5)])
            .unwrap();
        rel.insert_rel(2, 3, vec![Value::Int64(2021), Value::Double(0.9)])
            .unwrap();

        let props0 = rel.get_edge_properties(0);
        assert_eq!(props0, vec![Value::Int64(2020), Value::Double(0.5)]);

        let props1 = rel.get_edge_properties(1);
        assert_eq!(props1, vec![Value::Int64(2021), Value::Double(0.9)]);
    }

    #[test]
    fn test_rel_get_outgoing_edges() {
        let mut rel = make_rel_table();
        rel.insert_rel(0, 10, vec![Value::Int64(2020), Value::Double(1.0)])
            .unwrap();
        rel.insert_rel(0, 20, vec![Value::Int64(2021), Value::Double(2.0)])
            .unwrap();

        let outgoing = rel.get_outgoing_edges(0);
        assert_eq!(outgoing.len(), 2);
        assert_eq!(outgoing[0].0, 10);
        assert_eq!(outgoing[0].1, vec![Value::Int64(2020), Value::Double(1.0)]);
        assert_eq!(outgoing[1].0, 20);
    }

    #[test]
    fn test_rel_get_incoming_edges() {
        let mut rel = make_rel_table();
        rel.insert_rel(10, 5, vec![Value::Int64(2020), Value::Double(1.0)])
            .unwrap();
        rel.insert_rel(20, 5, vec![Value::Int64(2021), Value::Double(2.0)])
            .unwrap();

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
        rel.insert_rel(0, 1, vec![Value::Int64(2020), Value::Double(1.5)])
            .unwrap();
        rel.insert_rel(1, 2, vec![Value::Int64(2021), Value::Double(2.5)])
            .unwrap();

        let since_col = rel.get_column(0).unwrap();
        assert_eq!(since_col, &[Value::Int64(2020), Value::Int64(2021)]);

        let weight_col = rel.get_column(1).unwrap();
        assert_eq!(weight_col, &[Value::Double(1.5), Value::Double(2.5)]);
    }

    #[test]
    fn test_rel_to_column_major() {
        let mut rel = make_rel_table();
        rel.insert_rel(0, 1, vec![Value::Int64(2020), Value::Double(0.5)])
            .unwrap();
        rel.insert_rel(2, 3, vec![Value::Int64(2021), Value::Double(0.9)])
            .unwrap();

        let data = rel.to_column_major_data();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0], vec![Value::Int64(2020), Value::Int64(2021)]);
        assert_eq!(data[1], vec![Value::Double(0.5), Value::Double(0.9)]);
    }

    // ==================== TableCatalog tests ====================

    #[test]
    fn test_catalog_create_and_lookup() {
        let cat = TableCatalog::new();
        let node_table = cat.create_node_table(
            "Person".into(),
            vec![ColumnDefinition {
                name: "id".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: true,
            }],
        );
        assert_eq!(node_table.table_id, 0);

        let rel_table = cat.create_rel_table(
            "Knows".into(),
            0,
            1,
            vec![ColumnDefinition {
                name: "since".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: false,
            }],
        );
        assert_eq!(rel_table.table_id, 1);

        assert!(cat.get_node_table(0).is_some());
        assert!(cat.get_rel_table(1).is_some());
        assert_eq!(cat.node_table_num_rows("Person"), 0);
    }
}
