// ========================================================================
// Pass 9: ART Range Scan Detection
// Detects patterns like `ScanNode + Filter(pk >= lower AND pk < upper)`
// and rewrites them to `ArtIndexRangeScan` when the table has an ART index.
//
// Safety invariants:
// - The WHOLE filter expression must be bounds on a single property; any
//   conjunct on another property or any non-range conjunct keeps the plan
//   unchanged (the rewrite must never drop a predicate).
// - A ScanNode that already carries a folded predicate is never rewritten.
// - Index existence cannot be checked here (the pass has no catalog access);
//   the runtime `PhysicalArtIndexRangeScan` errors if the table has no ART
//   index. This pass only fires on the rare `ScanNode + Filter` pattern that
//   survives FilterPushDown, so in practice the range predicate is on a PK
//   column that already has an ART index.
// ========================================================================

use crate::passes::OptimizationPass;
use akar_common::types::Value;
use akar_parser::ast::Expression;
use akar_planner::logical_operator::*;

pub struct ArtRangeScanDetection;

impl OptimizationPass for ArtRangeScanDetection {
    fn name(&self) -> &str {
        "art_range_scan_detection"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result = Vec::with_capacity(operators.len());
        let mut i = 0;

        while i < operators.len() {
            // Look for: ScanNode + Filter(range bounds on a single property)
            //
            // Only rewrites when the WHOLE filter is a set of bounds on the
            // SAME property (`a.id >= 10 AND a.id < 20`). Any conjunct that is
            // not a bound, or a bound on a different property, keeps the plan
            // unchanged — the rewrite must never drop predicates. A ScanNode
            // that already carries a folded predicate is also left alone so the
            // existing predicate is not discarded.
            if i + 1 < operators.len()
                && let (LogicalOperator::ScanNode(sn), LogicalOperator::Filter(f)) = (&operators[i], &operators[i + 1])
                && sn.predicate.is_none()
                && let Some(bounds) = extract_range_bounds(&f.expression)
            {
                result.push(LogicalOperator::ArtIndexRangeScan(LogicalArtIndexRangeScan {
                    table_name: sn.table_name.clone(),
                    table_id: sn.table_id,
                    alias: sn.alias.clone(),
                    lower_bound: bounds.lower,
                    upper_bound: bounds.upper,
                    lower_inclusive: bounds.lower_inclusive,
                    upper_inclusive: bounds.upper_inclusive,
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

/// Range bounds extracted for a single property, e.g. `a.id >= 10 AND a.id < 20`.
#[derive(Debug, Clone)]
struct RangeBounds {
    /// Property identity (`<root variable>.<property>`), used to ensure all
    /// conjuncts bound the same column.
    property: String,
    lower: Option<Value>,
    lower_inclusive: bool,
    upper: Option<Value>,
    upper_inclusive: bool,
}

/// Extract range bounds from a filter expression.
///
/// Recognizes patterns like:
/// - `pk >= lower AND pk < upper`
/// - `pk >= lower AND pk <= upper`
/// - `pk > lower AND pk < upper`
/// - `pk >= lower` (single bound)
/// - `pk < upper` (single bound)
/// - `pk = value` (both bounds)
///
/// Returns `None` unless every conjunct is a bound on the *same* property —
/// merging bounds across different columns (e.g. `a.age >= 10 AND a.id < 20`)
/// or dropping non-bound conjuncts would silently change the query result.
fn extract_range_bounds(expr: &Expression) -> Option<RangeBounds> {
    match expr {
        Expression::BinaryOp(akar_parser::ast::BinaryOp::And, left, right) => {
            let left_bounds = extract_range_bounds(left);
            let right_bounds = extract_range_bounds(right);
            match (left_bounds, right_bounds) {
                (Some(l), Some(r)) if l.property == r.property => merge_same_property(l, r),
                // Different property or a non-range conjunct: do not rewrite.
                _ => None,
            }
        }
        // Comparison operators
        Expression::BinaryOp(
            akar_parser::ast::BinaryOp::GreaterThanOrEqual
            | akar_parser::ast::BinaryOp::GreaterThan
            | akar_parser::ast::BinaryOp::LessThanOrEqual
            | akar_parser::ast::BinaryOp::LessThan
            | akar_parser::ast::BinaryOp::Equal,
            _,
            _,
        ) => extract_single_bound(expr),
        _ => None,
    }
}

/// Merge two bounds on the same property into one range.
///
/// Returns `None` when both sides carry a lower (or both an upper) bound —
/// a single bound cannot represent "must satisfy both", and picking one would
/// silently drop the other.
fn merge_same_property(left: RangeBounds, right: RangeBounds) -> Option<RangeBounds> {
    if left.lower.is_some() && right.lower.is_some() {
        return None;
    }
    if left.upper.is_some() && right.upper.is_some() {
        return None;
    }
    let lower = left.lower.clone().or(right.lower);
    let lower_inclusive = if left.lower.is_some() {
        left.lower_inclusive
    } else {
        right.lower_inclusive
    };
    let upper = left.upper.clone().or(right.upper);
    let upper_inclusive = if left.upper.is_some() {
        left.upper_inclusive
    } else {
        right.upper_inclusive
    };
    Some(RangeBounds {
        property: left.property,
        lower,
        lower_inclusive,
        upper,
        upper_inclusive,
    })
}

/// Extract a single bound from a comparison expression like `p.id >= 10`.
///
/// Handles both `property OP constant` and the reversed `constant OP property`
/// (in which case the bound direction is flipped).
fn extract_single_bound(expr: &Expression) -> Option<RangeBounds> {
    match expr {
        Expression::BinaryOp(op, left, right) => {
            // `is_reversed` is true when the constant is on the LEFT side,
            // i.e. `10 <= p.id` ⇔ `p.id >= 10`.
            let (property, const_val, is_reversed) = match (left.as_ref(), right.as_ref()) {
                // p.prop >= constant
                (Expression::PropertyAccess(obj, prop), constant @ Expression::Constant(_)) => match obj.as_ref() {
                    Expression::Variable(v) => (format!("{}.{}", v, prop), constant_to_value(constant), false),
                    _ => return None,
                },
                // constant <= p.prop (reversed)
                (constant @ Expression::Constant(_), Expression::PropertyAccess(obj, prop)) => match obj.as_ref() {
                    Expression::Variable(v) => (format!("{}.{}", v, prop), constant_to_value(constant), true),
                    _ => return None,
                },
                _ => return None,
            };

            let val = const_val?;
            if is_reversed {
                match op {
                    akar_parser::ast::BinaryOp::LessThanOrEqual => {
                        // const <= prop  ⇔  prop >= const (lower inclusive)
                        Some(RangeBounds {
                            property,
                            lower: Some(val),
                            lower_inclusive: true,
                            upper: None,
                            upper_inclusive: true,
                        })
                    }
                    akar_parser::ast::BinaryOp::LessThan => {
                        // const < prop  ⇔  prop > const (lower exclusive)
                        Some(RangeBounds {
                            property,
                            lower: Some(val),
                            lower_inclusive: false,
                            upper: None,
                            upper_inclusive: true,
                        })
                    }
                    akar_parser::ast::BinaryOp::GreaterThanOrEqual => {
                        // const >= prop  ⇔  prop <= const (upper inclusive)
                        Some(RangeBounds {
                            property,
                            lower: None,
                            lower_inclusive: true,
                            upper: Some(val),
                            upper_inclusive: true,
                        })
                    }
                    akar_parser::ast::BinaryOp::GreaterThan => {
                        // const > prop  ⇔  prop < const (upper exclusive)
                        Some(RangeBounds {
                            property,
                            lower: None,
                            lower_inclusive: true,
                            upper: Some(val),
                            upper_inclusive: false,
                        })
                    }
                    akar_parser::ast::BinaryOp::Equal => Some(RangeBounds {
                        property,
                        lower: Some(val.clone()),
                        lower_inclusive: true,
                        upper: Some(val),
                        upper_inclusive: true,
                    }),
                    _ => None,
                }
            } else {
                match op {
                    akar_parser::ast::BinaryOp::GreaterThanOrEqual => Some(RangeBounds {
                        property,
                        lower: Some(val),
                        lower_inclusive: true,
                        upper: None,
                        upper_inclusive: true,
                    }),
                    akar_parser::ast::BinaryOp::GreaterThan => Some(RangeBounds {
                        property,
                        lower: Some(val),
                        lower_inclusive: false,
                        upper: None,
                        upper_inclusive: true,
                    }),
                    akar_parser::ast::BinaryOp::LessThanOrEqual => Some(RangeBounds {
                        property,
                        lower: None,
                        lower_inclusive: true,
                        upper: Some(val),
                        upper_inclusive: true,
                    }),
                    akar_parser::ast::BinaryOp::LessThan => Some(RangeBounds {
                        property,
                        lower: None,
                        lower_inclusive: true,
                        upper: Some(val),
                        upper_inclusive: false,
                    }),
                    akar_parser::ast::BinaryOp::Equal => Some(RangeBounds {
                        property,
                        lower: Some(val.clone()),
                        lower_inclusive: true,
                        upper: Some(val),
                        upper_inclusive: true,
                    }),
                    _ => None,
                }
            }
        }
        _ => None,
    }
}

/// Convert a parser `Constant` to a runtime `Value`.
fn constant_to_value(c: &Expression) -> Option<akar_common::types::Value> {
    match c {
        Expression::Constant(akar_parser::ast::Constant::Integer(i)) => Some(akar_common::types::Value::Int64(*i)),
        Expression::Constant(akar_parser::ast::Constant::Float(f)) => Some(akar_common::types::Value::Double(*f)),
        Expression::Constant(akar_parser::ast::Constant::String(s)) => {
            Some(akar_common::types::Value::String(s.clone()))
        }
        Expression::Constant(akar_parser::ast::Constant::Bool(b)) => Some(akar_common::types::Value::Bool(*b)),
        Expression::Constant(akar_parser::ast::Constant::Null) => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_parser::ast::{BinaryOp, Constant};

    fn prop(name: &str) -> Expression {
        Expression::PropertyAccess(Box::new(Expression::Variable("a".into())), name.into())
    }

    fn int_const(v: i64) -> Expression {
        Expression::Constant(Constant::Integer(v))
    }

    fn cmp(op: BinaryOp, left: Expression, right: Expression) -> Expression {
        Expression::BinaryOp(op, Box::new(left), Box::new(right))
    }

    fn scan() -> LogicalOperator {
        LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("a".into()),
            columns: vec!["id".into(), "age".into(), "name".into()],
            cardinality: 100,
            fts_query: None,
            predicate: None,
        })
    }

    fn filter(expr: Expression) -> LogicalOperator {
        LogicalOperator::Filter(LogicalFilter {
            expression: expr,
            children: vec![],
            cardinality: 0,
        })
    }

    fn apply(expr: Expression) -> Vec<LogicalOperator> {
        ArtRangeScanDetection.apply(&[scan(), filter(expr)])
    }

    #[test]
    fn test_art_rewrites_same_property_range() {
        let e = cmp(
            BinaryOp::And,
            cmp(BinaryOp::GreaterThanOrEqual, prop("id"), int_const(10)),
            cmp(BinaryOp::LessThan, prop("id"), int_const(20)),
        );
        let result = apply(e);
        assert_eq!(result.len(), 1);
        assert!(matches!(
            &result[0],
            LogicalOperator::ArtIndexRangeScan(ars)
                if ars.lower_bound == Some(Value::Int64(10))
                    && ars.lower_inclusive
                    && ars.upper_bound == Some(Value::Int64(20))
                    && !ars.upper_inclusive
        ));
    }

    #[test]
    fn test_art_does_not_merge_different_properties() {
        // a.age >= 10 AND a.id < 20 — merging these would produce one range
        // scan on a mix of two columns and return wrong rows.
        let e = cmp(
            BinaryOp::And,
            cmp(BinaryOp::GreaterThanOrEqual, prop("age"), int_const(10)),
            cmp(BinaryOp::LessThan, prop("id"), int_const(20)),
        );
        let result = apply(e);
        assert_eq!(result.len(), 2, "different-property bounds must not be merged");
    }

    #[test]
    fn test_art_does_not_drop_non_range_conjunct() {
        // a.age >= 10 AND a.name = 'x' — the equality must not be dropped.
        let e = cmp(
            BinaryOp::And,
            cmp(BinaryOp::GreaterThanOrEqual, prop("age"), int_const(10)),
            cmp(
                BinaryOp::Equal,
                prop("name"),
                Expression::Constant(Constant::String("x".into())),
            ),
        );
        let result = apply(e);
        assert_eq!(result.len(), 2, "non-range conjunct must prevent the rewrite");
    }

    #[test]
    fn test_art_reversed_comparison_bound_direction() {
        // 10 <= a.id AND a.id < 20 → lower bound 10 (inclusive) + upper 20 (exclusive).
        let e = cmp(
            BinaryOp::And,
            cmp(BinaryOp::LessThanOrEqual, int_const(10), prop("id")),
            cmp(BinaryOp::LessThan, prop("id"), int_const(20)),
        );
        let result = apply(e);
        assert_eq!(result.len(), 1);
        if let LogicalOperator::ArtIndexRangeScan(ars) = &result[0] {
            assert_eq!(ars.lower_bound, Some(Value::Int64(10)));
            assert!(ars.lower_inclusive);
            assert_eq!(ars.upper_bound, Some(Value::Int64(20)));
            assert!(!ars.upper_inclusive);
        } else {
            panic!("expected ArtIndexRangeScan");
        }
    }

    #[test]
    fn test_art_skips_scan_with_existing_predicate() {
        // A scan that already carries a folded predicate must never be
        // rewritten, otherwise that predicate would be silently dropped.
        let plan = vec![
            LogicalOperator::ScanNode(LogicalScanNode {
                table_name: "Person".into(),
                table_id: 0,
                alias: Some("a".into()),
                columns: vec!["id".into()],
                cardinality: 100,
                fts_query: None,
                predicate: Some(cmp(BinaryOp::Equal, prop("id"), int_const(1))),
            }),
            filter(cmp(
                BinaryOp::And,
                cmp(BinaryOp::GreaterThanOrEqual, prop("id"), int_const(10)),
                cmp(BinaryOp::LessThan, prop("id"), int_const(20)),
            )),
        ];
        let result = ArtRangeScanDetection.apply(&plan);
        assert_eq!(result.len(), 2);
    }
}
