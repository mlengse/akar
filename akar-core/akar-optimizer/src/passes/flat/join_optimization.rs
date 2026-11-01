// ========================================================================
// Pass 6: Join Optimization
// Converts filter equality conditions to join conditions.
// Reorders joins so the smallest tables are joined first (cardinality-aware).
// ========================================================================

use crate::passes::OptimizationPass;
use akar_planner::logical_operator::*;

pub struct JoinOptimization;

impl OptimizationPass for JoinOptimization {
    fn name(&self) -> &str {
        "join_optimization"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        // Try cardinality-aware join reordering
        if let Some(reordered) = crate::join_order::reorder_joins_dp_bushy(operators) {
            return reordered;
        }

        // Only drop equality join-condition filters when the plan contains a
        // real join (CrossProduct/HashJoin) that consumes them as keys.
        // A plan WITHOUT a join (single-scan pipelines, WCOJ/Intersect plans)
        // must keep these filters, otherwise `a.id = b.id` silently passes
        // every row (P48.4 BUG-B).
        let has_join = operators
            .iter()
            .any(|op| matches!(op, LogicalOperator::CrossProduct(_) | LogicalOperator::HashJoin(_)));
        if !has_join {
            return operators.to_vec();
        }

        // Fallback: just remove filter conditions that are join conditions
        let mut result: Vec<LogicalOperator> = Vec::new();
        let mut filters_to_remove: Vec<usize> = Vec::new();

        for (i, op) in operators.iter().enumerate() {
            if let LogicalOperator::Filter(f) = op
                && is_join_condition(&f.expression)
            {
                filters_to_remove.push(i);
            }
        }

        for (i, op) in operators.iter().enumerate() {
            if filters_to_remove.contains(&i) {
                continue;
            }
            result.push(op.clone());
        }

        result
    }
}

/// Check if an expression is an equality join condition between two variables.
pub fn is_join_condition(expr: &akar_parser::ast::Expression) -> bool {
    match expr {
        akar_parser::ast::Expression::BinaryOp(akar_parser::ast::BinaryOp::Equal, left, right) => {
            let left_var = extract_root_variable(left);
            let right_var = extract_root_variable(right);
            left_var.is_some() && right_var.is_some() && left_var != right_var
        }
        _ => false,
    }
}

/// Extract the root variable from an expression (e.g., `a.id` → `a`).
pub fn extract_root_variable(expr: &akar_parser::ast::Expression) -> Option<String> {
    match expr {
        akar_parser::ast::Expression::Variable(name) => Some(name.clone()),
        akar_parser::ast::Expression::PropertyAccess(obj, _) => extract_root_variable(obj),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_parser::ast::{BinaryOp, Constant, Expression};

    #[test]
    fn test_is_join_condition_valid() {
        // Direct variables: a = b
        let expr1 = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::Variable("a".into())),
            Box::new(Expression::Variable("b".into())),
        );
        assert!(is_join_condition(&expr1));

        // Property access: a.id = b.id
        let expr2 = Expression::BinaryOp(
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
        assert!(is_join_condition(&expr2));

        // Nested property access: a.user.id = b.account.id
        let expr3 = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("a".into())),
                    "user".into(),
                )),
                "id".into(),
            )),
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("b".into())),
                    "account".into(),
                )),
                "id".into(),
            )),
        );
        assert!(is_join_condition(&expr3));

        // Variable and property access: a = b.id
        let expr4 = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::Variable("a".into())),
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("b".into())),
                "id".into(),
            )),
        );
        assert!(is_join_condition(&expr4));
    }

    #[test]
    fn test_is_join_condition_invalid() {
        // Same root variable: a.id = a.id
        let expr1 = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())),
                "id".into(),
            )),
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())),
                "id".into(),
            )),
        );
        assert!(!is_join_condition(&expr1));

        // Same variable: a = a
        let expr2 = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::Variable("a".into())),
            Box::new(Expression::Variable("a".into())),
        );
        assert!(!is_join_condition(&expr2));

        // Variable vs constant: a.id = 10
        let expr3 = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())),
                "id".into(),
            )),
            Box::new(Expression::Constant(Constant::Integer(10))),
        );
        assert!(!is_join_condition(&expr3));

        // Non-equality binary operator: a.id > b.id
        let expr4 = Expression::BinaryOp(
            BinaryOp::GreaterThan,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())),
                "id".into(),
            )),
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("b".into())),
                "id".into(),
            )),
        );
        assert!(!is_join_condition(&expr4));

        // Binary operation other than comparison: a.id + b.id
        let expr5 = Expression::BinaryOp(
            BinaryOp::Add,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())),
                "id".into(),
            )),
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("b".into())),
                "id".into(),
            )),
        );
        assert!(!is_join_condition(&expr5));

        // Constant vs constant: 1 = 1
        let expr6 = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::Constant(Constant::Integer(1))),
            Box::new(Expression::Constant(Constant::Integer(1))),
        );
        assert!(!is_join_condition(&expr6));

        // Non-binary expression: constant
        let expr7 = Expression::Constant(Constant::Bool(true));
        assert!(!is_join_condition(&expr7));
    }

    #[test]
    fn test_extract_root_variable() {
        // Simple variable
        let v = Expression::Variable("x".into());
        assert_eq!(extract_root_variable(&v), Some("x".into()));

        // Single property access
        let p1 = Expression::PropertyAccess(Box::new(Expression::Variable("user".into())), "name".into());
        assert_eq!(extract_root_variable(&p1), Some("user".into()));

        // Deep property access
        let p2 = Expression::PropertyAccess(
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("org".into())),
                "department".into(),
            )),
            "name".into(),
        );
        assert_eq!(extract_root_variable(&p2), Some("org".into()));

        // Constant expression
        let c = Expression::Constant(Constant::String("hello".into()));
        assert_eq!(extract_root_variable(&c), None);

        // Binary op expression
        let bin = Expression::BinaryOp(
            BinaryOp::Add,
            Box::new(Expression::Variable("a".into())),
            Box::new(Expression::Variable("b".into())),
        );
        assert_eq!(extract_root_variable(&bin), None);
    }

    fn make_scan(name: &str) -> LogicalOperator {
        LogicalOperator::ScanNode(LogicalScanNode {
            predicate: None,
            table_name: name.into(),
            table_id: 0,
            alias: Some(name.into()),
            columns: vec!["id".into()],
            cardinality: 100,
            fts_query: None,
        })
    }

    fn make_equi_join_filter(left_var: &str, right_var: &str) -> LogicalOperator {
        LogicalOperator::Filter(LogicalFilter {
            expression: Expression::BinaryOp(
                BinaryOp::Equal,
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable(left_var.into())),
                    "id".into(),
                )),
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable(right_var.into())),
                    "id".into(),
                )),
            ),
            children: Vec::new(),
            cardinality: 0,
        })
    }

    fn make_non_equi_filter(var: &str, val: i64) -> LogicalOperator {
        LogicalOperator::Filter(LogicalFilter {
            expression: Expression::BinaryOp(
                BinaryOp::GreaterThan,
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable(var.into())),
                    "age".into(),
                )),
                Box::new(Expression::Constant(Constant::Integer(val))),
            ),
            children: Vec::new(),
            cardinality: 0,
        })
    }

    #[test]
    fn test_join_optimization_preserves_equi_join_filter_single_scan() {
        // In a single-scan pipeline without joins, `a.id = b.id` must be preserved.
        let plan = vec![make_scan("a"), make_equi_join_filter("a", "b")];
        let pass = JoinOptimization;
        let result = pass.apply(&plan);

        assert_eq!(result.len(), 2);
        assert!(matches!(result[0], LogicalOperator::ScanNode(_)));
        assert!(matches!(result[1], LogicalOperator::Filter(_)));
    }

    #[test]
    fn test_join_optimization_preserves_equi_join_filter_in_intersect_plan() {
        // In a plan without HashJoin/CrossProduct (e.g. WCOJ/Intersect),
        // equi-join filters must be kept.
        let intersect_op = LogicalOperator::Intersect(LogicalIntersect {
            num_build_sides: 1,
            build_key_exprs: vec![],
            left: Box::new(make_scan("a")),
            right: Box::new(make_scan("b")),
            cardinality: 0,
        });
        let plan = vec![intersect_op, make_equi_join_filter("a", "b")];
        let pass = JoinOptimization;
        let result = pass.apply(&plan);

        assert_eq!(result.len(), 2);
        assert!(matches!(result[0], LogicalOperator::Intersect(_)));
        assert!(matches!(result[1], LogicalOperator::Filter(_)));
    }

    #[test]
    fn test_join_optimization_fallback_removes_equi_join_filter_with_join() {
        // When a join (e.g. CrossProduct) is present and reorder_joins_dp_bushy
        // returns None (e.g. non-scan leaves), the fallback path drops
        // equi-join filters but keeps non-equi filters.
        let cp = LogicalOperator::CrossProduct(LogicalCrossProduct {
            left: Box::new(LogicalOperator::ExpressionsScan(LogicalExpressionsScan {
                expressions: vec!["a".into()],
                outer_accumulate_idx: None,
                cardinality: 10,
            })),
            right: Box::new(LogicalOperator::ExpressionsScan(LogicalExpressionsScan {
                expressions: vec!["b".into()],
                outer_accumulate_idx: None,
                cardinality: 10,
            })),
            cardinality: 100,
        });
        let equi_filter = make_equi_join_filter("a", "b");
        let non_equi_filter = make_non_equi_filter("a", 25);

        let plan = vec![cp, equi_filter, non_equi_filter];
        let pass = JoinOptimization;
        let result = pass.apply(&plan);

        assert_eq!(result.len(), 2);
        assert!(matches!(result[0], LogicalOperator::CrossProduct(_)));
        assert!(matches!(result[1], LogicalOperator::Filter(_)));
        if let LogicalOperator::Filter(ref f) = result[1] {
            assert!(!is_join_condition(&f.expression));
        }
    }
}
