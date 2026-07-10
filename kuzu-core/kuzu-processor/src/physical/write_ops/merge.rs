//! Physical operator for MERGE.

use kuzu_common::types::{PhysicalTypeID, Value};
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_storage::table::TableCatalog;
use std::sync::Arc;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::physical::write_ops::set::{PhysicalSet, evaluate_expression_for_row};

/// Physical operator for MERGE.
/// Represents a combination of MATCH and INSERT (Upsert).
pub struct PhysicalMerge {
    pub table_name: String,
    pub table_id: u64,
    pub properties: Vec<(String, kuzu_parser::ast::Expression)>,
    pub on_match: Vec<PhysicalSet>,
    pub on_create: Vec<PhysicalSet>,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalMerge {
    fn operator_type(&self) -> &str {
        "merge"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let mut merged_count = 0u64;

        // Evaluate constant helper
        let eval_const = |expr: &kuzu_parser::ast::Expression| -> Value {
            match expr {
                kuzu_parser::ast::Expression::Constant(c) => match c {
                    kuzu_parser::ast::Constant::Null => Value::Null,
                    kuzu_parser::ast::Constant::Bool(b) => Value::Bool(*b),
                    kuzu_parser::ast::Constant::Integer(i) => Value::Int64(*i),
                    kuzu_parser::ast::Constant::Float(f) => Value::Double(*f),
                    kuzu_parser::ast::Constant::String(s) => Value::String(s.clone()),
                },
                _ => Value::Null,
            }
        };

        // Get table info to build the row
        let num_cols = {
            let tbl = self.table_catalog
                .get_node_table_by_name(&self.table_name)
                .ok_or_else(|| format!("Table '{}' not found for MERGE", self.table_name))?;
            tbl.columns.len()
        };

        // Build values from properties
        let mut new_values: Vec<Value> = Vec::new();
        let table_info = self.table_catalog
            .get_node_table_by_name(&self.table_name)
            .ok_or_else(|| format!("Table '{}' not found", self.table_name))?;
        for col_idx in 0..num_cols {
            let col_name = &table_info.columns[col_idx].name;
            if let Some((_, expr)) = self.properties.iter().find(|(n, _)| n == col_name) {
                new_values.push(eval_const(expr));
            } else if table_info.columns[col_idx].is_primary_key {
                return Err(format!("MERGE requires primary key '{}'", col_name));
            } else {
                new_values.push(Value::Null);
            }
        }
        drop(table_info);

        // Simple match detection: scan the PK column for a match
        let mut matched = false;
        if let Some(tbl) = self.table_catalog.get_node_table_by_name(&self.table_name) {
            if let Some((prop_name, first_expr)) = self.properties.first() {
                let first_val = eval_const(first_expr);
                // Find which column index this property maps to
                if let Some(prop_col) = tbl.columns.iter().position(|c| &c.name == prop_name) {
                    let _ = prop_col; // Column index for matching
                    // Scan the column for matching values
                    for row_idx in 0..tbl.num_rows as usize {
                        if let Some(val) = tbl.get_value(row_idx, prop_col) {
                            if val == &first_val {
                                matched = true;
                                break;
                            }
                        }
                    }
                }
            }
        }

        if matched {
            // Apply ON MATCH SET
            for set_op in &self.on_match {
                let _ = set_op.execute(vec![])?;
            }
        } else {
            // CREATE new node
            if let Some(mut tbl) = self.table_catalog.get_node_table_by_name_mut(&self.table_name) {
                tbl.insert_row(new_values)
                    .map_err(|e| format!("MERGE CREATE failed: {e}"))?;
                merged_count += 1;
            }

            // Apply ON CREATE SET
            for set_op in &self.on_create {
                let _ = set_op.execute(vec![])?;
            }
        }

        tracing::info!("MERGE: processed 1 merge in '{}'", self.table_name);

        let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
        v.resize(1);
        v.set_i64(0, 1); // Returns 1 indicating success of the operation
        Ok(vec![DataChunk::new(vec![v])])
    }
}
