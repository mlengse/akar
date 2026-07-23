//! Physical operators for INSERT (CreateNode, CreateRel).

use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::physical::write_ops::set::evaluate_expression_for_row;
use akar_common::types::{PhysicalTypeID, Value};
use akar_common::vector::{DataChunk, ValueVector};
use akar_storage::table::TableCatalog;
use std::sync::Arc;

/// Physical operator for CREATE NODE.
pub struct PhysicalInsertNode {
    pub table_name: String,
    pub table_id: u64,
    pub out_var_name: String,
    pub properties: Vec<(String, akar_parser::ast::Expression)>,
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
            let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&v).array;
            vec![DataChunk::new(vec![arr], vec![PhysicalTypeID::Int64])]
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
        let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&v).array;
        Ok(vec![DataChunk::new(vec![arr], vec![PhysicalTypeID::Int64])])
    }
}

/// Physical operator for CREATE REL.
pub struct PhysicalInsertRel {
    pub table_name: String,
    pub table_id: u64,
    pub src_node_name: String,
    pub dst_node_name: String,
    pub properties: Vec<(String, akar_parser::ast::Expression)>,
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
            let src_name_id = format!("{}.{}", self.src_node_name, "_id");
            let src_name_pk = format!("{}.{}", self.src_node_name, "id");
            let src_name_pk_upper = format!("{}.{}", self.src_node_name, "ID");
            let src_node_col_idx = chunk
                .field_names
                .iter()
                .position(|name| name == &src_name_id)
                .or_else(|| chunk.field_names.iter().position(|name| name == &self.src_node_name))
                .or_else(|| chunk.field_names.iter().position(|name| name == &src_name_pk))
                .or_else(|| chunk.field_names.iter().position(|name| name == &src_name_pk_upper))
                .or_else(|| {
                    chunk
                        .field_names
                        .iter()
                        .position(|name| name.eq_ignore_ascii_case(&src_name_pk))
                })
                .ok_or_else(|| {
                    format!(
                        "Source node variable {} not found (fields: {:?})",
                        self.src_node_name, chunk.field_names
                    )
                })?;

            let dst_name_id = format!("{}.{}", self.dst_node_name, "_id");
            let dst_name_pk = format!("{}.{}", self.dst_node_name, "id");
            let dst_name_pk_upper = format!("{}.{}", self.dst_node_name, "ID");
            let dst_node_col_idx = chunk
                .field_names
                .iter()
                .position(|name| name == &dst_name_id)
                .or_else(|| chunk.field_names.iter().position(|name| name == &self.dst_node_name))
                .or_else(|| chunk.field_names.iter().position(|name| name == &dst_name_pk))
                .or_else(|| chunk.field_names.iter().position(|name| name == &dst_name_pk_upper))
                .or_else(|| {
                    chunk
                        .field_names
                        .iter()
                        .position(|name| name.eq_ignore_ascii_case(&dst_name_pk))
                })
                .ok_or_else(|| {
                    format!(
                        "Destination node variable {} not found (fields: {:?})",
                        self.dst_node_name, chunk.field_names
                    )
                })?;

            if src_node_col_idx >= chunk.fields.len() || dst_node_col_idx >= chunk.fields.len() {
                return Err("Src/Dst node column index out of bounds in INSERT REL".into());
            }

            for row in 0..chunk.size {
                let src_id = if let Some(Value::Int64(val)) = chunk.get_value(src_node_col_idx, row) {
                    val as u64
                } else {
                    0
                };
                let dst_id = if let Some(Value::Int64(val)) = chunk.get_value(dst_node_col_idx, row) {
                    val as u64
                } else {
                    0
                };

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
                .map_err(|e| format!("BatchInsert rel error: {e}"))?;
        }

        tracing::info!("INSERT REL: added {inserted_count} rels to '{}'", self.table_name);

        let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
        v.resize(1);
        v.set_i64(0, inserted_count as i64);
        let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&v).array;
        Ok(vec![DataChunk::new(vec![arr], vec![PhysicalTypeID::Int64])])
    }
}
