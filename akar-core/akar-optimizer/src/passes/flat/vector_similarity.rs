// ========================================================================
// Pass 8 (flat order): Vector Similarity Detection
// ========================================================================
//
// A `VectorSimilarityScan` output schema is `[<all table columns>, distance,
// _id]`. A query such as
//
//   MATCH (n:Memory)
//   WHERE cosine_similarity(n.embedding, [q]) > 0.8
//   RETURN n.id, n.content
//   ORDER BY cosine_similarity(n.embedding, [q]) DESC
//   LIMIT 5
//
// plans to a flat pipeline. FilterPushDown (pass 1) folds the single-variable
// cos filter into `ScanNode.predicate`, so by the time this pass runs the plan
// is:
//
//   [ScanNode(predicate = cos > thr), OrderBy(cos DESC), Projection, Limit(k)]
//
// This pass rewrites the scan node into
//
//   [VectorSimilarityScan(column, q, top_k=k), Filter(cos > thr)]
//
// The flat pipeline executes left-to-right, so the threshold Filter is emitted
// AFTER the scan so it consumes the scan's output rows. Everything else is
// preserved (no-op on the surrounding operators): the distance threshold stays
// (as the Filter after the scan), the RETURN projection stays, and the ORDER BY
// / LIMIT stay. This fixes the historical bug where the pass consumed the
// projection (destroying the RETURN schema), the ORDER BY and the LIMIT, and
// silently dropped the threshold.
//
// Safety invariants:
// - The ScanNode predicate must be EXACTLY `cos(col, [q]) > thr` (or `>=`); any
//   extra conjunct keeps the plan unchanged, so no predicate is ever dropped.
// - The ORDER BY must be a single sort key, DESC, on the identical
//   `cos(col, [q])` expression with the same query vector and column.
// - A following Projection and Limit(k) must be present; `top_k` is taken from
//   the LIMIT value.
// - Like `ArtRangeScanDetection`, this pass has no catalog access: the runtime
//   `PhysicalVectorSimilarityScan` resolves the index by `column_name`.
// ========================================================================

use crate::passes::OptimizationPass;
use akar_parser::ast::{BinaryOp, Constant, Expression};
use akar_planner::logical_operator::*;

pub struct VectorSimilarityDetection;

impl OptimizationPass for VectorSimilarityDetection {
    fn name(&self) -> &str {
        "vector_similarity_detection"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result: Vec<LogicalOperator> = Vec::with_capacity(operators.len());
        let mut i = 0;

        while i < operators.len() {
            // Canonical pattern, in flat execution order:
            // [ScanNode(cos(col,[q]) > thr), OrderBy(cos DESC), Projection, Limit(k)]
            //
            // Only the ScanNode is rewritten; OrderBy / Projection / Limit are
            // pushed through unchanged so the threshold, RETURN schema and
            // LIMIT are all preserved.
            if i + 3 < operators.len()
                && let LogicalOperator::ScanNode(sn) = &operators[i]
                && let LogicalOperator::OrderBy(ob) = &operators[i + 1]
                && matches!(&operators[i + 2], LogicalOperator::Projection(_))
                && let LogicalOperator::Limit(lim) = &operators[i + 3]
                && let Some((threshold_expr, column, query)) = extract_vector_threshold(&sn.predicate)
                && order_by_matches_cos(ob, &column, &query)
            {
                // Flat pipeline executes left-to-right, so the threshold Filter
                // must come AFTER the scan (it consumes the scan's output rows).
                // Order: VectorSimilarityScan → Filter(cos > thr) → OrderBy →
                // Projection → Limit.
                result.push(LogicalOperator::VectorSimilarityScan(LogicalVectorSimilarityScan {
                    table_name: sn.table_name.clone(),
                    column_name: column,
                    query_vector: query,
                    top_k: lim.limit.max(1),
                    alias: sn.alias.clone(),
                    cardinality: sn.cardinality.max(1),
                }));
                result.push(LogicalOperator::Filter(LogicalFilter {
                    expression: threshold_expr,
                    children: Vec::new(),
                    cardinality: 0,
                }));
                // Advance past the ScanNode only; OrderBy/Projection/Limit are
                // emitted on the next iterations.
                i += 1;
                continue;
            }

            result.push(operators[i].clone());
            i += 1;
        }

        result
    }
}

/// Extract the vector-similarity threshold from a scan predicate.
///
/// Returns `(predicate, column, query_vector)` when the predicate is exactly
/// `cosine_similarity(<var>.<col>, [q]) > thr` (or `>= thr`). Anything else —
/// a compound predicate, a non-numeric threshold, a different function name —
/// returns `None` so the plan is preserved verbatim.
fn extract_vector_threshold(predicate: &Option<Expression>) -> Option<(Expression, String, Vec<f64>)> {
    let expr = predicate.as_ref()?;
    let (column, query) = match expr {
        Expression::BinaryOp(BinaryOp::GreaterThan | BinaryOp::GreaterThanOrEqual, left, _right) => {
            extract_cos_and_query(left)?
        }
        _ => return None,
    };
    // The threshold must be a numeric constant (the Filter keeps the full
    // expression, so the exact comparison is preserved downstream).
    let rhs = match expr {
        Expression::BinaryOp(_, _, right) => right.as_ref(),
        _ => return None,
    };
    match rhs {
        Expression::Constant(Constant::Float(_)) | Expression::Constant(Constant::Integer(_)) => {}
        _ => return None,
    }
    Some((expr.clone(), column, query))
}

/// Match `cosine_similarity(<var>.<col>, [q])` and return `(col, q)`.
fn extract_cos_and_query(expr: &Expression) -> Option<(String, Vec<f64>)> {
    match expr {
        Expression::FunctionCall(name, args) if name.eq_ignore_ascii_case("cosine_similarity") && args.len() == 2 => {
            let column = match &args[0] {
                Expression::PropertyAccess(obj, col) if matches!(obj.as_ref(), Expression::Variable(_)) => col.clone(),
                _ => return None,
            };
            let query = query_vector_from_expr(&args[1])?;
            Some((column, query))
        }
        _ => None,
    }
}

/// Extract a query vector from an AST list literal `[1.0, 2.0, ...]`.
fn query_vector_from_expr(expr: &Expression) -> Option<Vec<f64>> {
    match expr {
        Expression::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Expression::Constant(Constant::Float(f)) => out.push(*f),
                    Expression::Constant(Constant::Integer(i)) => out.push(*i as f64),
                    _ => return None,
                }
            }
            Some(out)
        }
        _ => None,
    }
}

/// The ORDER BY must be a single DESC sort key on the same `cos(col, [q])`.
fn order_by_matches_cos(order: &LogicalOrderBy, column: &str, query: &[f64]) -> bool {
    if order.sort_keys.len() != 1 {
        return false;
    }
    let (expr, ascending) = &order.sort_keys[0];
    if *ascending {
        return false; // must be DESC (ascending == false)
    }
    match extract_cos_and_query(expr) {
        Some((col, q)) => col == column && q == query,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_parser::ast::{BinaryOp, Constant};

    fn prop(name: &str) -> Expression {
        Expression::PropertyAccess(Box::new(Expression::Variable("n".into())), name.into())
    }

    fn cosine(col: &str, q: &[f64]) -> Expression {
        let list = Expression::List(q.iter().map(|v| Expression::Constant(Constant::Float(*v))).collect());
        Expression::FunctionCall("cosine_similarity".into(), vec![prop(col), list])
    }

    fn gt_threshold(col: &str, q: &[f64], thr: f64) -> Expression {
        Expression::BinaryOp(
            BinaryOp::GreaterThan,
            Box::new(cosine(col, q)),
            Box::new(Expression::Constant(Constant::Float(thr))),
        )
    }

    fn scan_with_predicate(predicate: Option<Expression>) -> LogicalOperator {
        LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "Memory".into(),
            table_id: 0,
            alias: Some("n".into()),
            columns: vec!["id".into(), "embedding".into()],
            cardinality: 100,
            fts_query: None,
            predicate,
        })
    }

    fn order_by_cos(col: &str, q: &[f64]) -> LogicalOperator {
        LogicalOperator::OrderBy(LogicalOrderBy {
            sort_keys: vec![(cosine(col, q), false)],
            children: Vec::new(),
            cardinality: 0,
        })
    }

    fn projection() -> LogicalOperator {
        // The pass only checks that a Projection follows the OrderBy; it never
        // inspects the projected expressions, so an empty projection suffices.
        LogicalOperator::Projection(LogicalProjection {
            expressions: Vec::new(),
            children: Vec::new(),
            cardinality: 0,
        })
    }

    fn limit(k: u64) -> LogicalOperator {
        LogicalOperator::Limit(LogicalLimit {
            limit: k,
            offset: 0,
            children: Vec::new(),
            cardinality: 0,
        })
    }

    fn apply(plan: Vec<LogicalOperator>) -> Vec<LogicalOperator> {
        VectorSimilarityDetection.apply(&plan)
    }

    #[test]
    fn test_rewrites_canonical_pattern_preserving_rest() {
        let q = vec![0.9, 0.1];
        let plan = vec![
            scan_with_predicate(Some(gt_threshold("embedding", &q, 0.8))),
            order_by_cos("embedding", &q),
            projection(),
            limit(5),
        ];
        let result = apply(plan);
        assert_eq!(result.len(), 5, "Scan+Filter replace 1 ScanNode, rest preserved");
        // [0] VectorSimilarityScan with correct column/vector/top_k
        match &result[0] {
            LogicalOperator::VectorSimilarityScan(vs) => {
                assert_eq!(vs.column_name, "embedding");
                assert_eq!(vs.query_vector, q);
                assert_eq!(vs.top_k, 5);
                assert_eq!(vs.table_name, "Memory");
            }
            other => panic!("expected VectorSimilarityScan at [0], got {other:?}"),
        }
        // [1] Filter (threshold preserved) must follow the scan so it filters
        // the scan's output in the left-to-right flat executor.
        assert!(matches!(&result[1], LogicalOperator::Filter(_)));
        // [2..] OrderBy, Projection, Limit unchanged
        assert!(matches!(&result[2], LogicalOperator::OrderBy(_)));
        assert!(matches!(&result[3], LogicalOperator::Projection(_)));
        assert!(matches!(&result[4], LogicalOperator::Limit(_)));
    }

    #[test]
    fn test_does_not_rewrite_when_threshold_has_extra_conjunct() {
        let q = vec![0.9, 0.1];
        let pred = Expression::BinaryOp(
            BinaryOp::And,
            Box::new(gt_threshold("embedding", &q, 0.8)),
            Box::new(Expression::BinaryOp(
                BinaryOp::Equal,
                Box::new(prop("id")),
                Box::new(Expression::Constant(Constant::Integer(1))),
            )),
        );
        let plan = vec![
            scan_with_predicate(Some(pred)),
            order_by_cos("embedding", &q),
            projection(),
            limit(5),
        ];
        let result = apply(plan);
        assert_eq!(result.len(), 4, "compound predicate must prevent the rewrite");
        assert!(matches!(&result[0], LogicalOperator::ScanNode(_)));
    }

    #[test]
    fn test_does_not_rewrite_when_order_by_ascending() {
        let q = vec![0.9, 0.1];
        let mut plan = vec![
            scan_with_predicate(Some(gt_threshold("embedding", &q, 0.8))),
            order_by_cos("embedding", &q),
            projection(),
            limit(5),
        ];
        // Flip the sort to ascending; the rewrite must not fire.
        if let LogicalOperator::OrderBy(ob) = &mut plan[1] {
            ob.sort_keys[0].1 = true;
        }
        let result = apply(plan);
        assert_eq!(result.len(), 4, "ASC sort must prevent the rewrite");
        assert!(matches!(&result[0], LogicalOperator::ScanNode(_)));
    }

    #[test]
    fn test_does_not_rewrite_when_order_by_mismatches_vector() {
        let q = vec![0.9, 0.1];
        let other_q = vec![0.5, 0.5];
        let plan = vec![
            scan_with_predicate(Some(gt_threshold("embedding", &q, 0.8))),
            order_by_cos("embedding", &other_q),
            projection(),
            limit(5),
        ];
        let result = apply(plan);
        assert_eq!(result.len(), 4, "mismatched query vector must prevent the rewrite");
        assert!(matches!(&result[0], LogicalOperator::ScanNode(_)));
    }

    #[test]
    fn test_does_not_rewrite_without_limit() {
        let q = vec![0.9, 0.1];
        let plan = vec![
            scan_with_predicate(Some(gt_threshold("embedding", &q, 0.8))),
            order_by_cos("embedding", &q),
            projection(),
        ];
        let result = apply(plan);
        assert_eq!(result.len(), 3, "missing LIMIT must prevent the rewrite");
        assert!(matches!(&result[0], LogicalOperator::ScanNode(_)));
    }
}
