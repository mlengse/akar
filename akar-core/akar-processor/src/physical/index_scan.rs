//! PhysicalIndexScan — equality/set-membership lookup via a secondary index.
//!
//! Uses the hash secondary index on a non-primary-key column to efficiently
//! find matching row offsets. Produces a DataChunk with all matching rows
//! or empty result if no match is found.

use crate::physical::common::store_value_in_vector;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use akar_common::types::{PhysicalTypeID, Value, pk_value_to_string};
use akar_common::vector::{DataChunk, ValueVector};
use akar_storage::table::TableCatalog;
use std::sync::Arc;

/// Physical operator for index-based equality/set-membership lookup.
///
/// Uses the secondary index on a non-primary-key column to find rows matching
/// one or more key values. Returns the full row data for each match, or an
/// empty result if no match is found. Falls back gracefully (empty result)
/// when the index is not yet `Available`.
pub struct PhysicalIndexScan {
    pub table_name: String,
    pub table_id: u64,
    pub alias: Option<String>,
    pub column_name: String,
    pub index_name: String,
    pub values: Vec<Value>,
    pub table_catalog: Arc<TableCatalog>,
    pub storage_manager: Arc<akar_storage::StorageManager>,
}

impl PhysicalOperatorExec for PhysicalIndexScan {
    fn operator_type(&self) -> &str {
        "index_scan"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        // Get the secondary index from the table catalog.
        let index = self
            .table_catalog
            .get_secondary_index(&self.index_name)
            .ok_or_else(|| {
                format!(
                    "Secondary index '{}' not found for IndexScan on table '{}'",
                    self.index_name, self.table_name
                )
            })?;

        if !index.is_available() {
            // Index not yet available: treat as empty result (FR-004 fallback
            // would be a full scan, but the planner only routes here when an
            // index exists; a not-yet-available index yields no matches).
            return Ok(vec![DataChunk::new(vec![], vec![])]);
        }

        // Get the node table for reading full row data.
        let node_table = self
            .table_catalog
            .get_node_table_by_name(&self.table_name)
            .ok_or_else(|| format!("Node table '{}' not found for IndexScan", self.table_name))?;

        let num_cols = node_table.columns.len();

        // Collect all matching row IDs. The index is on a non-unique column, so
        // one key can match several rows (FR-002).
        let mut row_ids: Vec<u64> = Vec::new();
        let bm = self.storage_manager.buffer_manager();
        let mut bm_guard = bm.lock().map_err(|e| format!("buffer manager lock poisoned: {e}"))?;

        for value in &self.values {
            let key = pk_value_to_string(value);
            let offsets = index
                .lookup_all(&key, &mut bm_guard)
                .map_err(|e| format!("index lookup error: {e}"))?;
            row_ids.extend(offsets);
        }
        row_ids.sort_unstable();
        row_ids.dedup();

        drop(bm_guard);

        if row_ids.is_empty() {
            return Ok(vec![DataChunk::new(vec![], vec![])]);
        }

        let num_rows = row_ids.len();
        let mut fields = Vec::with_capacity(num_cols);
        let mut field_types = Vec::with_capacity(num_cols);
        let mut field_names = Vec::with_capacity(num_cols);

        for col_idx in 0..num_cols {
            // Collect all values for this column, determining physical type
            // from the first non-null value (like PhysicalIndexLookup does).
            let mut col_values: Vec<Value> = Vec::with_capacity(num_rows);
            let mut phys_type: Option<PhysicalTypeID> = None;

            for &row_id in &row_ids {
                let row_id_usize = row_id as usize;
                let val = node_table
                    .get_value(row_id_usize, col_idx)
                    .cloned()
                    .unwrap_or(Value::Null);
                if phys_type.is_none() && !matches!(val, Value::Null) {
                    phys_type = Some(val.physical_type());
                }
                col_values.push(val);
            }

            let phys_type = phys_type.unwrap_or(PhysicalTypeID::Int64);
            let mut v = ValueVector::new(phys_type, num_rows);
            v.resize(num_rows);

            for (row_idx, val) in col_values.into_iter().enumerate() {
                if matches!(val, Value::Null) {
                    v.set_null(row_idx, true);
                } else {
                    store_value_in_vector(&mut v, row_idx, &val)?;
                }
            }

            fields.push(akar_common::arrow_vector::ArrowVector::from_legacy(&v).array);
            field_types.push(phys_type);
            field_names.push(format!(
                "{}.{}",
                self.alias.as_deref().unwrap_or(&self.table_name),
                node_table.columns[col_idx].name
            ));
        }

        drop(node_table);

        Ok(vec![DataChunk {
            fields,
            field_types,
            size: num_rows,
            field_names,
            sel_vector: None,
        }])
    }
}
