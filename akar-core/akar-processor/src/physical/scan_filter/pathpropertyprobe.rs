//! Auto-extracted from physical_operator.rs
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use akar_common::types::Value;
use akar_common::vector::{DataChunk, ValueVector};
use std::sync::Arc;

// ==================== PathPropertyProbe ====================

/// A resolved property column specification for PathPropertyProbe.
#[derive(Debug, Clone)]
pub struct PathPropertySpec {
    /// The table name to look up (node or rel table).
    pub table_name: String,
    /// Whether this is a node or rel table.
    pub is_node: bool,
    /// The property column(s) to extract.
    pub property_names: Vec<String>,
}

/// PathPropertyProbe — resolves properties on path-typed results.
///
/// Path results contain node/edge IDs as `List(Int64)` columns.
/// This operator looks up the actual property values from the node/rel
/// tables and appends them as new columns.
pub struct PhysicalPathPropertyProbe {
    /// The column index in the input containing path node IDs (List(Int64)).
    pub node_ids_col_idx: usize,
    /// The column index in the input containing path edge IDs (List(Int64)).
    pub edge_ids_col_idx: Option<usize>,
    /// Property specifications to resolve.
    pub properties: Vec<PathPropertySpec>,
    /// Catalog for table lookups.
    pub table_catalog: Arc<akar_storage::table::TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalPathPropertyProbe {
    fn operator_type(&self) -> &str {
        "path_property_probe"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        if input.is_empty() {
            return Ok(input);
        }

        let mut output = Vec::with_capacity(input.len());

        for chunk in input {
            if chunk.size == 0 || self.node_ids_col_idx >= chunk.fields.len() {
                output.push(chunk);
                continue;
            }

            let _node_ids_field = &chunk.fields[self.node_ids_col_idx];
            let mut extra_fields: Vec<(String, ValueVector)> = Vec::new();

            for spec in &self.properties {
                let is_node = spec.is_node;
                let table = if is_node {
                    self.table_catalog.get_node_table_by_name(&spec.table_name)
                } else {
                    None
                };

                let Some(ref table) = table else {
                    continue;
                };

                // Build a map from property name to column index
                let col_map: std::collections::HashMap<&str, usize> = table
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(i, c)| (c.name.as_str(), i))
                    .collect();

                for prop_name in &spec.property_names {
                    let Some(&col_idx) = col_map.get(prop_name.as_str()) else {
                        continue;
                    };
                    let phys_type = akar_common::types::physical_type_from_logical(table.columns[col_idx].logical_type);

                    let mut fv = ValueVector::new(phys_type, chunk.size);
                    fv.resize(chunk.size);

                    for row in 0..chunk.size {
                        let path_val = chunk.get_value(self.node_ids_col_idx, row);
                        match path_val {
                            Some(Value::List(nodes)) => {
                                // For path property probe: take the LAST node in the path
                                // (the destination node). This matches C++ Akar behavior
                                // where property probe resolves the destination node.
                                if let Some(Value::Int64(node_id)) = nodes.last() {
                                    let val = table
                                        .get_value(*node_id as usize, col_idx)
                                        .cloned()
                                        .unwrap_or(Value::Null);
                                    if matches!(val, Value::Null) {
                                        fv.set_null(row, true);
                                    } else {
                                        crate::physical::common::store_value_in_vector(&mut fv, row, &val)?;
                                    }
                                } else {
                                    fv.set_null(row, true);
                                }
                            }
                            _ => {
                                fv.set_null(row, true);
                            }
                        }
                    }

                    extra_fields.push((prop_name.clone(), fv));
                }
            }

            // Append resolved property columns to the chunk
            let mut fields = chunk.fields;
            let mut field_types = chunk.field_types;
            let mut field_names = chunk.field_names;
            for (name, fv) in extra_fields {
                field_names.push(name);
                fields.push(akar_common::arrow_vector::ArrowVector::from_legacy(&fv).array);
                field_types.push(fv.physical_type());
            }

            output.push(DataChunk {
                fields,
                field_types,
                size: chunk.size,
                field_names,
                sel_vector: None,
            });
        }

        Ok(output)
    }
}
