// ========================================================================
// Pass 9: ART Range Scan Detection
// Detects patterns like `ScanNode + Filter(pk >= lower AND pk < upper)`
// and rewrites them to `ArtIndexRangeScan` when the table has an ART index.
// ========================================================================

use crate::passes::OptimizationPass;
use kuzu_parser::ast::Expression;
use kuzu_planner::logical_operator::*;

pub struct ArtRangeScanDetection;

impl OptimizationPass for ArtRangeScanDetection {
    fn name(&self) -> &str {
        "art_range_scan_detection"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result = Vec::with_capacity(operators.len());
        let mut i = 0;

        while i < operators.len() {
            // Look for: ScanNode + Filter(comparison on PK column)
            if i + 1 < operators.len()
                && let (LogicalOperator::ScanNode(sn), LogicalOperator::Filter(f)) = (&operators[i], &operators[i + 1])
                && let Some((lower, lower_inc, upper, upper_inc)) = extract_range_bounds(&f.expression)
            {
                result.push(LogicalOperator::ArtIndexRangeScan(LogicalArtIndexRangeScan {
                    table_name: sn.table_name.clone(),
                    table_id: sn.table_id,
                    alias: sn.alias.clone(),
                    lower_bound: lower,
                    upper_bound: upper,
                    lower_inclusive: lower_inc,
                    upper_inclusive: upper_inc,
                    cardinality: sn.cardinality.max(1),
                }));
                i += 2;
                continue;
            }

            result.push(operators[i].clone());
            i += 1;
        }

        result
    }
}

/// Extract range bounds from a filter expression.
///
/// Recognizes patterns like:
/// - `pk >= lower AND pk < upper`
/// - `pk >= lower AND pk <= upper`
/// - `pk > lower AND pk < upper`
/// - `pk >= lower` (single bound)
/// - `pk < upper` (single bound)
///
/// Returns `(lower, lower_inclusive, upper, upper_inclusive)`.
fn extract_range_bounds(
    expr: &Expression,
) -> Option<(
    Option<kuzu_common::types::Value>,
    bool,
    Option<kuzu_common::types::Value>,
    bool,
)> {
    match expr {
        Expression::BinaryOp(op, left, right) => {
            match op {
                kuzu_parser::ast::BinaryOp::And => {
                    // Recursively extract from both sides
                    let left_bounds = extract_range_bounds(left);
                    let right_bounds = extract_range_bounds(right);
                    match (left_bounds, right_bounds) {
                        (Some((l1, li1, u1, ui1)), Some((l2, li2, u2, ui2))) => {
                            // Merge bounds: use the tighter lower and upper from both sides
                            let lower = l1.clone().or(l2);
                            let lower_inc = if l1.is_some() { li1 } else { li2 };
                            let upper = u1.clone().or(u2);
                            let upper_inc = if u1.is_some() { ui1 } else { ui2 };
                            Some((lower, lower_inc, upper, upper_inc))
                        }
                        (Some(bounds), None) | (None, Some(bounds)) => Some(bounds),
                        _ => None,
                    }
                }
                // Comparison operators
                kuzu_parser::ast::BinaryOp::GreaterThanOrEqual
                | kuzu_parser::ast::BinaryOp::GreaterThan
                | kuzu_parser::ast::BinaryOp::LessThanOrEqual
                | kuzu_parser::ast::BinaryOp::LessThan
                | kuzu_parser::ast::BinaryOp::Equal => {
                    // Expect `property_access OP constant` or `constant OP property_access`
                    extract_single_bound(expr)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Extract a single bound from a comparison expression like `p.id >= 10`.
fn extract_single_bound(
    expr: &Expression,
) -> Option<(
    Option<kuzu_common::types::Value>,
    bool,
    Option<kuzu_common::types::Value>,
    bool,
)> {
    match expr {
        Expression::BinaryOp(op, left, right) => {
            let (_prop_expr, const_val) = match (left.as_ref(), right.as_ref()) {
                // p.prop >= constant
                (Expression::PropertyAccess(obj, prop), constant @ Expression::Constant(_))
                    if matches!(obj.as_ref(), Expression::Variable(_)) =>
                {
                    (prop.clone(), constant_to_value(constant))
                }
                // constant <= p.prop (reversed)
                (constant @ Expression::Constant(_), Expression::PropertyAccess(obj, prop))
                    if matches!(obj.as_ref(), Expression::Variable(_)) =>
                {
                    (prop.clone(), constant_to_value(constant))
                }
                _ => return None,
            };

            let val = const_val?;
            match op {
                kuzu_parser::ast::BinaryOp::GreaterThanOrEqual => {
                    Some((Some(val), true, None, true)) // lower inclusive
                }
                kuzu_parser::ast::BinaryOp::GreaterThan => {
                    Some((Some(val), false, None, true)) // lower exclusive
                }
                kuzu_parser::ast::BinaryOp::LessThanOrEqual => {
                    Some((None, true, Some(val), true)) // upper inclusive
                }
                kuzu_parser::ast::BinaryOp::LessThan => {
                    Some((None, true, Some(val), false)) // upper exclusive
                }
                kuzu_parser::ast::BinaryOp::Equal => {
                    // Equality: treat as both lower and upper bound
                    Some((Some(val.clone()), true, Some(val), true))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Convert a parser `Constant` to a runtime `Value`.
fn constant_to_value(c: &Expression) -> Option<kuzu_common::types::Value> {
    match c {
        Expression::Constant(kuzu_parser::ast::Constant::Integer(i)) => Some(kuzu_common::types::Value::Int64(*i)),
        Expression::Constant(kuzu_parser::ast::Constant::Float(f)) => Some(kuzu_common::types::Value::Double(*f)),
        Expression::Constant(kuzu_parser::ast::Constant::String(s)) => {
            Some(kuzu_common::types::Value::String(s.clone()))
        }
        Expression::Constant(kuzu_parser::ast::Constant::Bool(b)) => Some(kuzu_common::types::Value::Bool(*b)),
        Expression::Constant(kuzu_parser::ast::Constant::Null) => None,
        _ => None,
    }
}
