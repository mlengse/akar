//! Physical operator for MERGE.

use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::physical::write_ops::set::PhysicalSet;
use akar_common::types::{PhysicalTypeID, Value};
use akar_common::vector::{DataChunk, ValueVector};
use akar_storage::table::TableCatalog;
use akar_transaction::UndoRecord;
use std::sync::{Arc, Mutex};

/// Physical operator for MERGE.
/// Represents a combination of MATCH and INSERT (Upsert).
pub struct PhysicalMerge {
    pub table_name: String,
    pub table_id: u64,
    pub properties: Vec<(String, akar_parser::ast::Expression)>,
    pub on_match: Vec<PhysicalSet>,
    pub on_create: Vec<PhysicalSet>,
    pub table_catalog: Arc<TableCatalog>,
    /// Active transaction id (P52.18).
    pub txn_id: Option<u64>,
    /// Undo sink for rollback records (P52.18).
    pub undo_sink: Option<Arc<Mutex<Vec<UndoRecord>>>>,
}

impl PhysicalOperatorExec for PhysicalMerge {
    fn operator_type(&self) -> &str {
        "merge"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        let mut _merged_count: u64 = 0;

        // Evaluate constant helper (fallback if input is empty)
        let eval_const = |expr: &akar_parser::ast::Expression, chunk: Option<&DataChunk>, row: usize| -> Value {
            if let Some(c) = chunk {
                crate::physical::write_ops::set::evaluate_expression_for_row(expr, c, row)
            } else {
                match expr {
                    akar_parser::ast::Expression::Constant(c) => match c {
                        akar_parser::ast::Constant::Null => Value::Null,
                        akar_parser::ast::Constant::Bool(b) => Value::Bool(*b),
                        akar_parser::ast::Constant::Integer(i) => Value::Int64(*i),
                        akar_parser::ast::Constant::Float(f) => Value::Double(*f),
                        akar_parser::ast::Constant::String(s) => Value::String(s.clone()),
                    },
                    _ => Value::Null,
                }
            }
        };

        // Get table info to build the row
        let num_cols = {
            let tbl = self
                .table_catalog
                .get_node_table_by_name(&self.table_name)
                .ok_or_else(|| format!("Table '{}' not found for MERGE", self.table_name))?;
            tbl.columns.len()
        };

        // Handle input chunks for pipeline (or just 1 iteration if empty)
        let chunks = if _input.is_empty() {
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
            v.resize(1);
            v.set_i64(0, 0);
            let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&v).array;
            vec![DataChunk::new(vec![arr], vec![PhysicalTypeID::Int64])]
        } else {
            _input
        };

        for chunk in &chunks {
            for row in 0..chunk.size {
                let table_info = self
                    .table_catalog
                    .get_node_table_by_name(&self.table_name)
                    .ok_or_else(|| format!("Table '{}' not found", self.table_name))?;

                // Determine PK property
                let mut pk_val = None;
                for col_idx in 0..num_cols {
                    let col = &table_info.columns[col_idx];
                    if col.is_primary_key {
                        if let Some((_, expr)) = self.properties.iter().find(|(n, _)| n == &col.name) {
                            let val = eval_const(expr, Some(chunk), row);
                            pk_val = Some(val);
                        }
                    }
                }

                let is_node = true; // For now assuming node merge
                let mut matched = false;

                if is_node {
                    if let Some(val) = &pk_val {
                        let row_ids = table_info.lookup_by_pk_range(Some(val), true, Some(val), true, 1);
                        if !row_ids.is_empty() {
                            matched = true;
                        }
                    } else {
                        // Without PK, we cannot effectively match a node in O(1).
                        // Cypher MERGE on non-PK fields requires full table scan or secondary index.
                        // For this implementation, we fail if PK is not provided.
                        if let Some((prop_name, expr)) = self.properties.first() {
                            let first_val = eval_const(expr, Some(chunk), row);
                            if let Some(prop_col) = table_info.columns.iter().position(|c| &c.name == prop_name) {
                                // Fallback scan
                                for row_idx in 0..table_info.num_rows as usize {
                                    if let Some(val) = table_info.get_value(row_idx, prop_col) {
                                        if val == &first_val {
                                            matched = true;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // Rel merge fallback
                }
                drop(table_info);

                if matched {
                    for set_op in &self.on_match {
                        let single_chunk = DataChunk::new(chunk.fields.clone(), chunk.field_types.clone()); // pass the row
                        let _ = set_op.execute(vec![single_chunk])?;
                    }
                } else {
                    let table_info = self.table_catalog.get_node_table_by_name(&self.table_name).unwrap();
                    let mut new_values: Vec<Value> = Vec::new();
                    for col_idx in 0..num_cols {
                        let col_name = &table_info.columns[col_idx].name;
                        if let Some((_, expr)) = self.properties.iter().find(|(n, _)| n == col_name) {
                            new_values.push(eval_const(expr, Some(chunk), row));
                        } else if table_info.columns[col_idx].is_primary_key {
                            return Err(format!("MERGE CREATE requires primary key '{}'", col_name).into());
                        } else {
                            new_values.push(Value::Null);
                        }
                    }
                    drop(table_info);

                    if let Some(mut tbl) = self.table_catalog.get_node_table_by_name_mut(&self.table_name) {
                        let row_id = tbl
                            .insert_row_with_txn(new_values, self.txn_id)
                            .map_err(|e| format!("MERGE CREATE failed: {e}"))?;
                        if let Some(sink) = self.undo_sink.as_ref()
                            && let Ok(mut u) = sink.lock()
                        {
                            u.push(UndoRecord::insert(self.table_id, row_id));
                        }
                        _merged_count += 1;
                    }

                    for set_op in &self.on_create {
                        let single_chunk = DataChunk::new(chunk.fields.clone(), chunk.field_types.clone());
                        let _ = set_op.execute(vec![single_chunk])?;
                    }
                }
            }
        }

        tracing::info!("MERGE: processed merges in '{}'", self.table_name);

        let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
        v.resize(1);
        v.set_i64(0, _merged_count as i64);
        let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&v).array;
        Ok(vec![DataChunk::new(vec![arr], vec![PhysicalTypeID::Int64])])
    }
}
