use crate::physical::common::store_value_in_vector;
use crate::physical::scan_filter::PhysicalScan;
use akar_common::error::ProcessorError;
use akar_common::types::{PhysicalTypeID, Value};
use akar_common::vector::{DataChunk, ValueVector};
use akar_parser::ast::EdgeDirection;
use akar_storage::table::TableCatalog;
use std::collections::HashMap;
use std::sync::Arc;

// ==================== OptionalExtend ====================

/// Physical operator for `OPTIONAL MATCH` over an already-bound pair of node
/// variables, e.g. `OPTIONAL MATCH (a)-[existing:Connected]-(b)` where both
/// `a` and `b` come from the mandatory side (P53.25).
///
/// For each input row the relationship-table adjacency is probed for an edge
/// between the source and destination node ids. When an edge exists, its
/// property columns (plus an internal `{rel_var}._id` holding the edge index)
/// are emitted; when no edge exists, those columns are NULL-padded and exactly
/// one row is still produced (outer-join semantics).
///
/// Output layout: `[input_fields | rel_properties | {rel_var}._id]`.
pub struct PhysicalOptionalExtend {
    /// Name of the relationship table to probe.
    pub rel_table_name: String,
    /// ID of the relationship table.
    pub rel_table_id: u64,
    /// Variable name of the relationship (e.g., "existing"); prefix for the
    /// emitted edge property columns.
    pub rel_var: String,
    /// Variable name of the bound source node (e.g., "a").
    pub src_node_var: String,
    /// Variable name of the bound destination node (e.g., "b").
    pub dst_node_var: String,
    /// Direction of the probe (forward, backward, or both).
    pub direction: EdgeDirection,
    /// Table catalog for data access.
    pub table_catalog: Arc<TableCatalog>,
}

/// Resolve the internal node-id column (`{var}._id`, falling back to the bare
/// variable or the primary key column) in the input chunk.
fn find_node_id_col(chunk: &DataChunk, var: &str) -> Result<usize, ProcessorError> {
    let name_id = format!("{}.{}", var, "_id");
    let name_pk = format!("{}.{}", var, "id");
    let idx = chunk
        .field_names
        .iter()
        .position(|n| n == &name_id)
        .or_else(|| chunk.field_names.iter().position(|n| n == var))
        .or_else(|| chunk.field_names.iter().position(|n| n == &name_pk));
    idx.ok_or_else(|| {
        format!(
            "Node variable {} not found in OptionalExtend input. Available fields: {:?}",
            var, chunk.field_names
        )
        .into()
    })
}

/// Find an edge index between `src` and `dst` in the forward/reverse adjacency
/// maps, honoring the probe direction. Returns the first matching edge index.
fn probe_edge(
    fwd_adj: &HashMap<u64, Vec<(u64, usize)>>,
    rev_adj: &HashMap<u64, Vec<(u64, usize)>>,
    src: u64,
    dst: u64,
    direction: &EdgeDirection,
) -> Option<usize> {
    match direction {
        EdgeDirection::LeftToRight => fwd_adj
            .get(&src)
            .and_then(|e| e.iter().find(|(o, _)| *o == dst).map(|(_, i)| *i)),
        EdgeDirection::RightToLeft => rev_adj
            .get(&src)
            .and_then(|e| e.iter().find(|(o, _)| *o == dst).map(|(_, i)| *i)),
        EdgeDirection::Both => fwd_adj
            .get(&src)
            .and_then(|e| e.iter().find(|(o, _)| *o == dst).map(|(_, i)| *i))
            .or_else(|| {
                rev_adj
                    .get(&src)
                    .and_then(|e| e.iter().find(|(o, _)| *o == dst).map(|(_, i)| *i))
            }),
    }
}

impl PhysicalOptionalExtend {
    pub fn execute(&self, input: Vec<DataChunk>) -> Result<Vec<DataChunk>, ProcessorError> {
        if input.is_empty() {
            return Ok(input);
        }

        // Collect rel table data upfront (owned)
        let (fwd_adj, rev_adj, rel_props, rel_cols) = {
            let rel_table = self
                .table_catalog
                .get_rel_table_by_name(&self.rel_table_name)
                .ok_or_else(|| format!("Rel table {} not found", self.rel_table_name))?;
            (
                rel_table.fwd_adj.clone(),
                rel_table.rev_adj.clone(),
                rel_table.properties.clone(),
                rel_table.columns.clone(),
            )
        };

        let num_rel_cols = rel_cols.len();
        let rel_prefix = if self.rel_var.is_empty() {
            self.rel_table_name.clone()
        } else {
            self.rel_var.clone()
        };
        let rel_field_names: Vec<String> = rel_cols.iter().map(|c| format!("{}.{}", rel_prefix, c.name)).collect();

        let mut output = Vec::with_capacity(input.len());

        for chunk in input {
            if chunk.size == 0 {
                output.push(chunk);
                continue;
            }

            let src_idx = find_node_id_col(&chunk, &self.src_node_var)?;
            let dst_idx = find_node_id_col(&chunk, &self.dst_node_var)?;

            let num_input_fields = chunk.fields.len();
            let num_out_cols = num_input_fields + num_rel_cols + 1;
            let mut out_data: Vec<Vec<Value>> = vec![Vec::with_capacity(chunk.size); num_out_cols];

            for i in 0..chunk.size {
                for col in 0..num_input_fields {
                    let val = chunk.get_value(col, i).unwrap_or(Value::Null);
                    out_data[col].push(val);
                }

                // Probe the adjacency for an edge between src and dst.
                let edge_idx = match (chunk.get_value(src_idx, i), chunk.get_value(dst_idx, i)) {
                    (Some(Value::Int64(s)), Some(Value::Int64(d))) => {
                        probe_edge(&fwd_adj, &rev_adj, s as u64, d as u64, &self.direction)
                    }
                    _ => None,
                };

                for col in 0..num_rel_cols {
                    let val = match edge_idx {
                        Some(ei) => rel_props
                            .get(col)
                            .and_then(|c| c.get(ei))
                            .cloned()
                            .unwrap_or(Value::Null),
                        None => Value::Null,
                    };
                    out_data[num_input_fields + col].push(val);
                }
                // Internal edge index — non-NULL iff an edge exists, so a
                // zero-property rel table still answers `{rel_var} IS NULL`.
                out_data[num_input_fields + num_rel_cols].push(match edge_idx {
                    Some(ei) => Value::Int64(ei as i64),
                    None => Value::Null,
                });
            }

            // Build output columns.
            let mut fields = Vec::with_capacity(num_out_cols);
            let mut field_types = Vec::with_capacity(num_out_cols);
            let mut field_names = Vec::with_capacity(num_out_cols);

            for col in 0..num_input_fields {
                let phys_type = chunk.field_types[col];
                let mut v = ValueVector::new(phys_type, chunk.size);
                v.resize(chunk.size);
                for row in 0..chunk.size {
                    store_value_in_vector(&mut v, row, &out_data[col][row])?;
                }
                fields.push(v);
                field_types.push(phys_type);
                field_names.push(if col < chunk.field_names.len() {
                    chunk.field_names[col].clone()
                } else {
                    format!("field_{}", col)
                });
            }
            for col in 0..num_rel_cols {
                let phys_type = if col < rel_cols.len() {
                    PhysicalScan::logical_to_physical(&rel_cols[col].logical_type)
                } else {
                    PhysicalTypeID::Int64
                };
                let mut v = ValueVector::new(phys_type, chunk.size);
                v.resize(chunk.size);
                for row in 0..chunk.size {
                    store_value_in_vector(&mut v, row, &out_data[num_input_fields + col][row])?;
                }
                fields.push(v);
                field_types.push(phys_type);
                field_names.push(rel_field_names[col].clone());
            }
            // Internal edge index column (`{rel_var}._id`).
            let mut id_v = ValueVector::new(PhysicalTypeID::Int64, chunk.size);
            id_v.resize(chunk.size);
            for row in 0..chunk.size {
                store_value_in_vector(&mut id_v, row, &out_data[num_input_fields + num_rel_cols][row])?;
            }
            fields.push(id_v);
            field_types.push(PhysicalTypeID::Int64);
            field_names.push(format!("{}.{}", rel_prefix, "_id"));

            let arrow_fields = fields
                .iter()
                .map(|v| akar_common::arrow_vector::ArrowVector::from_legacy(v).array)
                .collect::<Vec<_>>();
            let arrow_field_types = fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
            output.push(DataChunk {
                fields: arrow_fields,
                field_types: arrow_field_types,
                size: chunk.size,
                field_names,
                sel_vector: None,
            });
        }

        Ok(output)
    }
}
