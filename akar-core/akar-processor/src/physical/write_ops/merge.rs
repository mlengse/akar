//! Physical operator for MERGE.

use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::physical::write_ops::set::{PhysicalSet, append_pipeline_columns};
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

/// Build a single-column chunk carrying physical row indices under the `_id`
/// pseudo-column name, so a `PhysicalSet` passed as ON MATCH / ON CREATE can
/// re-target exactly those rows.
fn row_id_chunk(row_ids: &[u64]) -> DataChunk {
    let mut v = ValueVector::new(PhysicalTypeID::Int64, row_ids.len());
    v.resize(row_ids.len());
    for (i, r) in row_ids.iter().enumerate() {
        v.set_i64(i, *r as i64);
    }
    let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&v).array;
    DataChunk::new(vec![arr], vec![PhysicalTypeID::Int64]).with_names(vec!["_id".to_string()])
}

impl PhysicalOperatorExec for PhysicalMerge {
    fn operator_type(&self) -> &str {
        "merge"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        // A filtered-out pipeline stays empty (P53.25): nothing was merged.
        if !input.is_empty() && input.iter().all(|c| c.size == 0) {
            return Ok(vec![DataChunk::new(vec![], vec![])]);
        }

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
        let chunks = if input.is_empty() {
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
            v.resize(1);
            v.set_i64(0, 0);
            let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&v).array;
            vec![DataChunk::new(vec![arr], vec![PhysicalTypeID::Int64])]
        } else {
            input
        };

        // One output row per processed input row, carrying the physical row id
        // (matched or newly created) plus its source (chunk, row) so pipeline
        // columns (e.g. UNWIND variables) survive into the output (P53.31).
        let mut merged_row_ids: Vec<u64> = Vec::new();
        let mut source_rows: Vec<(usize, usize)> = Vec::new();
        let mut matched_ids: Vec<u64> = Vec::new();
        let mut created_ids: Vec<u64> = Vec::new();

        for (ci, chunk) in chunks.iter().enumerate() {
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

                // Match against the existing table. Without a PK property, fall
                // back to a full scan over the first pattern property (Cypher
                // MERGE on non-PK fields has no O(1) lookup without a secondary
                // index).
                let mut matched: Option<u64> = None;
                if let Some(val) = &pk_val {
                    let row_ids = table_info.lookup_by_pk_range(Some(val), true, Some(val), true, 1);
                    if !row_ids.is_empty() {
                        matched = Some(row_ids[0]);
                    }
                } else if let Some((prop_name, expr)) = self.properties.first() {
                    let first_val = eval_const(expr, Some(chunk), row);
                    if let Some(prop_col) = table_info.columns.iter().position(|c| &c.name == prop_name) {
                        for row_idx in 0..table_info.num_rows as usize {
                            if let Some(val) = table_info.get_value(row_idx, prop_col)
                                && val == &first_val
                            {
                                matched = Some(row_idx as u64);
                                break;
                            }
                        }
                    }
                }
                drop(table_info);

                if let Some(row_id) = matched {
                    matched_ids.push(row_id);
                    merged_row_ids.push(row_id);
                    source_rows.push((ci, row));
                } else {
                    let table_info = self
                        .table_catalog
                        .get_node_table_by_name(&self.table_name)
                        .ok_or_else(|| format!("Table '{}' not found", self.table_name))?;
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
                        created_ids.push(row_id);
                        merged_row_ids.push(row_id);
                        source_rows.push((ci, row));
                    }
                }
            }
        }

        // Apply ON MATCH / ON CREATE SET once per group, targeting the affected
        // row ids via the `_id` pseudo-column (mirrors the edge MERGE path).
        for set_op in &self.on_match {
            if !matched_ids.is_empty() {
                let chunk = row_id_chunk(&matched_ids);
                set_op.execute(vec![chunk])?;
            }
        }
        for set_op in &self.on_create {
            if !created_ids.is_empty() {
                let chunk = row_id_chunk(&created_ids);
                set_op.execute(vec![chunk])?;
            }
        }

        tracing::info!("MERGE: processed merges in '{}'", self.table_name);

        if merged_row_ids.is_empty() {
            return Ok(vec![DataChunk::new(vec![], vec![])]);
        }

        // Output one row per processed input row: the post-update table columns
        // (named, so a following RETURN resolves `<alias>.<prop>`), the input
        // pipeline columns, and the `_id` pseudo-column. Previously MERGE
        // emitted a single count chunk, so `MERGE ... SET ... RETURN n.prop`
        // resolved the projection against the count (P53.31).
        let mut output = {
            let table = self
                .table_catalog
                .get_node_table_by_name(&self.table_name)
                .ok_or_else(|| format!("Node table '{}' not found for MERGE", self.table_name))?;
            crate::physical::write_ops::set::build_old_row_chunk(&table.columns, &merged_row_ids, &|row_id, col| {
                table.get_value(row_id as usize, col).cloned()
            })?
        };
        append_pipeline_columns(&mut output, &chunks, &source_rows)?;

        let mut v = ValueVector::new(PhysicalTypeID::Int64, merged_row_ids.len());
        v.resize(merged_row_ids.len());
        for (i, r) in merged_row_ids.iter().enumerate() {
            v.set_i64(i, *r as i64);
        }
        output
            .fields
            .push(akar_common::arrow_vector::ArrowVector::from_legacy(&v).array);
        output.field_types.push(PhysicalTypeID::Int64);
        output.field_names.push("_id".to_string());

        Ok(vec![output])
    }
}
