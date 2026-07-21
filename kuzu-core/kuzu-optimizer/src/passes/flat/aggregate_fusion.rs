use crate::passes::OptimizationPass;
use kuzu_planner::logical_operator::*;

pub struct AggregateFusion;

impl OptimizationPass for AggregateFusion {
    fn name(&self) -> &str {
        "aggregate_fusion"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result = Vec::with_capacity(operators.len());
        let mut i = 0;
        while i < operators.len() {
            if i + 1 < operators.len() {
                if let (LogicalOperator::Aggregate(outer), LogicalOperator::Aggregate(inner)) =
                    (&operators[i], &operators[i + 1])
                {
                    if can_fuse(outer, inner) {
                        result.push(LogicalOperator::Aggregate(fuse(outer, inner)));
                        i += 2;
                        continue;
                    }
                }
            }
            result.push(operators[i].clone());
            i += 1;
        }
        result
    }
}

fn can_fuse(outer: &LogicalAggregate, inner: &LogicalAggregate) -> bool {
    outer.group_by == inner.group_by
}

fn fuse(outer: &LogicalAggregate, inner: &LogicalAggregate) -> LogicalAggregate {
    let mut merged = inner.aggregates.clone();
    merged.extend(outer.aggregates.clone());
    LogicalAggregate {
        group_by: outer.group_by.clone(),
        aggregates: merged,
        children: inner.children.clone(),
        cardinality: outer.cardinality,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuzu_parser::ast::Expression;

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
        let outer = make_agg(
            vec![Expression::Variable("b".into())],
            vec![("COUNT".into(), vec![])],
        );
        let result = pass.apply(&[outer, inner]);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_fuse_same_group_by() {
        let pass = AggregateFusion;
        let gb = vec![Expression::Variable("name".into())];
        let inner = make_agg(gb.clone(), vec![("SUM".into(), vec![Expression::Variable("x".into())])]);
        let outer = make_agg(gb, vec![("COUNT".into(), vec![])]);
        let result = pass.apply(&[outer, inner]);
        assert_eq!(result.len(), 1);
        if let LogicalOperator::Aggregate(agg) = &result[0] {
            assert_eq!(agg.aggregates.len(), 2);
        } else {
            panic!("Expected Aggregate");
        }
    }

    #[test]
    fn test_fuse_no_group_by() {
        let pass = AggregateFusion;
        let inner = make_agg(vec![], vec![("COUNT".into(), vec![Expression::Star])]);
        let outer = make_agg(vec![], vec![("MAX".into(), vec![Expression::Variable("cnt".into())])]);
        let result = pass.apply(&[outer, inner]);
        assert_eq!(result.len(), 1);
        if let LogicalOperator::Aggregate(agg) = &result[0] {
            assert_eq!(agg.aggregates.len(), 2);
            assert_eq!(agg.children, inner.children);
        } else {
            panic!("Expected Aggregate");
        }
    }
}
