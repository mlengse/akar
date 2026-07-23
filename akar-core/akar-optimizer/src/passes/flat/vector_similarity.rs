// ========================================================================
// Pass 8: Vector Similarity Detection
// Detects the pattern: ScanNode + Filter(distance_fn) + OrderBy + Limit
// and rewrites to use VectorSimilarityScan for index-accelerated search.
//
// Pattern detected:
//   Filter(distance_fn(n.column, $query) <op> threshold)
//   → OrderBy(distance_fn(n.column, $query) ASC/DESC)
//   → Limit(K)
//
// Rewritten to:
//   VectorSimilarityScan(table_name, query_vector, top_k)
// ========================================================================

use crate::passes::OptimizationPass;
use akar_parser::ast::Expression;
use akar_planner::logical_operator::*;

/// Names of distance functions that can be accelerated by the vector index.
const DISTANCE_FUNCTIONS: &[&str] = &["cosine_similarity", "euclidean_distance", "l2_distance", "dot_product"];

pub struct VectorSimilarityDetection;

impl OptimizationPass for VectorSimilarityDetection {
    fn name(&self) -> &str {
        "vector_similarity_detection"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result = Vec::with_capacity(operators.len());
        let mut i = 0;

        while i < operators.len() {
            // Look for: ScanNode + Filter(distance_fn) + [Proj] + OrderBy + Limit
            if i + 4 < operators.len() {
                let scan = &operators[i];
                let filter = &operators[i + 1];

                // Check if we have a Projection between OrderBy and Filter
                let has_proj = matches!(&operators[i + 2], LogicalOperator::Projection(_));
                let order_by_idx = if has_proj { i + 3 } else { i + 2 };
                let limit_idx = if has_proj { i + 4 } else { i + 3 };

                if order_by_idx < operators.len() && limit_idx < operators.len() {
                    let order_by = &operators[order_by_idx];
                    let limit_op = &operators[limit_idx];

                    if let (
                        LogicalOperator::ScanNode(sn),
                        LogicalOperator::Filter(f),
                        LogicalOperator::OrderBy(ob),
                        LogicalOperator::Limit(lim),
                    ) = (scan, filter, order_by, limit_op)
                    {
                        // Check if the Filter contains a distance function call
                        if let Some((dist_fn_name, _dist_args)) = extract_distance_function(&f.expression) {
                            // Check that the OrderBy sorts by the same distance function
                            let order_matches = ob.sort_keys.iter().any(|(expr, _asc)| {
                                extract_distance_function(expr)
                                    .map(|(name, _)| name == dist_fn_name)
                                    .unwrap_or(false)
                            });

                            if order_matches {
                                // Extract the query vector from the filter expression
                                let query_vector = extract_query_vector(&f.expression);
                                let top_k = lim.limit;

                                result.push(LogicalOperator::VectorSimilarityScan(LogicalVectorSimilarityScan {
                                    index_name: String::new(), // resolved at execution
                                    index_id: 0,
                                    query_vector,
                                    top_k,
                                    table_name: sn.table_name.clone(),
                                    cardinality: top_k,
                                }));

                                // Skip past the consumed operators
                                if has_proj {
                                    i += 5;
                                } else {
                                    i += 4;
                                }
                                continue;
                            }
                        }
                    }
                }
            }

            result.push(operators[i].clone());
            i += 1;
        }

        result
    }
}

/// Extract a distance function call from an expression.
///
/// Returns `(function_name, args)` if the expression contains a recognized
/// distance function call (cosine_similarity, euclidean_distance, etc.),
/// searching through BinaryOp wrappers (like comparison operators).
fn extract_distance_function(expr: &Expression) -> Option<(String, Vec<Expression>)> {
    match expr {
        Expression::FunctionCall(name, args) => {
            let lower = name.to_lowercase();
            if DISTANCE_FUNCTIONS.contains(&lower.as_str()) {
                return Some((lower, args.clone()));
            }
            None
        }
        Expression::BinaryOp(_op, left, right) => {
            // Search both sides for a distance function
            extract_distance_function(left).or_else(|| extract_distance_function(right))
        }
        Expression::UnaryOp(_op, inner) => extract_distance_function(inner),
        _ => None,
    }
}

/// Extract the query vector (second argument to a distance function) from
/// an expression that contains `distance_fn(n.column, query_vector)`.
///
/// If the query vector is a literal list, returns the parsed `Vec<f64>`.
/// Otherwise returns an empty vector (the processor will resolve it).
fn extract_query_vector(expr: &Expression) -> Vec<f64> {
    match expr {
        Expression::FunctionCall(name, args) => {
            let lower = name.to_lowercase();
            if DISTANCE_FUNCTIONS.contains(&lower.as_str()) && args.len() >= 2 {
                match &args[1] {
                    Expression::List(items) => {
                        let mut vec = Vec::with_capacity(items.len());
                        for item in items {
                            match item {
                                Expression::Constant(c) => match c {
                                    akar_parser::ast::Constant::Float(f) => vec.push(*f),
                                    akar_parser::ast::Constant::Integer(i) => vec.push(*i as f64),
                                    _ => return Vec::new(),
                                },
                                _ => return Vec::new(),
                            }
                        }
                        vec
                    }
                    _ => Vec::new(),
                }
            } else {
                Vec::new()
            }
        }
        Expression::BinaryOp(_op, left, right) => {
            let left_res = extract_query_vector(left);
            if !left_res.is_empty() {
                left_res
            } else {
                extract_query_vector(right)
            }
        }
        Expression::UnaryOp(_op, inner) => extract_query_vector(inner),
        _ => Vec::new(),
    }
}
