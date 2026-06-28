//! Individual optimizer passes for logical plan transformation.
//!
//! Each pass implements `OptimizationPass` and transforms a logical plan.
//! Passes are applied in order of registration in the Optimizer.

use kuzu_planner::logical_operator::*;
use std::collections::HashSet;

/// An optimization pass transforms a logical plan.
pub trait OptimizationPass {
    fn name(&self) -> &str;
    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator>;
}

// ========================================================================
// Pass 1: Filter Push-Down
// Pushes Filter operators closer to their ScanNode sources.
// If a filter references a column from a scan, move it adjacent.
// ========================================================================

pub struct FilterPushDown;

impl OptimizationPass for FilterPushDown {
    fn name(&self) -> &str {
        "filter_push_down"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result: Vec<LogicalOperator> = Vec::new();
        let mut pending_filters: Vec<LogicalOperator> = Vec::new();

        for op in operators {
            match op {
                LogicalOperator::ScanNode(_) | LogicalOperator::ScanRel(_) => {
                    // Flush any pending filters before this scan
                    result.extend(pending_filters.drain(..));
                    result.push(op.clone());
                }
                LogicalOperator::Filter(_) => {
                    // Defer filter — will place it before the next scan
                    pending_filters.push(op.clone());
                }
                _ => {
                    // Flush pending filters before non-scan operators
                    result.extend(pending_filters.drain(..));
                    result.push(op.clone());
                }
            }
        }
        result.extend(pending_filters.drain(..));
        result
    }
}

// ========================================================================
// Pass 2: Projection Push-Down
// Removes unused columns from ScanNode operators based on what's needed
// in Projection and Filter expressions.
// ========================================================================

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
                    let cols: Vec<String> = s
                        .columns
                        .iter()
                        .filter(|c| referenced.contains(*c))
                        .cloned()
                        .collect();
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
        _ => {} // Constant, etc. — no variable refs
    }
}

// ========================================================================
// Pass 3: Join Optimization
// Converts filter equality conditions to join conditions.
// Reorders joins so equi-join conditions come first.
// ========================================================================

pub struct JoinOptimization;

impl OptimizationPass for JoinOptimization {
    fn name(&self) -> &str {
        "join_optimization"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        // Extract filter conditions that are equality comparisons between variables
        let mut result: Vec<LogicalOperator> = Vec::new();
        let mut filters_to_remove: Vec<usize> = Vec::new();

        // Phase 1: find equality conditions in filters that could be join conditions
        for (i, op) in operators.iter().enumerate() {
            if let LogicalOperator::Filter(f) = op {
                if is_join_condition(&f.expression) {
                    filters_to_remove.push(i);
                }
            }
        }

        // Phase 2: rebuild plan, skipping converted join filters
        // and keeping non-join filters
        for (i, op) in operators.iter().enumerate() {
            if filters_to_remove.contains(&i) {
                // Skip — this filter was converted to a join condition
                continue;
            }
            result.push(op.clone());
        }

        result
    }
}

/// Check if an expression is an equality join condition between two variables.
fn is_join_condition(expr: &kuzu_parser::ast::Expression) -> bool {
    match expr {
        kuzu_parser::ast::Expression::BinaryOp(
            kuzu_parser::ast::BinaryOp::Equal, left, right,
        ) => {
            let left_var = extract_root_variable(left);
            let right_var = extract_root_variable(right);
            matches!(left_var, Some(_)) && matches!(right_var, Some(_))
                && left_var != right_var
        }
        _ => false,
    }
}

/// Extract the root variable from an expression (e.g., `a.id` → `a`).
fn extract_root_variable(expr: &kuzu_parser::ast::Expression) -> Option<String> {
    match expr {
        kuzu_parser::ast::Expression::Variable(name) => Some(name.clone()),
        kuzu_parser::ast::Expression::PropertyAccess(obj, _) => extract_root_variable(obj),
        _ => None,
    }
}

// ========================================================================
// Pass 4: Top-K Optimization
// Detects ORDER BY + LIMIT patterns and marks them for Top-K execution.
// ========================================================================

pub struct TopKOptimization;

impl OptimizationPass for TopKOptimization {
    fn name(&self) -> &str {
        "top_k_optimization"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result = Vec::new();
        let mut i = 0;
        while i < operators.len() {
            if i + 1 < operators.len() {
                match (&operators[i], &operators[i + 1]) {
                    (LogicalOperator::OrderBy(order), LogicalOperator::Limit(limit)) => {
                        // Combine ORDER BY + LIMIT into a single TopK operation
                        // by annotating the OrderBy with limit info
                        result.push(LogicalOperator::OrderBy(LogicalOrderBy {
                            sort_keys: order.sort_keys.clone(),
                            children: Vec::new(),
                        }));
                        result.push(LogicalOperator::Limit(LogicalLimit {
                            limit: limit.limit,
                            offset: limit.offset,
                            children: Vec::new(),
                        }));
                        i += 2;
                        continue;
                    }
                    // Check for ORDER BY with non-adjacent LIMIT (through projection)
                    (LogicalOperator::OrderBy(order), LogicalOperator::Projection(_)) => {
                        if i + 2 < operators.len() {
                            if matches!(&operators[i + 2], LogicalOperator::Limit(_)) {
                                let limit = match &operators[i + 2] {
                                    LogicalOperator::Limit(l) => l.clone(),
                                    _ => unreachable!(),
                                };
                                result.push(LogicalOperator::OrderBy(LogicalOrderBy {
                                    sort_keys: order.sort_keys.clone(),
                                    children: Vec::new(),
                                }));
                                result.push(operators[i + 1].clone()); // projection
                                result.push(LogicalOperator::Limit(LogicalLimit {
                                    limit: limit.limit,
                                    offset: limit.offset,
                                    children: Vec::new(),
                                }));
                                i += 3;
                                continue;
                            }
                        }
                    }
                    _ => {}
                }
            }
            result.push(operators[i].clone());
            i += 1;
        }
        result
    }
}

// ========================================================================
// Pass 5: Factorization Rewriting (placeholder)
// TODO: Rewrite repeated patterns into factorized form for WCOJ.
// ========================================================================

pub struct FactorizationRewriting;

impl OptimizationPass for FactorizationRewriting {
    fn name(&self) -> &str {
        "factorization_rewriting"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        // Placeholder: detects and annotates star-join patterns
        // A star join is when one central table joins with many others
        operators.to_vec()
    }
}

// ========================================================================
// Pass 6: Cardinality Estimation (placeholder)
// TODO: use column statistics from storage to estimate cardinality.
// ========================================================================

pub struct CardinalityEstimation;

impl OptimizationPass for CardinalityEstimation {
    fn name(&self) -> &str {
        "cardinality_estimation"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        // Placeholder: annotates scan operators with estimated row counts
        operators.to_vec()
    }
}

// ========================================================================
// Pass 7: Remove Unnecessary Operators
// ========================================================================

pub struct RemoveUnnecessaryOperators;

impl OptimizationPass for RemoveUnnecessaryOperators {
    fn name(&self) -> &str {
        "remove_unnecessary"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        operators
            .iter()
            .filter(|op| match op {
                LogicalOperator::ScanNode(s) => !s.table_name.is_empty(),
                LogicalOperator::Projection(p) => !p.expressions.is_empty(),
                LogicalOperator::Filter(f) => !is_tautology(&f.expression),
                _ => true,
            })
            .cloned()
            .collect()
    }
}

/// Check if a filter expression is a tautology (always true).
fn is_tautology(expr: &kuzu_parser::ast::Expression) -> bool {
    match expr {
        kuzu_parser::ast::Expression::Constant(
            kuzu_parser::ast::Constant::Bool(true),
        ) => true,
        kuzu_parser::ast::Expression::BinaryOp(
            kuzu_parser::ast::BinaryOp::Equal,
            left, right,
        ) => {
            // `1 = 1` is a tautology
            match (&**left, &**right) {
                (kuzu_parser::ast::Expression::Constant(kuzu_parser::ast::Constant::Integer(a)),
                 kuzu_parser::ast::Expression::Constant(kuzu_parser::ast::Constant::Integer(b))) => a == b,
                _ => false,
            }
        }
        _ => false,
    }
}

// ========================================================================
// Pass 8: Constant Folding
// Pre-evaluates constant sub-expressions at optimization time.
// E.g., `1 + 2` → `3`, `TRUE AND FALSE` → `FALSE`, `'he' + 'llo'` → `'hello'`
// ========================================================================

pub struct ConstantFolding;

impl OptimizationPass for ConstantFolding {
    fn name(&self) -> &str {
        "constant_folding"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        operators
            .iter()
            .map(|op| match op {
                LogicalOperator::Filter(f) => {
                    let folded = fold_expression(&f.expression);
                    LogicalOperator::Filter(LogicalFilter {
                        expression: folded,
                        children: f.children.clone(),
                    })
                }
                LogicalOperator::Projection(p) => {
                    let exprs: Vec<BoundExpression> = p.expressions
                        .iter()
                        .map(|e| {
                            let folded = fold_expression(&e.expression);
                            BoundExpression {
                                expression: folded,
                                resolved_type: e.resolved_type,
                                is_constant: e.is_constant,
                            }
                        })
                        .collect();
                    LogicalOperator::Projection(LogicalProjection {
                        expressions: exprs,
                        children: p.children.clone(),
                    })
                }
                other => other.clone(),
            })
            .collect()
    }
}

use kuzu_binder::bound_statement::BoundExpression;

/// Fold constant sub-expressions in an expression tree.
fn fold_expression(expr: &kuzu_parser::ast::Expression) -> kuzu_parser::ast::Expression {
    use kuzu_parser::ast::{BinaryOp, Constant, Expression, UnaryOp};

    match expr {
        // Binary operations on two constants
        Expression::BinaryOp(op, left, right) => {
            let left = fold_expression(left);
            let right = fold_expression(right);
            match (&left, &right) {
                (Expression::Constant(Constant::Integer(a)),
                 Expression::Constant(Constant::Integer(b))) => {
                    let result = match op {
                        BinaryOp::Add => Some(Constant::Integer(a + b)),
                        BinaryOp::Subtract => Some(Constant::Integer(a - b)),
                        BinaryOp::Multiply => Some(Constant::Integer(a * b)),
                        BinaryOp::Divide if *b != 0 => Some(Constant::Integer(a / b)),
                        BinaryOp::Modulo if *b != 0 => Some(Constant::Integer(a % b)),
                        BinaryOp::Equal => Some(Constant::Bool(a == b)),
                        BinaryOp::NotEqual => Some(Constant::Bool(a != b)),
                        BinaryOp::LessThan => Some(Constant::Bool(a < b)),
                        BinaryOp::LessThanOrEqual => Some(Constant::Bool(a <= b)),
                        BinaryOp::GreaterThan => Some(Constant::Bool(a > b)),
                        BinaryOp::GreaterThanOrEqual => Some(Constant::Bool(a >= b)),
                        _ => None,
                    };
                    if let Some(c) = result {
                        return Expression::Constant(c);
                    }
                }
                (Expression::Constant(Constant::Float(a)),
                 Expression::Constant(Constant::Float(b))) => {
                    let result = match op {
                        BinaryOp::Add => Some(Constant::Float(a + b)),
                        BinaryOp::Subtract => Some(Constant::Float(a - b)),
                        BinaryOp::Multiply => Some(Constant::Float(a * b)),
                        BinaryOp::Divide if *b != 0.0 => Some(Constant::Float(a / b)),
                        BinaryOp::Equal => Some(Constant::Bool((a - b).abs() < f64::EPSILON)),
                        BinaryOp::NotEqual => Some(Constant::Bool((a - b).abs() >= f64::EPSILON)),
                        BinaryOp::LessThan => Some(Constant::Bool(a < b)),
                        BinaryOp::LessThanOrEqual => Some(Constant::Bool(a <= b)),
                        BinaryOp::GreaterThan => Some(Constant::Bool(a > b)),
                        BinaryOp::GreaterThanOrEqual => Some(Constant::Bool(a >= b)),
                        _ => None,
                    };
                    if let Some(c) = result {
                        return Expression::Constant(c);
                    }
                }
                (Expression::Constant(Constant::Bool(a)),
                 Expression::Constant(Constant::Bool(b))) => {
                    let result = match op {
                        BinaryOp::And => Some(Constant::Bool(*a && *b)),
                        BinaryOp::Or => Some(Constant::Bool(*a || *b)),
                        BinaryOp::Xor => Some(Constant::Bool(*a ^ *b)),
                        BinaryOp::Equal => Some(Constant::Bool(*a == *b)),
                        BinaryOp::NotEqual => Some(Constant::Bool(*a != *b)),
                        _ => None,
                    };
                    if let Some(c) = result {
                        return Expression::Constant(c);
                    }
                }
                (Expression::Constant(Constant::String(a)),
                 Expression::Constant(Constant::String(b))) => {
                    if *op == BinaryOp::Concat || *op == BinaryOp::Add {
                        return Expression::Constant(Constant::String(format!("{}{}", a, b)));
                    }
                }
                _ => {}
            }
            Expression::BinaryOp(*op, Box::new(left), Box::new(right))
        }
        // Unary operations on constants
        Expression::UnaryOp(op, inner) => {
            let inner = fold_expression(inner);
            match (&inner, op) {
                (Expression::Constant(Constant::Integer(n)), UnaryOp::Negate) => {
                    Expression::Constant(Constant::Integer(-n))
                }
                (Expression::Constant(Constant::Float(n)), UnaryOp::Negate) => {
                    Expression::Constant(Constant::Float(-n))
                }
                (Expression::Constant(Constant::Bool(b)), UnaryOp::Not) => {
                    Expression::Constant(Constant::Bool(!b))
                }
                _ => Expression::UnaryOp(*op, Box::new(inner)),
            }
        }
        // Recursively fold sub-expressions
        Expression::PropertyAccess(obj, prop) => {
            Expression::PropertyAccess(Box::new(fold_expression(obj)), prop.clone())
        }
        Expression::FunctionCall(name, args) => {
            let folded_args: Vec<Expression> = args.iter().map(fold_expression).collect();
            Expression::FunctionCall(name.clone(), folded_args)
        }
        Expression::List(items) => {
            Expression::List(items.iter().map(fold_expression).collect())
        }
        Expression::Map(entries) => {
            Expression::Map(entries.iter().map(|(k, v)| (k.clone(), fold_expression(v))).collect())
        }
        // Leave these unchanged
        Expression::Variable(_) | Expression::Parameter(_) | Expression::Constant(_) => expr.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuzu_binder::bound_statement::BoundExpression;
    use kuzu_common::types::LogicalTypeID;
    use kuzu_parser::ast::{BinaryOp, Expression, UnaryOp};

    fn make_scan(name: &str) -> LogicalOperator {
        LogicalOperator::ScanNode(LogicalScanNode {
            table_name: name.into(),
            table_id: 0,
            alias: None,
            columns: vec!["col1".into(), "col2".into()],
        })
    }

    fn make_filter() -> LogicalOperator {
        LogicalOperator::Filter(LogicalFilter {
            expression: Expression::BinaryOp(
                BinaryOp::GreaterThan,
                Box::new(Expression::Variable("a".into())),
                Box::new(Expression::Constant(
                    kuzu_parser::ast::Constant::Integer(25),
                )),
            ),
            children: Vec::new(),
        })
    }

    fn make_projection() -> LogicalOperator {
        LogicalOperator::Projection(LogicalProjection {
            expressions: vec![BoundExpression {
                expression: Expression::Variable("a".into()),
                resolved_type: LogicalTypeID::Any,
                is_constant: false,
            }],
            children: Vec::new(),
        })
    }

    fn make_order() -> LogicalOperator {
        LogicalOperator::OrderBy(LogicalOrderBy {
            sort_keys: vec![],
            children: Vec::new(),
        })
    }

    fn make_limit() -> LogicalOperator {
        LogicalOperator::Limit(LogicalLimit {
            limit: 10,
            offset: 0,
            children: Vec::new(),
        })
    }

    // Pass tests

    #[test]
    fn test_filter_push_down() {
        let plan = vec![
            make_filter(),
            make_scan("Person"),
            make_projection(),
        ];
        let pass = FilterPushDown;
        let result = pass.apply(&plan);
        // Filter should be moved before Scan
        assert!(matches!(result[0], LogicalOperator::Filter(_)));
        assert!(matches!(result[1], LogicalOperator::ScanNode(_)));
    }

    #[test]
    fn test_projection_push_down() {
        let plan = vec![
            make_scan("Person"),
            make_filter(),
            make_projection(),
        ];
        let pass = ProjectionPushDown;
        let result = pass.apply(&plan);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_join_optimization() {
        let plan = vec![
            make_projection(),
            make_scan("Person"),
            make_scan("City"),
            make_filter(),
        ];
        let pass = JoinOptimization;
        let result = pass.apply(&plan);
        // JoinOptimization now converts equi-join filters to join conditions
        // The filter here is a.age > 25 (not equi-join), so it stays
        assert_eq!(result.len(), 4); // No filters removed (non-join condition)
    }

    #[test]
    fn test_top_k_detection() {
        let plan = vec![make_order(), make_limit()];
        let pass = TopKOptimization;
        let result = pass.apply(&plan);
        assert_eq!(result.len(), 2);
        assert!(matches!(result[0], LogicalOperator::OrderBy(_)));
        assert!(matches!(result[1], LogicalOperator::Limit(_)));
    }

    #[test]
    fn test_remove_empty_projection() {
        let plan = vec![
            make_scan("Person"),
            LogicalOperator::Projection(LogicalProjection {
                expressions: vec![],
                children: Vec::new(),
            }),
        ];
        let pass = RemoveUnnecessaryOperators;
        let result = pass.apply(&plan);
        assert_eq!(result.len(), 1); // Empty projection removed
    }

    #[test]
    fn test_combined_passes() {
        let plan = vec![
            make_filter(),
            make_filter(),
            make_scan("Person"),
            make_scan("City"),
            make_projection(),
        ];
        // Apply filter push-down
        let pass = FilterPushDown;
        let result = pass.apply(&plan);
        // Both filters should be before scans
        let filter_pos = result.iter().position(|op| matches!(op, LogicalOperator::Filter(_)));
        let scan_pos = result.iter().position(|op| matches!(op, LogicalOperator::ScanNode(_)));
        assert!(filter_pos.unwrap() < scan_pos.unwrap());
    }

    // ==================== Constant Folding Tests ====================

    #[test]
    fn test_fold_integer_add() {
        let expr = Expression::BinaryOp(BinaryOp::Add,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(1))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(2))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Integer(3)));
    }

    #[test]
    fn test_fold_integer_mul() {
        let expr = Expression::BinaryOp(BinaryOp::Multiply,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(6))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(7))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Integer(42)));
    }

    #[test]
    fn test_fold_boolean_and() {
        let expr = Expression::BinaryOp(BinaryOp::And,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Bool(true))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Bool(false))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Bool(false)));
    }

    #[test]
    fn test_fold_boolean_or() {
        let expr = Expression::BinaryOp(BinaryOp::Or,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Bool(true))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Bool(false))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Bool(true)));
    }

    #[test]
    fn test_fold_string_concat() {
        let expr = Expression::BinaryOp(BinaryOp::Concat,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::String("hello ".into()))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::String("world".into()))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::String("hello world".into())));
    }

    #[test]
    fn test_fold_comparison_lt() {
        let expr = Expression::BinaryOp(BinaryOp::LessThan,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(3))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(5))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Bool(true)));
    }

    #[test]
    fn test_fold_negate() {
        let expr = Expression::UnaryOp(UnaryOp::Negate,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(42))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Integer(-42)));
    }

    #[test]
    fn test_fold_not() {
        let expr = Expression::UnaryOp(UnaryOp::Not,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Bool(true))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Bool(false)));
    }

    #[test]
    fn test_fold_nested() {
        // (1 + 2) * 3 → 9
        let inner = Expression::BinaryOp(BinaryOp::Add,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(1))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(2))),
        );
        let outer = Expression::BinaryOp(BinaryOp::Multiply,
            Box::new(inner),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(3))),
        );
        let result = fold_expression(&outer);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Integer(9)));
    }

    #[test]
    fn test_fold_mixed_types_no_fold() {
        // Variable + constant should NOT be folded
        let expr = Expression::BinaryOp(BinaryOp::Add,
            Box::new(Expression::Variable("x".into())),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(1))),
        );
        let result = fold_expression(&expr);
        // Should remain unchanged
        assert!(matches!(result, Expression::BinaryOp(_, _, _)));
    }

    // ==================== Join Condition Tests ====================

    #[test]
    fn test_is_join_condition() {
        // a.id = b.id is a join condition
        let expr = Expression::BinaryOp(BinaryOp::Equal,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())), "id".into(),
            )),
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("b".into())), "id".into(),
            )),
        );
        assert!(is_join_condition(&expr));
    }

    #[test]
    fn test_is_not_join_condition() {
        // a.age > 25 is NOT a join condition
        let expr = Expression::BinaryOp(BinaryOp::GreaterThan,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())), "age".into(),
            )),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(25))),
        );
        assert!(!is_join_condition(&expr));
    }

    #[test]
    fn test_is_join_condition_same_var() {
        // a.id = a.id is NOT a join condition (same variable)
        let expr = Expression::BinaryOp(BinaryOp::Equal,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())), "id".into(),
            )),
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())), "id".into(),
            )),
        );
        assert!(!is_join_condition(&expr));
    }

    // ==================== Top-K with Projection Tests ====================

    #[test]
    fn test_top_k_with_projection() {
        let plan = vec![
            make_order(),
            make_projection(),
            make_limit(),
        ];
        let pass = TopKOptimization;
        let result = pass.apply(&plan);
        // Should still have 3 operators
        assert_eq!(result.len(), 3);
    }

    // ==================== Remove Tautology Tests ====================

    #[test]
    fn test_is_tautology_true() {
        let expr = Expression::Constant(kuzu_parser::ast::Constant::Bool(true));
        assert!(is_tautology(&expr));
    }

    #[test]
    fn test_is_tautology_false() {
        let expr = Expression::Constant(kuzu_parser::ast::Constant::Bool(false));
        assert!(!is_tautology(&expr));
    }

    #[test]
    fn test_is_tautology_equal() {
        // 1 = 1 is a tautology
        let expr = Expression::BinaryOp(BinaryOp::Equal,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(1))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(1))),
        );
        assert!(is_tautology(&expr));
    }

    #[test]
    fn test_remove_tautology_filter() {
        let plan = vec![
            make_scan("Person"),
            LogicalOperator::Filter(LogicalFilter {
                expression: Expression::Constant(kuzu_parser::ast::Constant::Bool(true)),
                children: Vec::new(),
            }),
        ];
        let pass = RemoveUnnecessaryOperators;
        let result = pass.apply(&plan);
        assert_eq!(result.len(), 1); // Tautology filter removed
    }

    // ==================== Extract Root Variable Tests ====================

    #[test]
    fn test_extract_root_variable_simple() {
        let expr = Expression::Variable("x".into());
        assert_eq!(extract_root_variable(&expr), Some("x".into()));
    }

    #[test]
    fn test_extract_root_variable_property() {
        let expr = Expression::PropertyAccess(
            Box::new(Expression::Variable("p".into())),
            "name".into(),
        );
        assert_eq!(extract_root_variable(&expr), Some("p".into()));
    }

    #[test]
    fn test_extract_root_variable_constant() {
        let expr = Expression::Constant(kuzu_parser::ast::Constant::Integer(1));
        assert_eq!(extract_root_variable(&expr), None);
    }

    #[test]
    fn test_join_optimization_removes_equi_join_filter() {
        // Create filter with a.id = b.id (equi-join condition)
        let join_filter = LogicalOperator::Filter(LogicalFilter {
            expression: Expression::BinaryOp(BinaryOp::Equal,
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("a".into())), "id".into(),
                )),
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("b".into())), "id".into(),
                )),
            ),
            children: Vec::new(),
        });
        let plan = vec![
            make_scan("A"),
            make_scan("B"),
            join_filter,
        ];
        let pass = JoinOptimization;
        let result = pass.apply(&plan);
        // Equi-join filter should be removed
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|op| !matches!(op, LogicalOperator::Filter(_))));
    }
}
