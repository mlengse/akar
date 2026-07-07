// ========================================================================
// Pass 4: Top-K Optimization
// Fuses ORDER BY + LIMIT into a single LogicalTopK for BinaryHeap execution.
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
            // Pattern: ORDER BY + LIMIT → TopK
            if i + 1 < operators.len() {
                match (&operators[i], &operators[i + 1]) {
                    (LogicalOperator::OrderBy(order), LogicalOperator::Limit(limit)) => {
                        result.push(LogicalOperator::TopK(LogicalTopK {
                            sort_keys: order.sort_keys.clone(),
                            limit: limit.limit,
                            offset: limit.offset,
                            children: Vec::new(),
                            cardinality: limit.limit,
                        }));
                        i += 2;
                        continue;
                    }
                    // Pattern: ORDER BY + Projection + LIMIT → ORDER BY pushed through projection as TopK
                    (LogicalOperator::OrderBy(order), LogicalOperator::Projection(_))
                        if i + 2 < operators.len()
                            && matches!(&operators[i + 2], LogicalOperator::Limit(_)) =>
                    {
                        let limit = match &operators[i + 2] {
                            LogicalOperator::Limit(l) => l.clone(),
                            _ => unreachable!(),
                        };
                        // Keep the projection, push TopK after it
                        result.push(operators[i + 1].clone()); // Projection
                        result.push(LogicalOperator::TopK(LogicalTopK {
                            sort_keys: order.sort_keys.clone(),
                            limit: limit.limit,
                            offset: limit.offset,
                            children: Vec::new(),
                            cardinality: limit.limit,
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
