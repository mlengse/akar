use crate::passes::OptimizationPass;
use kuzu_planner::logical_operator::*;

pub struct SortElision;

impl OptimizationPass for SortElision {
    fn name(&self) -> &str {
        "sort_elision"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result = Vec::with_capacity(operators.len());
        let mut i = 0;
        while i < operators.len() {
            if let LogicalOperator::OrderBy(ob) = &operators[i] {
                if ob.sort_keys.is_empty() {
                    i += 1;
                    continue;
                }
                if i + 1 < operators.len() {
                    if let LogicalOperator::OrderBy(inner_ob) = &operators[i + 1] {
                        if ob.sort_keys == inner_ob.sort_keys {
                            result.push(operators[i].clone());
                            i += 2;
                            continue;
                        }
                        let merged = LogicalOperator::OrderBy(LogicalOrderBy {
                            sort_keys: ob.sort_keys.clone(),
                            children: inner_ob.children.clone(),
                            cardinality: ob.cardinality,
                        });
                        result.push(merged);
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

#[cfg(test)]
mod tests {
    use super::*;
    use kuzu_parser::ast::Expression;

    fn make_sort(keys: Vec<(Expression, bool)>) -> LogicalOperator {
        LogicalOperator::OrderBy(LogicalOrderBy {
            sort_keys: keys,
            children: vec![],
            cardinality: 100,
        })
    }

    #[test]
    fn test_remove_empty_sort_keys() {
        let pass = SortElision;
        let sort = make_sort(vec![]);
        let scan = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "t".into(),
            table_id: 0,
            alias: None,
            columns: vec![],
            cardinality: 10,
            fts_query: None,
            predicate: None,
        });
        let result = pass.apply(&[sort, scan]);
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], LogicalOperator::ScanNode(_)));
    }

    #[test]
    fn test_remove_duplicate_sort() {
        let pass = SortElision;
        let keys = vec![(Expression::Variable("a".into()), true)];
        let sort1 = make_sort(keys.clone());
        let sort2 = make_sort(keys);
        let result = pass.apply(&[sort1, sort2]);
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], LogicalOperator::OrderBy(_)));
    }

    #[test]
    fn test_merge_different_sorts() {
        let pass = SortElision;
        let keys1 = vec![(Expression::Variable("a".into()), true)];
        let keys2 = vec![(Expression::Variable("b".into()), false)];
        let sort1 = make_sort(keys1);
        let sort2 = make_sort(keys2);
        let result = pass.apply(&[sort1, sort2]);
        assert_eq!(result.len(), 1);
        if let LogicalOperator::OrderBy(ob) = &result[0] {
            assert_eq!(ob.sort_keys.len(), 1);
            assert!(ob.sort_keys[0].1);
        } else {
            panic!("Expected OrderBy");
        }
    }
}
