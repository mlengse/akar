//! PhysicalIndexLookup — point lookup via ART index.
//!
//! Uses the ART index on a table's primary key column to efficiently
//! find a single row. Produces a DataChunk with the matching row's
//! column values or empty result if the key is not found.

use crate::physical::common::store_value_in_vector;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use akar_common::types::Value;
use akar_common::vector::{DataChunk, ValueVector};
use akar_storage::table::TableCatalog;
use std::sync::Arc;

/// Physical operator for index-based point lookup.
///
/// Uses the ART index on a node table's primary key to find a single
/// row matching `key_value`. Returns the full row data or empty if
/// no match is found.
pub struct PhysicalIndexLookup {
    pub table_name: String,
    pub table_id: u64,
    /// The key value to look up in the ART index.
    pub key_value: Value,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalIndexLookup {
    fn operator_type(&self) -> &str {
        "index_lookup"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        let node_table = self
            .table_catalog
            .get_node_table_by_name(&self.table_name)
            .ok_or_else(|| format!("Node table '{}' not found for IndexLookup", self.table_name))?;

        // Use ART index for point lookup
        let row_ids = node_table.lookup_by_pk_range(
            Some(&self.key_value),
            true, // lower_inclusive
            Some(&self.key_value),
            true, // upper_inclusive
            1,    // max results (point lookup = single key)
        );

        if row_ids.is_empty() {
            return Ok(vec![DataChunk::new(vec![], vec![])]);
        }

        let row_id = row_ids[0] as usize;
        let num_cols = node_table.columns.len();

        let mut fields = Vec::with_capacity(num_cols);
        let mut field_types = Vec::with_capacity(num_cols);
        let mut field_names = Vec::with_capacity(num_cols);

        for col_idx in 0..num_cols {
            let val = node_table.get_value(row_id, col_idx).cloned().unwrap_or(Value::Null);

            let phys_type = val.physical_type();
            let mut v = ValueVector::new(phys_type, 1);
            v.resize(1);
            if matches!(val, Value::Null) {
                v.set_null(0, true);
            } else {
                store_value_in_vector(&mut v, 0, &val);
            }
            fields.push(akar_common::arrow_vector::ArrowVector::from_legacy(&v).array);
            field_types.push(phys_type);
            field_names.push(node_table.columns[col_idx].name.clone());
        }

        drop(node_table);

        Ok(vec![DataChunk {
            fields,
            field_types,
            size: 1,
            field_names,
            sel_vector: None,
        }])
    }
}
