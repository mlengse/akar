// ========================================================================
// Pass: Agg Key Dependency Optimization
// Removes redundant GROUP BY keys that are functionally dependent on others.
//
// Ported from C++ `agg_key_dependency_optimizer.cpp`.
//
// If a GROUP BY contains both `a.id` (the primary key) and `a.name`,
// then `a.name` is functionally dependent on `a.id` and can be removed.
// This reduces the hash table size in the aggregate operator.
//
// Uses a naming heuristic: properties named "id", "_id", "ID" etc.
// are treated as primary keys. Other properties of the same variable
// are removed from GROUP BY.
// ========================================================================

use crate::passes::TreeOptimizationPass;
use kuzu_planner::logical_operator::*;

pub struct AggKeyDependency;

impl AggKeyDependency {
    /// Check if a property name looks like a primary key identifier.
    fn is_id_property(name: &str) -> bool {
        matches!(name.to_lowercase().as_str(), "id" | "_id")
    }
}

impl TreeOptimizationPass for AggKeyDependency {
    fn name(&self) -> &str {
        "agg_key_dependency"
    }

    fn apply_tree(&self, root: &mut LogicalOperator) {
        LogicalOperator::visit_bottom_up(root, &mut |op| {
            if let LogicalOperator::Aggregate(agg) = op {
                if agg.group_by.len() <= 1 {
                    return; // Nothing to optimize
                }

                let original_count = agg.group_by.len();

                // Phase 1: Identify which PropertyAccess expressions are "keys"
                // (primary identifiers) and which are "dependent" (redundant).
                // A key is one where:
                //   - The property is named "id", "_id", "ID" (heuristic PK), OR
                //   - It's the first PropertyAccess encountered for that variable
                //
                // Every other PropertyAccess for the same variable is dependent
                // (functionally determined by the key).

                // Maps variable → property names that are keys
                let mut var_key_props: std::collections::HashMap<String, String> = std::collections::HashMap::new();

                // First pass: find ID properties (these take priority as keys)
                for key in &agg.group_by {
                    if let kuzu_parser::ast::Expression::PropertyAccess(obj, prop) = key
                        && let kuzu_parser::ast::Expression::Variable(var) = obj.as_ref()
                        && Self::is_id_property(prop)
                    {
                        var_key_props.entry(var.clone()).or_insert_with(|| prop.clone());
                    }
                }

                // Second pass: for variables without an ID property, use the first
                // PropertyAccess as the key.
                for key in &agg.group_by {
                    if let kuzu_parser::ast::Expression::PropertyAccess(obj, prop) = key
                        && let kuzu_parser::ast::Expression::Variable(var) = obj.as_ref()
                        && !var_key_props.contains_key(var)
                    {
                        var_key_props.insert(var.clone(), prop.clone());
                    }
                }

                // Phase 2: Filter keys — keep key expressions and non-property
                // expressions. Remove dependent property expressions.
                let mut new_keys: Vec<kuzu_parser::ast::Expression> = Vec::new();
                for key in agg.group_by.drain(..) {
                    let is_dependent = match &key {
                        kuzu_parser::ast::Expression::PropertyAccess(obj, prop) => {
                            if let kuzu_parser::ast::Expression::Variable(var) = obj.as_ref() {
                                // If this variable has a registered key property,
                                // and this expression is NOT that key → dependent
                                if let Some(key_prop) = var_key_props.get(var) {
                                    prop != key_prop
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        }
                        _ => false,
                    };
                    if !is_dependent {
                        new_keys.push(key);
                    }
                }
                agg.group_by = new_keys;

                if agg.group_by.len() < original_count {
                    tracing::debug!(
                        "AggKeyDependency: reduced group_by from {} to {} keys",
                        original_count,
                        agg.group_by.len()
                    );
                }
            }
        });
    }
}
