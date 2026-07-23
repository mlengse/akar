// ========================================================================
// Pass 1: Filter Push-Down
// Pushes Filter operators closer to their ScanNode sources.
// If a filter references a column from a scan, move it adjacent.
// ========================================================================

use crate::passes::OptimizationPass;
use akar_parser::ast::Expression;
use akar_planner::logical_operator::*;

pub struct FilterPushDown;

impl FilterPushDown {
    fn get_variables(expr: &Expression) -> Vec<String> {
        let mut vars = Vec::new();
        match expr {
            Expression::Variable(v) => vars.push(v.clone()),
            Expression::PropertyAccess(e, _) => vars.extend(Self::get_variables(e)),
            Expression::FunctionCall(_, args) => {
                for arg in args {
                    vars.extend(Self::get_variables(arg));
                }
            }
            Expression::BinaryOp(_, left, right) => {
                vars.extend(Self::get_variables(left));
                vars.extend(Self::get_variables(right));
            }
            Expression::UnaryOp(_, e) => vars.extend(Self::get_variables(e)),
            Expression::List(items) => {
                for item in items {
                    vars.extend(Self::get_variables(item));
                }
            }
            _ => {}
        }
        vars
    }
}

impl OptimizationPass for FilterPushDown {
    fn name(&self) -> &str {
        "filter_push_down"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result: Vec<LogicalOperator> = Vec::new();
        let mut pending_filters: Vec<LogicalOperator> = Vec::new();
        let mut current_scan: Option<LogicalScanNode> = None;

        for op in operators {
            match op {
                LogicalOperator::ScanNode(s) => {
                    // Flush any pending filters and the previous scan
                    if let Some(prev_scan) = current_scan.take() {
                        result.push(LogicalOperator::ScanNode(prev_scan));
                    }
                    result.append(&mut pending_filters);
                    current_scan = Some(s.clone());
                }
                LogicalOperator::Filter(f) => {
                    if let Some(ref mut scan) = current_scan {
                        let vars = Self::get_variables(&f.expression);
                        let scan_alias = scan.alias.as_deref().unwrap_or("");
                        let matches_scan = !vars.is_empty() && vars.iter().all(|v| v == scan_alias);

                        if matches_scan {
                            // Fold filter into scan predicate
                            if let Some(ref existing) = scan.predicate {
                                // Combine with AND
                                scan.predicate = Some(Expression::BinaryOp(
                                    akar_parser::ast::BinaryOp::And,
                                    Box::new(existing.clone()),
                                    Box::new(f.expression.clone()),
                                ));
                            } else {
                                scan.predicate = Some(f.expression.clone());
                            }
                            continue;
                        }
                    }

                    // If not folded into scan
                    if let Some(prev_scan) = current_scan.take() {
                        result.push(LogicalOperator::ScanNode(prev_scan));
                    }
                    pending_filters.push(op.clone());
                }
                LogicalOperator::ScanRel(_) => {
                    if let Some(prev_scan) = current_scan.take() {
                        result.push(LogicalOperator::ScanNode(prev_scan));
                    }
                    result.append(&mut pending_filters);
                    result.push(op.clone());
                }
                _ => {
                    // Flush pending filters before non-scan operators
                    if let Some(prev_scan) = current_scan.take() {
                        result.push(LogicalOperator::ScanNode(prev_scan));
                    }
                    result.append(&mut pending_filters);
                    result.push(op.clone());
                }
            }
        }

        if let Some(prev_scan) = current_scan.take() {
            result.push(LogicalOperator::ScanNode(prev_scan));
        }
        result.append(&mut pending_filters);
        result
    }
}
