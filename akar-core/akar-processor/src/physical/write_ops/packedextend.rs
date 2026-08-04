//! Auto-extracted from physical_operator.rs
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use akar_common::types::Value;
use akar_common::vector::{DataChunk, ValueVector};
use akar_storage::table::TableCatalog;
use std::sync::Arc;

// ==================== PackedExtend ====================

/// Physical operator for multi-rel extend, producing flattened output rows.
///
/// Extends from a source node using a `CsrIndex` or adjacency list and
/// produces one output row per relationship, duplicating the source node
/// properties for each destination neighbor. This normalized format is
/// what downstream operators (HashJoin, Filter, etc.) expect.
pub struct PhysicalPackedExtend {
    pub rel_table_name: String,
    pub rel_table_id: u64,
    pub bound_node_var: String,
    pub direction: akar_parser::ast::EdgeDirection,
    pub dst_node_var: String,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalPackedExtend {
    fn operator_type(&self) -> &str {
        "packed_extend"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        if input.is_empty() || input.iter().all(|c| c.size == 0) {
            return Ok(input);
        }

        let rel_table = self
            .table_catalog
            .get_rel_table_by_name(&self.rel_table_name)
            .ok_or_else(|| format!("Rel table {} not found", self.rel_table_name))?;

        let mut output_chunks = Vec::new();

        for chunk in input {
            if chunk.size == 0 {
                continue;
            }

            // Find bound node column index
            let bound_idx = chunk
                .field_names
                .iter()
                .position(|name| name == &self.bound_node_var)
                .unwrap_or(0);

            // --- Pass 1: collect neighbor lists and estimate total output size ---
            let mut per_src_neighbors: Vec<Vec<u64>> = Vec::with_capacity(chunk.size);
            let mut total_output_rows: usize = 0;

            for i in 0..chunk.size {
                if chunk.fields[bound_idx].is_null(i) {
                    per_src_neighbors.push(Vec::new());
                    continue;
                }

                let src_id = match chunk.get_value(bound_idx, i) {
                    Some(Value::Int64(id)) => id as u64,
                    Some(Value::UInt64(id)) => id,
                    _ => {
                        per_src_neighbors.push(Vec::new());
                        continue;
                    }
                };

                let neighbors = self.fetch_neighbors(&rel_table, src_id);
                total_output_rows += neighbors.len();
                per_src_neighbors.push(neighbors);
            }

            if total_output_rows == 0 {
                continue;
            }

            // --- Pass 2: build flat output vectors with pre-allocated capacity ---
            let num_input_cols = chunk.fields.len();
            let mut out_fields: Vec<arrow::array::ArrayRef> = Vec::with_capacity(num_input_cols + 1);
            let mut out_field_types: Vec<akar_common::types::PhysicalTypeID> = Vec::with_capacity(num_input_cols + 1);

            // Duplicate each input column for every output row
            for col_idx in 0..num_input_cols {
                let phys_type = chunk.field_types[col_idx];
                let mut v = ValueVector::new(phys_type, total_output_rows);
                v.resize(total_output_rows);

                let mut out_pos = 0;
                for (src_row, neighbors) in per_src_neighbors.iter().enumerate() {
                    if neighbors.is_empty() {
                        continue;
                    }
                    // Copy source row value once, then duplicate for each neighbor
                    let is_null = chunk.fields[col_idx].is_null(src_row);
                    let val = chunk.get_value(col_idx, src_row);
                    for _ in neighbors {
                        if is_null {
                            v.set_null(out_pos, true);
                        } else if let Some(ref val) = val {
                            crate::physical::common::store_value_in_vector(&mut v, out_pos, val)?;
                        }
                        out_pos += 1;
                    }
                }
                out_fields.push(akar_common::arrow_vector::ArrowVector::from_legacy(&v).array);
                out_field_types.push(phys_type);
            }

            // Destination column: flat Int64 of neighbor node IDs
            let mut dst_field = ValueVector::new(akar_common::types::PhysicalTypeID::Int64, total_output_rows);
            dst_field.resize(total_output_rows);

            let mut out_pos = 0;
            for neighbors in &per_src_neighbors {
                for &dst_id in neighbors {
                    crate::physical::common::store_value_in_vector(
                        &mut dst_field,
                        out_pos,
                        &Value::Int64(dst_id as i64),
                    )?;
                    out_pos += 1;
                }
            }
            out_fields.push(akar_common::arrow_vector::ArrowVector::from_legacy(&dst_field).array);
            out_field_types.push(akar_common::types::PhysicalTypeID::Int64);

            let mut new_names = chunk.field_names.clone();
            new_names.push(self.dst_node_var.clone());

            output_chunks.push(DataChunk {
                fields: out_fields,
                field_types: out_field_types,
                size: total_output_rows,
                field_names: new_names,
                sel_vector: None,
            });
        }

        if output_chunks.is_empty() {
            Ok(vec![DataChunk {
                fields: vec![],
                field_types: vec![],
                size: 0,
                field_names: vec![],
                sel_vector: None,
            }])
        } else {
            Ok(output_chunks)
        }
    }
}

impl PhysicalPackedExtend {
    fn fetch_neighbors(&self, rel_table: &akar_storage::table::RelTable, src_id: u64) -> Vec<u64> {
        if let Some(csr) = &rel_table.csr_index {
            let is_fwd = matches!(self.direction, akar_parser::ast::EdgeDirection::LeftToRight);
            csr.get_neighbors(src_id, is_fwd).unwrap_or_default()
        } else {
            match self.direction {
                akar_parser::ast::EdgeDirection::LeftToRight => rel_table
                    .get_outgoing_edges(src_id)
                    .into_iter()
                    .map(|(dst, _)| dst)
                    .collect(),
                akar_parser::ast::EdgeDirection::RightToLeft => rel_table
                    .get_incoming_edges(src_id)
                    .into_iter()
                    .map(|(dst, _)| dst)
                    .collect(),
                akar_parser::ast::EdgeDirection::Both => {
                    let mut n: Vec<u64> = rel_table
                        .get_outgoing_edges(src_id)
                        .into_iter()
                        .map(|(dst, _)| dst)
                        .collect();
                    n.extend(rel_table.get_incoming_edges(src_id).into_iter().map(|(dst, _)| dst));
                    n
                }
            }
        }
    }
}
