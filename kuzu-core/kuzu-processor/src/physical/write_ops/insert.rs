//! Physical operators for INSERT (CreateNode, CreateRel).

use kuzu_common::types::{PhysicalTypeID, Value};
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_storage::table::TableCatalog;
use std::sync::Arc;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::physical::write_ops::set::evaluate_expression_for_row;

/// Physical operator for CREATE NODE.
pub struct PhysicalInsertNode {
    pub table_name: String,
    pub table_id: u64,
    pub out_var_name: String,
    pub properties: Vec<(String, kuzu_parser::ast::Expression)>,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalInsertNode {
    fn operator_type(&self) -> &str {
        "insert_node"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let mut inserted_count = 0u64;
        let mut table = self
            .table_catalog
            .get_node_table_by_name_mut(&self.table_name)
            .ok_or_else(|| format!("Node table '{}' not found for INSERT", self.table_name))?;

        // If input is empty (no previous pipeline), we insert exactly one node.
        // Otherwise, we insert a node for each row in the input.
        let chunks = if input.is_empty() {
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
            v.resize(1);
            v.set_i64(0, 0);
            vec![DataChunk::new(vec![v])]
        } else {
            input
        };

        let num_cols = table.columns.len();
        for chunk in &chunks {
            for row in 0..chunk.size {
                let mut row_values = vec![Value::Null; num_cols];
                
                for (prop_name, expr) in &self.properties {
                    if let Some(col_idx) = table.columns.iter().position(|c| c.name == *prop_name) {
                        let val = evaluate_expression_for_row(expr, chunk, row);
                        row_values[col_idx] = val;
                    }
                }
                
                // Add the row to the node table
                if table.insert_row(row_values).is_ok() {
                    inserted_count += 1;
                }
            }
        }

        tracing::info!("INSERT NODE: added {inserted_count} rows to '{}'", self.table_name);

        let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
        v.resize(1);
        v.set_i64(0, inserted_count as i64);
        Ok(vec![DataChunk::new(vec![v])])
    }
}

/// Physical operator for CREATE REL.
pub struct PhysicalInsertRel {
    pub table_name: String,
    pub table_id: u64,
    pub src_node_col_idx: usize,
    pub dst_node_col_idx: usize,
    pub properties: Vec<(String, kuzu_parser::ast::Expression)>,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalInsertRel {
    fn operator_type(&self) -> &str {
        "insert_rel"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let mut inserted_count = 0u64;
        let mut table = self
            .table_catalog
            .get_rel_table_by_name_mut(&self.table_name)
            .ok_or_else(|| format!("Rel table '{}' not found for INSERT", self.table_name))?;

        let num_cols = table.columns.len();        
        let mut rels_to_insert = Vec::new();

        for chunk in &input {
            if self.src_node_col_idx >= chunk.fields.len() || self.dst_node_col_idx >= chunk.fields.len() {
                return Err("Src/Dst node column index out of bounds in INSERT REL".into());
            }
            
            for row in 0..chunk.size {
                let src_id = chunk.fields[self.src_node_col_idx].get_i64(row).unwrap_or(0) as u64;
                let dst_id = chunk.fields[self.dst_node_col_idx].get_i64(row).unwrap_or(0) as u64;
                
                let mut props = vec![Value::Null; num_cols];
                for (prop_name, expr) in &self.properties {
                    if let Some(col_idx) = table.columns.iter().position(|c| c.name == *prop_name) {
                        let val = evaluate_expression_for_row(expr, chunk, row);
                        props[col_idx] = val;
                    }
                }
                
                rels_to_insert.push((src_id, dst_id, props));
            }
        }
        
        // Batch insert the collected relationships
        if !rels_to_insert.is_empty() {
            inserted_count = table
                .insert_rels_batch(&rels_to_insert)
                .map_err(|e| format!("BatchInsert rel error: {e}"))? as u64;
        }

        tracing::info!("INSERT REL: added {inserted_count} rels to '{}'", self.table_name);

        let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
        v.resize(1);
        v.set_i64(0, inserted_count as i64);
        Ok(vec![DataChunk::new(vec![v])])
    }
}
