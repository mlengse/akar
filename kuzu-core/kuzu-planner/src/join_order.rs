//! Join order enumeration — builds optimal join trees from query patterns.
//!
//! Uses a simple greedy heuristic: join the smallest tables first.

use crate::logical_operator::*;
use kuzu_binder::bound_statement::BoundExpression;
use kuzu_parser::ast::{BinaryOp, Expression};

/// A join plan tree representing how to combine scan operators.
#[derive(Debug, Clone)]
pub enum JoinPlan {
    /// A single leaf operator (ScanNode or ScanRel).
    Leaf(LogicalOperator),
    /// Hash join of two sub-plans with join keys.
    HashJoin {
        keys: Vec<Expression>,
        left: Box<JoinPlan>,
        right: Box<JoinPlan>,
    },
    /// Cross product of two sub-plans.
    CrossProduct {
        left: Box<JoinPlan>,
        right: Box<JoinPlan>,
    },
}

/// Build a join tree from a list of scan operators and an optional filter expression.
///
/// Uses a greedy heuristic:
/// 1. Start with the first scan as the base
/// 2. For each remaining scan, find any join conditions from the filter
/// 3. If join conditions exist → HashJoin, otherwise → CrossProduct
pub fn build_join_tree(
    scans: Vec<LogicalOperator>,
    filter_expr: Option<&BoundExpression>,
) -> JoinPlan {
    if scans.is_empty() {
        return JoinPlan::Leaf(LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "empty".into(),
            table_id: 0,
            alias: None,
            columns: Vec::new(),
            cardinality: 0,
        }));
    }

    if scans.len() == 1 {
        return JoinPlan::Leaf(scans.into_iter().next().unwrap());
    }

    // Extract table aliases from scans for join condition matching

    // Extract potential join conditions from the filter
    let join_conditions = filter_expr.map_or(Vec::new(), |f| extract_join_conditions(&f.expression));

    // Greedy join ordering: start with the first scan, then join each subsequent one
    let mut scans_iter = scans.into_iter();
    let first = scans_iter.next().unwrap();
    let mut result = JoinPlan::Leaf(first);

    for scan in scans_iter {
        let alias = get_scan_alias(&scan);

        // Try to find a join condition matching this scan's alias
        let matching_conditions: Vec<Expression> = join_conditions.iter()
            .filter(|(left_alias, right_alias, _expr)| {
                left_alias == &alias || right_alias == &alias
            })
            .map(|(_, _, expr)| expr.clone())
            .collect();

        if matching_conditions.is_empty() {
            // No join condition found — use cross product
            result = JoinPlan::CrossProduct {
                left: Box::new(result),
                right: Box::new(JoinPlan::Leaf(scan)),
            };
        } else {
            // Use the first matching join condition
            result = JoinPlan::HashJoin {
                keys: matching_conditions,
                left: Box::new(result),
                right: Box::new(JoinPlan::Leaf(scan)),
            };
        }
    }

    result
}

/// Extract table alias from a logical operator.
fn get_scan_alias(op: &LogicalOperator) -> Option<String> {
    match op {
        LogicalOperator::ScanNode(s) => s.alias.clone(),
        LogicalOperator::ScanRel(s) => {
            // Rel scans don't have aliases; use table_name
            Some(s.table_name.clone())
        }
        _ => None,
    }
}

/// Extract potential join conditions from a filter expression.
///
/// Looks for equality comparisons between variables (e.g., `a.id = b.id`).
/// Returns tuples of (left_alias, right_alias, condition_expression).
fn extract_join_conditions(expr: &Expression) -> Vec<(Option<String>, Option<String>, Expression)> {
    let mut conditions = Vec::new();
    collect_equality_conditions(expr, &mut conditions);
    conditions
}

/// Recursively collect equality conditions that reference different variables.
fn collect_equality_conditions(
    expr: &Expression,
    conditions: &mut Vec<(Option<String>, Option<String>, Expression)>,
) {
    match expr {
        Expression::BinaryOp(BinaryOp::Equal, left, right) => {
            let left_var = extract_variable_alias(left);
            let right_var = extract_variable_alias(right);
            if let (Some(lv), Some(rv)) = (&left_var, &right_var) {
                if lv != rv {
                    // This is a potential join condition between two different variables
                    conditions.push((left_var, right_var, expr.clone()));
                    return;
                }
            }
            // Fall through to check children
        }
        Expression::BinaryOp(BinaryOp::And, left, right) => {
            collect_equality_conditions(left, conditions);
            collect_equality_conditions(right, conditions);
            return;
        }
        _ => {}
    }
}

/// Extract the variable alias from an expression.
/// e.g., `a.id` → `"a"`, `b` → `"b"`
fn extract_variable_alias(expr: &Expression) -> Option<String> {
    match expr {
        Expression::Variable(name) => Some(name.clone()),
        Expression::PropertyAccess(obj, _) => extract_variable_alias(obj),
        _ => None,
    }
}

/// Convert a JoinPlan tree to a flat Vec<LogicalOperator> for the processor.
///
/// The flattening order ensures scans appear before joins, and
/// joins appear before filters/projections.
pub fn flatten_join_plan(plan: &JoinPlan) -> Vec<LogicalOperator> {
    let mut ops = Vec::new();
    flatten_plan(plan, &mut ops);
    ops
}

fn flatten_plan(plan: &JoinPlan, ops: &mut Vec<LogicalOperator>) {
    match plan {
        JoinPlan::Leaf(op) => {
            ops.push(op.clone());
        }
        JoinPlan::HashJoin { keys, left, right } => {
            // Flatten children first (scans before join)
            flatten_plan(left, ops);
            flatten_plan(right, ops);
            // Then add the join operator
            ops.push(LogicalOperator::HashJoin(LogicalHashJoin {
                join_keys: keys.clone(),
                build_side: Box::new(LogicalOperator::ScanNode(LogicalScanNode {
                    table_name: "join_build".into(),
                    table_id: 0,
                    alias: None,
                    columns: Vec::new(),
                    cardinality: 0,
                })),
                probe_side: Box::new(LogicalOperator::ScanNode(LogicalScanNode {
                    table_name: "join_probe".into(),
                    table_id: 0,
                    alias: None,
                    columns: Vec::new(),
                    cardinality: 0,
                })),
                cardinality: 0,
            }));
        }
        JoinPlan::CrossProduct { left, right } => {
            flatten_plan(left, ops);
            flatten_plan(right, ops);
            ops.push(LogicalOperator::CrossProduct(LogicalCrossProduct {
                left: Box::new(LogicalOperator::ScanNode(LogicalScanNode {
                    table_name: "cp_left".into(),
                    table_id: 0,
                    alias: None,
                    columns: Vec::new(),
                    cardinality: 0,
                })),
                right: Box::new(LogicalOperator::ScanNode(LogicalScanNode {
                    table_name: "cp_right".into(),
                    table_id: 0,
                    alias: None,
                    columns: Vec::new(),
                    cardinality: 0,
                })),
                cardinality: 0,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_scan_leaf() {
        let scan = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("a".into()),
            columns: Vec::new(),
            cardinality: 0,
        });
        let plan = build_join_tree(vec![scan], None);
        match plan {
            JoinPlan::Leaf(_) => {}
            _ => panic!("Expected Leaf"),
        }
    }

    #[test]
    fn test_two_scans_cross_product() {
        let scan1 = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "Person".into(), table_id: 0, alias: Some("a".into()), columns: Vec::new(), cardinality: 0,
        });
        let scan2 = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "City".into(), table_id: 1, alias: Some("c".into()), columns: Vec::new(), cardinality: 0,
        });
        let plan = build_join_tree(vec![scan1, scan2], None);
        match plan {
            JoinPlan::CrossProduct { .. } => {}
            _ => panic!("Expected CrossProduct"),
        }
    }

    #[test]
    fn test_join_condition_extraction() {
        use kuzu_parser::ast::Expression;
        // a.id = b.id
        let expr = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())),
                "id".into(),
            )),
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("b".into())),
                "id".into(),
            )),
        );
        let conditions = extract_join_conditions(&expr);
        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].0, Some("a".into()));
        assert_eq!(conditions[0].1, Some("b".into()));
    }

    #[test]
    fn test_no_join_condition() {
        use kuzu_parser::ast::Constant;
        let expr = Expression::BinaryOp(
            BinaryOp::GreaterThan,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())),
                "age".into(),
            )),
            Box::new(Expression::Constant(Constant::Integer(25))),
        );
        let conditions = extract_join_conditions(&expr);
        assert!(conditions.is_empty());
    }

    #[test]
    fn test_and_condition_extraction() {
        use kuzu_parser::ast::Expression;
        // a.id = b.id AND a.age > 25
        let expr = Expression::BinaryOp(
            BinaryOp::And,
            Box::new(Expression::BinaryOp(
                BinaryOp::Equal,
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("a".into())), "id".into(),
                )),
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("b".into())), "id".into(),
                )),
            )),
            Box::new(Expression::BinaryOp(
                BinaryOp::GreaterThan,
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("a".into())), "age".into(),
                )),
                Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(25))),
            )),
        );
        let conditions = extract_join_conditions(&expr);
        assert_eq!(conditions.len(), 1, "Should find 1 join condition");
        // The age > 25 is NOT a join condition
    }

    #[test]
    fn test_extract_variable_alias() {
        let expr = Expression::PropertyAccess(
            Box::new(Expression::Variable("p".into())),
            "name".into(),
        );
        assert_eq!(extract_variable_alias(&expr), Some("p".into()));

        let expr = Expression::Variable("x".into());
        assert_eq!(extract_variable_alias(&expr), Some("x".into()));
    }

    #[test]
    fn test_flatten_join_plan() {
        let scan = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "T".into(), table_id: 0, alias: None, columns: Vec::new(), cardinality: 0,
        });
        let plan = JoinPlan::Leaf(scan.clone());
        let flat = flatten_join_plan(&plan);
        assert_eq!(flat.len(), 1);
    }

    #[test]
    fn test_flatten_cross_product() {
        let scan1 = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "A".into(), table_id: 0, alias: None, columns: Vec::new(), cardinality: 0,
        });
        let scan2 = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "B".into(), table_id: 1, alias: None, columns: Vec::new(), cardinality: 0,
        });
        let plan = JoinPlan::CrossProduct {
            left: Box::new(JoinPlan::Leaf(scan1)),
            right: Box::new(JoinPlan::Leaf(scan2)),
        };
        let flat = flatten_join_plan(&plan);
        assert_eq!(flat.len(), 3); // 2 scans + 1 cross product
        assert!(matches!(flat[2], LogicalOperator::CrossProduct(_)));
    }
}
