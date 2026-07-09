//! Auto-extracted from physical_operator.rs
use kuzu_common::types::{PhysicalTypeID, Value};
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_parser::ast::{Constant, Expression};
use kuzu_storage::table::{ColumnDefinition, TableCatalog};
use std::path::Path;
use std::sync::{Arc, Mutex};
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::physical::scan_filter::PhysicalScan;
use crate::physical::common::store_value_in_vector;

// ==================== PackedExtend ====================

/// Physical operator for multi-rel extend, producing packed columns.
///
/// Extends from a source node using a `CsrIndex` and produces a packed 
/// list of destination nodes for each source node.
pub struct PhysicalPackedExtend {
    pub rel_table_name: String,
    pub rel_table_id: u64,
    pub bound_node_var: String,
    pub direction: kuzu_parser::ast::EdgeDirection,
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
                .unwrap_or(0); // Fallback to 0

            let bound_field = &chunk.fields[bound_idx];
            
            // Create output chunk fields (copy input fields, append packed dst field)
            let mut new_fields = Vec::with_capacity(chunk.fields.len() + 1);
            for field in &chunk.fields {
                let mut new_v = kuzu_common::vector::ValueVector::new(field.physical_type(), chunk.size);
                new_v.resize(chunk.size);
                for i in 0..chunk.size {
                    if field.is_null(i) {
                        new_v.set_null(i, true);
                    } else if let Some(val) = field.get_value(i) {
                        crate::physical::common::store_value_in_vector(&mut new_v, i, &val);
                    }
                }
                new_fields.push(new_v);
            }
            
            // Output field for packed destination nodes (using List)
            let mut dst_field = kuzu_common::vector::ValueVector::new(kuzu_common::types::PhysicalTypeID::List, chunk.size);
            dst_field.resize(chunk.size);
            
            for i in 0..chunk.size {
                if bound_field.is_null(i) {
                    dst_field.set_null(i, true);
                    continue;
                }
                
                let src_id = if let Some(kuzu_common::types::Value::Int64(id)) = bound_field.get_value(i) {
                    id as u64
                } else if let Some(kuzu_common::types::Value::UInt64(id)) = bound_field.get_value(i) {
                    id
                } else {
                    continue;
                };

                // Read from CsrIndex if available, otherwise fallback to adjacency list
                let neighbors = if let Some(csr) = &rel_table.csr_index {
                    csr.get_neighbors(src_id).unwrap_or_default()
                } else {
                    // Fallback to simple adjacency lookup
                    match self.direction {
                        kuzu_parser::ast::EdgeDirection::LeftToRight => {
                            rel_table.get_outgoing_edges(src_id).into_iter().map(|(dst, _)| dst).collect()
                        }
                        kuzu_parser::ast::EdgeDirection::RightToLeft => {
                            rel_table.get_incoming_edges(src_id).into_iter().map(|(dst, _)| dst).collect()
                        }
                        kuzu_parser::ast::EdgeDirection::Both => {
                            let mut n = rel_table.get_outgoing_edges(src_id).into_iter().map(|(dst, _)| dst).collect::<Vec<_>>();
                            n.extend(rel_table.get_incoming_edges(src_id).into_iter().map(|(dst, _)| dst));
                            n
                        }
                    }
                };

                // Output as List
                let list_val = kuzu_common::types::Value::List(
                    neighbors.into_iter().map(|n| kuzu_common::types::Value::Int64(n as i64)).collect()
                );
                crate::physical::common::store_value_in_vector(&mut dst_field, i, &list_val);
            }
            
            new_fields.push(dst_field);
            
            let mut new_names = chunk.field_names.clone();
            new_names.push(self.dst_node_var.clone());
            
            output_chunks.push(DataChunk {
                fields: new_fields,
                size: chunk.size,
                field_names: new_names,
            });
        }
        
        Ok(output_chunks)
    }
}
