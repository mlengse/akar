//! Auto-extracted from physical_operator.rs
use kuzu_common::types::PhysicalTypeID;
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_storage::table::TableCatalog;
use std::sync::Arc;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};

// ==================== Set ====================

/// Physical operator for SET — updates a property on matched rows.
pub struct PhysicalSet {
    pub table_name: String,
    pub table_id: u64,
    pub column_name: String,
    pub column_idx: usize,
    pub value: kuzu_parser::ast::Expression,
    pub is_node: bool,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalSet {
    fn operator_type(&self) -> &str {
        "set"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        // Collect row indices from input chunks (first column has row index)
        let mut rows_to_update: Vec<(u64, kuzu_common::types::Value)> = Vec::new();

        for chunk in &input {
            for row in 0..chunk.size {
                if !chunk.fields.is_empty()
                    && let Some(kuzu_common::types::Value::Int64(val)) = chunk.get_value(0, row)
                {
                    // Evaluate the SET value expression against the current row
                    let set_val = evaluate_expression_for_row(&self.value, chunk, row);
                    rows_to_update.push((val as u64, set_val));
                }
            }
        }

        if rows_to_update.is_empty() {
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
            v.resize(1);
            v.set_i64(0, 0);
            let arr = kuzu_common::arrow_vector::ArrowVector::from_legacy(&v).array;
            return Ok(vec![DataChunk::new(vec![arr], vec![PhysicalTypeID::Int64])]);
        }

        // Apply updates to the table
        let mut updated = 0u64;
        if self.is_node {
            if let Some(mut table) = self.table_catalog.get_node_table_by_name_mut(&self.table_name) {
                for (row_idx, val) in &rows_to_update {
                    if table.update_cell(*row_idx, self.column_idx, val.clone()).is_ok() {
                        updated += 1;
                    }
                }
            } else {
                return Err(format!("Node table '{}' not found for SET", self.table_name));
            }
        } else {
            if let Some(mut table) = self.table_catalog.get_rel_table_by_name_mut(&self.table_name) {
                for (edge_idx, val) in &rows_to_update {
                    if table
                        .update_cell(*edge_idx as usize, self.column_idx, val.clone())
                        .is_ok()
                    {
                        updated += 1;
                    }
                }
            } else {
                return Err(format!("Rel table '{}' not found for SET", self.table_name));
            }
        }

        tracing::info!("SET: updated {updated} rows in '{}'", self.table_name);

        let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
        v.resize(1);
        v.set_i64(0, updated as i64);
        let arr = kuzu_common::arrow_vector::ArrowVector::from_legacy(&v).array;
        Ok(vec![DataChunk::new(vec![arr], vec![PhysicalTypeID::Int64])])
    }
}

/// Simple expression evaluator for SET value expressions against a DataChunk row.
pub fn evaluate_expression_for_row(
    expr: &kuzu_parser::ast::Expression,
    chunk: &DataChunk,
    row: usize,
) -> kuzu_common::types::Value {
    match expr {
        kuzu_parser::ast::Expression::Constant(c) => match c {
            kuzu_parser::ast::Constant::Null => kuzu_common::types::Value::Null,
            kuzu_parser::ast::Constant::Bool(b) => kuzu_common::types::Value::Bool(*b),
            kuzu_parser::ast::Constant::Integer(i) => kuzu_common::types::Value::Int64(*i),
            kuzu_parser::ast::Constant::Float(f) => kuzu_common::types::Value::Double(*f),
            kuzu_parser::ast::Constant::String(s) => kuzu_common::types::Value::String(s.clone()),
        },
        _ => {
            // Fallback: try to get value from chunk fields
            if chunk.fields.len() > 1 {
                chunk.get_value(1, row).unwrap_or(kuzu_common::types::Value::Null)
            } else {
                kuzu_common::types::Value::Null
            }
        }
    }
}


