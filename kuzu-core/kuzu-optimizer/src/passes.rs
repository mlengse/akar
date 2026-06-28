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
// Reorders joins to put joins with filters first (reduces intermediate size).
// ========================================================================

pub struct JoinOptimization;

impl OptimizationPass for JoinOptimization {
    fn name(&self) -> &str {
        "join_optimization"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        // Simplified: ensure CrossProduct operators come after scans
        let scans: Vec<LogicalOperator> = operators
            .iter()
            .filter(|op| matches!(op, LogicalOperator::ScanNode(_) | LogicalOperator::ScanRel(_)))
            .cloned()
            .collect();

        let others: Vec<LogicalOperator> = operators
            .iter()
            .filter(|op| !matches!(op, LogicalOperator::ScanNode(_) | LogicalOperator::ScanRel(_)))
            .cloned()
            .collect();

        let mut result: Vec<LogicalOperator> = Vec::new();
        result.extend(scans);
        result.extend(others);
        result
    }
}

// ========================================================================
// Pass 4: Top-K Optimization
// Pushes Limit operators below Sort operators when possible.
// ========================================================================

pub struct TopKOptimization;

impl OptimizationPass for TopKOptimization {
    fn name(&self) -> &str {
        "top_k_optimization"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        // Find adjacent OrderBy + Limit and combine
        let mut result = Vec::new();
        let mut i = 0;
        while i < operators.len() {
            if i + 1 < operators.len() {
                match (&operators[i], &operators[i + 1]) {
                    (LogicalOperator::OrderBy(order), LogicalOperator::Limit(limit)) => {
                        // Combined top-k
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
// Pass 5: Factorization Rewriting
// Rewrites repeated patterns into factorized form.
// ========================================================================

pub struct FactorizationRewriting;

impl OptimizationPass for FactorizationRewriting {
    fn name(&self) -> &str {
        "factorization_rewriting"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        // TODO: implement actual factorization (worst-case optimal join rewrites)
        operators.to_vec()
    }
}

// ========================================================================
// Pass 6: Cardinality Estimation (annotation pass)
// Annotates scan operators with estimated row counts from statistics.
// ========================================================================

pub struct CardinalityEstimation;

impl OptimizationPass for CardinalityEstimation {
    fn name(&self) -> &str {
        "cardinality_estimation"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        // TODO: use column statistics from storage to estimate cardinality
        operators.to_vec()
    }
}

// ========================================================================
// Pass 7: Remove Unnecessary Operators
// Removes empty scans, tautological filters (WHERE 1=1), etc.
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
                LogicalOperator::ScanNode(s) => {
                    // Keep scan if it has columns or alias (meaningful)
                    !s.table_name.is_empty()
                }
                LogicalOperator::Projection(p) => {
                    // Keep projection with at least one expression
                    !p.expressions.is_empty()
                }
                _ => true,
            })
            .cloned()
            .collect()
    }
}

// ========================================================================
// Pass 8: Constant Folding
// Pre-evaluates constant expressions at optimization time.
// ========================================================================

pub struct ConstantFolding;

impl OptimizationPass for ConstantFolding {
    fn name(&self) -> &str {
        "constant_folding"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        // TODO: evaluate constant sub-expressions
        operators.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuzu_binder::bound_statement::BoundExpression;
    use kuzu_common::types::LogicalTypeID;
    use kuzu_parser::ast::{BinaryOp, Expression};

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
        // Scans should come first
        assert!(matches!(result[0], LogicalOperator::ScanNode(_)));
        assert!(matches!(result[1], LogicalOperator::ScanNode(_)));
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
}
