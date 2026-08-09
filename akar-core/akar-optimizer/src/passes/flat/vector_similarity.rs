// ========================================================================
// Pass 8: Vector Similarity Detection
// ========================================================================
//
// This pass is a NO-OP. The original implementation rewrote
// `[ScanNode, Filter(distance), (Projection), OrderBy, Limit]` into a single
// `VectorSimilarityScan(query_vector, top_k)`, consuming the Projection
// (destroying the RETURN schema), the OrderBy and the Limit, and silently
// dropping the distance threshold from the Filter (e.g.
// `cosine_similarity(n.v, $q) > 0.8` → no threshold carried over).
//
// It is also unreachable in the current pipeline: FilterPushDown (pass 2)
// folds single-alias filters into `ScanNode.predicate` before this pass runs,
// so the `[ScanNode, Filter, ...]` pattern never survives. Re-enabling this
// pass requires fixing the whole vector-scan path first — threshold plumbing,
// proper output field names/types and an `_id` column (see P52.38/P52.40 in
// the implementation plan) — so the plan is kept untouched to guarantee
// correct, un-accelerated results.

use crate::passes::OptimizationPass;
use akar_planner::logical_operator::LogicalOperator;

pub struct VectorSimilarityDetection;

impl OptimizationPass for VectorSimilarityDetection {
    fn name(&self) -> &str {
        "vector_similarity_detection"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        operators.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_planner::logical_operator::{LogicalFilter, LogicalLimit, LogicalOrderBy, LogicalScanNode};

    #[test]
    fn test_vector_similarity_pattern_is_left_untouched() {
        // The distance-filter pattern must survive the pass unchanged:
        // rewriting it would drop the threshold and the RETURN projection.
        let plan = vec![
            LogicalOperator::ScanNode(LogicalScanNode {
                table_name: "Person".into(),
                table_id: 0,
                alias: Some("n".into()),
                columns: vec!["v".into()],
                cardinality: 100,
                fts_query: None,
                predicate: None,
            }),
            LogicalOperator::Filter(LogicalFilter {
                expression: akar_parser::ast::Expression::FunctionCall(
                    "cosine_similarity".into(),
                    vec![],
                ),
                children: vec![],
                cardinality: 0,
            }),
            LogicalOperator::OrderBy(LogicalOrderBy {
                sort_keys: vec![],
                children: vec![],
                cardinality: 0,
            }),
            LogicalOperator::Limit(LogicalLimit {
                limit: 5,
                offset: 0,
                children: vec![],
                cardinality: 0,
            }),
        ];
        let result = VectorSimilarityDetection.apply(&plan);
        assert_eq!(result.len(), 4, "plan must be preserved verbatim");
        assert!(matches!(result[0], LogicalOperator::ScanNode(_)));
        assert!(matches!(result[1], LogicalOperator::Filter(_)));
        assert!(matches!(result[2], LogicalOperator::OrderBy(_)));
        assert!(matches!(result[3], LogicalOperator::Limit(_)));
    }
}
