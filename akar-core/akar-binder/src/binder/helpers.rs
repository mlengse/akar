//! Shared helper functions for the binder.

use crate::bound_statement::*;
use akar_catalog::Catalog;
use akar_parser::ast::{Expression, SetItem};

/// Resolve SET clause items against the catalog to find column info.
pub(crate) fn resolve_set_items(catalog: &Catalog, items: &[SetItem]) -> Result<Vec<BoundSetItem>, String> {
    let mut result = Vec::new();
    for item in items {
        // Expect property expression like `n.property_name = value`
        match &item.property {
            Expression::PropertyAccess(obj, prop_name) => {
                // Find the variable name by looking at the object
                let _var_name = match obj.as_ref() {
                    Expression::Variable(v) => v.clone(),
                    other => return Err(format!("Unsupported SET target: {:?}", other)),
                };
                // Look up the column in the table schema
                let found = catalog.all_entries().find_map(|entry| {
                    entry.columns().iter().find(|c| c.name == *prop_name).map(|_| {
                        let is_node = entry.is_node_table();
                        (entry.name().to_string(), entry.table_id(), is_node)
                    })
                });
                match found {
                    Some((table_name, table_id, is_node)) => {
                        let col_idx = catalog
                            .get_entry_by_name(&table_name)
                            .and_then(|e| e.columns().iter().position(|c| c.name == *prop_name))
                            .unwrap_or(0);
                        result.push(BoundSetItem {
                            property: item.property.clone(),
                            value: item.value.clone(),
                            column_name: prop_name.clone(),
                            column_idx: col_idx,
                            table_name: table_name.to_string(),
                            table_id,
                            is_node,
                        });
                    }
                    None => {
                        return Err(format!("Property '{}' not found in any table", prop_name));
                    }
                }
            }
            _ => return Err(format!("Expected property assignment in SET, got: {:?}", item.property)),
        }
    }
    Ok(result)
}

/// Convert an Expression AST to a debug string for macro display.
pub(crate) fn expr_to_debug_string(expr: &akar_parser::ast::Expression) -> String {
    format!("{:?}", expr)
}
