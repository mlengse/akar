// ========================================================================
// Pass 2: Projection Push-Down
// Removes unused columns from ScanNode operators based on what's needed
// in Projection and Filter expressions.
// ========================================================================

use crate::passes::OptimizationPass;
use kuzu_planner::logical_operator::*;
use std::collections::HashSet;

pub struct ProjectionPushDown;

impl OptimizationPass for ProjectionPushDown {
    fn name(&self) -> &str {
        "projection_push_down"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        // Collect referenced column names from Projection and Filter
        let referenced = collect_referenced_columns(operators);

        if referenced.is_empty() {
            return operators.to_vec();
        }

        operators
            .iter()
            .map(|op| match op {
                LogicalOperator::ScanNode(s) => {
                    let cols: Vec<String> = s.columns.iter().filter(|c| referenced.contains(*c)).cloned().collect();
                    LogicalOperator::ScanNode(LogicalScanNode {
                        columns: cols,
                        ..s.clone()
                    })
                }
                other => other.clone(),
            })
            .collect()
    }
}

/// Collect column names referenced in projection and filter expressions.
fn collect_referenced_columns(operators: &[LogicalOperator]) -> HashSet<String> {
    let mut refs = HashSet::new();
    for op in operators {
        match op {
            LogicalOperator::Projection(p) => {
                for expr in &p.expressions {
                    extract_variables(&expr.expression, &mut refs);
                }
            }
            LogicalOperator::Filter(f) => {
                extract_variables(&f.expression, &mut refs);
            }
            LogicalOperator::CreateRel(c) => {
                refs.insert(format!("{}.id", c.src_node_name));
                refs.insert(format!("{}.id", c.dst_node_name));
                refs.insert(format!("{}._id", c.src_node_name));
                refs.insert(format!("{}._id", c.dst_node_name));
                for (_, expr) in &c.properties {
                    extract_variables(expr, &mut refs);
                }
            }
            LogicalOperator::CreateNode(c) => {
                for (_, expr) in &c.properties {
                    extract_variables(expr, &mut refs);
                }
            }
            LogicalOperator::Set(s) => {
                extract_variables(&s.value, &mut refs);
            }
            LogicalOperator::Unwind(u) => {
                extract_variables(&u.expression, &mut refs);
            }
            _ => {}
        }
    }
    refs
}

/// Extract variable names from an expression tree.
fn extract_variables(expr: &kuzu_parser::ast::Expression, refs: &mut HashSet<String>) {
    match expr {
        kuzu_parser::ast::Expression::Variable(name) => {
            refs.insert(name.clone());
        }
        kuzu_parser::ast::Expression::PropertyAccess(obj, _prop) => {
            extract_variables(obj, refs);
        }
        kuzu_parser::ast::Expression::BinaryOp(_, left, right) => {
            extract_variables(left, refs);
            extract_variables(right, refs);
        }
        kuzu_parser::ast::Expression::UnaryOp(_, inner) => {
            extract_variables(inner, refs);
        }
        kuzu_parser::ast::Expression::FunctionCall(_, args) => {
            for arg in args {
                extract_variables(arg, refs);
            }
        }
        kuzu_parser::ast::Expression::List(items) => {
            for item in items {
                extract_variables(item, refs);
            }
        }
        kuzu_parser::ast::Expression::Map(entries) => {
            for (_, v) in entries {
                extract_variables(v, refs);
            }
        }
        kuzu_parser::ast::Expression::Case(case_expr) => {
            if let Some(subj) = &case_expr.subject {
                extract_variables(subj, refs);
            }
            for alt in &case_expr.alternatives {
                extract_variables(&alt.when, refs);
                extract_variables(&alt.then, refs);
            }
            if let Some(else_e) = &case_expr.else_expr {
                extract_variables(else_e, refs);
            }
        }
        _ => {} // Constant, etc. — no variable refs
    }
}
