use crate::passes::OptimizationPass;
use akar_planner::logical_operator::*;

/// Aggregate fusion is a NO-OP.
///
/// The original pass merged two consecutive `Aggregate` operators with the
/// same GROUP BY into one, extending the inner aggregate's expressions with the
/// outer's. That is only valid if the outer's arguments can be evaluated
/// against the inner's RAW input — but they normally reference the inner
/// aggregate's output (e.g. `COUNT(*) AS cnt` followed by `MAX(cnt)`), which
/// disappears after fusion and resolves to NULL. Even for matching GROUP BY
/// keys the fusion is wrong: `[Agg(gb, [SUM(x)]), Agg(gb, [COUNT(*)])]` reduces
/// each name to a single row before the outer COUNT runs, whereas the fused
/// COUNT(*) would count the raw rows.
///
/// Merging chained aggregates requires rewriting the outer's arguments against
/// a merged output schema — something this flat pass cannot express — so
/// consecutive aggregates are left untouched.
pub struct AggregateFusion;

impl OptimizationPass for AggregateFusion {
    fn name(&self) -> &str {
        "aggregate_fusion"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        operators.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_parser::ast::Expression;

    fn make_agg(group_by: Vec<Expression>, aggs: Vec<(String, Vec<Expression>)>) -> LogicalOperator {
        LogicalOperator::Aggregate(LogicalAggregate {
            group_by,
            aggregates: aggs,
            children: vec![],
            cardinality: 100,
        })
    }

    #[test]
    fn test_no_merge_different_group_by() {
        let pass = AggregateFusion;
        let inner = make_agg(
            vec![Expression::Variable("a".into())],
            vec![("SUM".into(), vec![Expression::Variable("x".into())])],
        );
        let outer = make_agg(vec![Expression::Variable("b".into())], vec![("COUNT".into(), vec![])]);
        let result = pass.apply(&[outer, inner]);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_no_fuse_same_group_by() {
        let pass = AggregateFusion;
        let gb = vec![Expression::Variable("name".into())];
        let inner = make_agg(gb.clone(), vec![("SUM".into(), vec![Expression::Variable("x".into())])]);
        let outer = make_agg(gb, vec![("COUNT".into(), vec![])]);
        let result = pass.apply(&[outer, inner]);
        // Fusing would change the outer COUNT from "number of groups" to
        // "number of raw rows" — the two aggregates must stay separate.
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_no_fuse_chained_aggregate() {
        let pass = AggregateFusion;
        let inner = make_agg(vec![], vec![("COUNT".into(), vec![Expression::Star])]);
        let outer = make_agg(vec![], vec![("MAX".into(), vec![Expression::Variable("cnt".into())])]);
        let result = pass.apply(&[outer, inner]);
        // `MAX(cnt)` references the inner aggregate's output; fusing would make
        // `Variable("cnt")` resolve against the raw input (NULL).
        assert_eq!(result.len(), 2);
    }
}
