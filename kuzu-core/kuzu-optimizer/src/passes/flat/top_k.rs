// ========================================================================
// Pass 4: Top-K Optimization
// Detects ORDER BY + LIMIT patterns and marks them for Top-K execution.
// ========================================================================

use crate::passes::OptimizationPass;
use kuzu_planner::logical_operator::*;

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
                        result.push(LogicalOperator::OrderBy(LogicalOrderBy {
                            sort_keys: order.sort_keys.clone(),
                            children: Vec::new(),
                            cardinality: 0,
                        }));
                        result.push(LogicalOperator::Limit(LogicalLimit {
                            limit: limit.limit,
                            offset: limit.offset,
                            children: Vec::new(),
                            cardinality: 0,
                        }));
                        i += 2;
                        continue;
                    }
                    (LogicalOperator::OrderBy(order), LogicalOperator::Projection(_))
                        if i + 2 < operators.len() && matches!(&operators[i + 2], LogicalOperator::Limit(_)) =>
                    {
                        let limit = match &operators[i + 2] {
                            LogicalOperator::Limit(l) => l.clone(),
                            _ => unreachable!(),
                        };
                        result.push(LogicalOperator::OrderBy(LogicalOrderBy {
                            sort_keys: order.sort_keys.clone(),
                            children: Vec::new(),
                            cardinality: 0,
                        }));
                        result.push(operators[i + 1].clone());
                        result.push(LogicalOperator::Limit(LogicalLimit {
                            limit: limit.limit,
                            offset: limit.offset,
                            children: Vec::new(),
                            cardinality: 0,
                        }));
                        i += 3;
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
