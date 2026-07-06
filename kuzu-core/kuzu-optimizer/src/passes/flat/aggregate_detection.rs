// ========================================================================
// Pass 5: Aggregate Detection
// Scans Projection operators for aggregate function calls (COUNT, SUM, AVG,
// MIN, MAX) and replaces them with Aggregate operators. This is necessary
// because aggregates must process ALL rows (not per-row like projections).
// ========================================================================

use crate::passes::OptimizationPass;
use kuzu_parser::ast::Expression;
use kuzu_planner::logical_operator::*;

/// Detect aggregate function calls in projections and replace with Aggregate.
pub struct AggregateDetection;

impl OptimizationPass for AggregateDetection {
    fn name(&self) -> &str {
        "aggregate_detection"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result = Vec::with_capacity(operators.len());

        for op in operators {
            match op {
                LogicalOperator::Projection(proj) => {
                    // Check if any expression contains an aggregate function call
                    let aggregates: Vec<(String, Vec<Expression>)> = proj
                        .expressions
                        .iter()
                        .filter_map(|be| extract_aggregate_function(&be.expression))
                        .collect();

                    if aggregates.is_empty() {
                        // No aggregates — keep as projection
                        result.push(op.clone());
                    } else {
                        // Replace with Aggregate operator
                        // Non-aggregate expressions that are GROUP BY keys
                        // For simple RETURN COUNT(*) there are no GROUP BY keys
                        let group_by: Vec<Expression> = Vec::new();

                        result.push(LogicalOperator::Aggregate(LogicalAggregate {
                            group_by,
                            aggregates,
                            children: proj.children.clone(),
                            cardinality: proj.cardinality,
                        }));
                    }
                }
                _ => {
                    result.push(op.clone());
                }
            }
        }

        result
    }
}

/// Extract an aggregate function from an expression, returning (name, args) if found.
fn extract_aggregate_function(expr: &Expression) -> Option<(String, Vec<Expression>)> {
    match expr {
        Expression::FunctionCall(name, args) => {
            let upper = name.to_uppercase();
            match upper.as_str() {
                "COUNT" | "SUM" | "AVG" | "MIN" | "MAX" | "STDDEV" | "VARIANCE" | "COLLECT" => {
                    Some((upper, args.clone()))
                }
                _ => None,
            }
        }
        _ => None,
    }
}
