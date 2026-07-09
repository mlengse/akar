//! Auto-extracted from physical_operator.rs
use kuzu_common::types::{PhysicalTypeID, Value};
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_storage::table::ColumnDefinition;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};

// ==================== ScanRel ====================

/// Physical scan operator for relationship tables.
///
/// Reads data from a relationship table, with direction metadata.
/// Currently delegates to the same data resolution as PhysicalScan
/// but carries direction info for future optimization (e.g.,
/// adjacency-aware scanning, direction-based filtering).
#[derive(Debug, Clone)]
pub struct PhysicalScanRel {
    pub table_name: String,
    pub table_id: u64,
    pub direction: kuzu_parser::ast::EdgeDirection,
    pub table_data: Option<Vec<Vec<Value>>>,
    pub table_columns: Vec<ColumnDefinition>,
}

impl PhysicalOperatorExec for PhysicalScanRel {
    fn operator_type(&self) -> &str {
        "scan_rel"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        if let Some(ref data) = self.table_data {
            let num_cols = data.len();
            let num_rows = data.first().map(|c| c.len()).unwrap_or(0);
            if num_rows == 0 || num_cols == 0 {
                return Ok(vec![DataChunk::new(vec![])]);
            }
            let mut fields: Vec<ValueVector> = Vec::with_capacity(num_cols);
            for col in 0..num_cols {
                // Default to Int64; column definitions provide accurate types when available
                let phys_type = self
                    .table_columns
                    .get(col)
                    .map(|c| match c.logical_type {
                        kuzu_common::types::LogicalTypeID::Int64 => PhysicalTypeID::Int64,
                        kuzu_common::types::LogicalTypeID::Int32 => PhysicalTypeID::Int32,
                        kuzu_common::types::LogicalTypeID::Double => PhysicalTypeID::Double,
                        kuzu_common::types::LogicalTypeID::String => PhysicalTypeID::String,
                        kuzu_common::types::LogicalTypeID::Bool => PhysicalTypeID::Bool,
                        kuzu_common::types::LogicalTypeID::Float => PhysicalTypeID::Float,
                        _ => PhysicalTypeID::Int64,
                    })
                    .unwrap_or(PhysicalTypeID::Int64);
                let mut v = ValueVector::new(phys_type, num_rows.max(1));
                for row in 0..num_rows {
                    if let Some(val) = data[col].get(row) {
                        let _ = v.set_value(row, val);
                    }
                }
                v.resize(num_rows);
                fields.push(v);
            }
            let names: Vec<String> = (0..num_cols)
                .filter_map(|ci| self.table_columns.get(ci).map(|c| c.name.clone()))
                .collect();
            Ok(vec![DataChunk {
                fields,
                size: num_rows,
                field_names: names,
            }])
        } else {
            Ok(vec![DataChunk::new(vec![])])
        }
    }
}


