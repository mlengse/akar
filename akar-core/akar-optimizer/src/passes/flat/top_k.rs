// ========================================================================
// Pass 4: Top-K Optimization
// Fuses ORDER BY + LIMIT into a single LogicalTopK for BinaryHeap execution.
// ========================================================================

use crate::passes::OptimizationPass;
use akar_planner::logical_operator::*;

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
                    (LogicalOperator::OrderBy(order), LogicalOperator::Projection(proj))
                        if i + 2 < operators.len() && matches!(&operators[i + 2], LogicalOperator::Limit(_)) =>
                    {
                        // Only fuse when every sort key is covered by the projection
                        // output. When a key references an unprojected column (e.g.
                        // `RETURN m.id ORDER BY m.access_count`), the planner placed
                        // the sort below the projection on purpose (P53.37); pushing
                        // it back above would evaluate the key against the pruned
                        // chunk and fail.
                        let covered = order.sort_keys.iter().all(|(expr, _)| {
                            akar_planner::planner::projection_covers_sort_key(&proj.expressions, expr)
                        });
                        if !covered {
                            result.push(operators[i].clone());
                            i += 1;
                            continue;
                        }
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
