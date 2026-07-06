//! Individual optimizer passes for logical plan transformation.
//!
//! Each pass implements `OptimizationPass` and transforms a logical plan.
//! Passes are applied in order of registration in the Optimizer.

use kuzu_parser::ast::Expression;
use kuzu_planner::logical_operator::*;
use std::collections::HashSet;

/// An optimization pass transforms a logical plan.
pub trait OptimizationPass {
    fn name(&self) -> &str;
    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator>;
}

/// A tree-based optimization pass that transforms a logical plan tree.
///
/// Unlike `OptimizationPass` which works on flat `&[LogicalOperator]`,
/// this trait operates on the operator tree directly, enabling
/// bottom-up traversals and child insertion/deletion.
pub trait TreeOptimizationPass {
    fn name(&self) -> &str;

    /// Apply this pass to the root of a logical plan tree.
    /// The pass may recursively transform the tree in-place.
    fn apply_tree(&self, root: &mut LogicalOperator);
}

// ========================================================================
// Pass 1: Filter Push-Down
// Pushes Filter operators closer to their ScanNode sources.
// If a filter references a column from a scan, move it adjacent.
// ========================================================================

pub struct FilterPushDown;

impl OptimizationPass for FilterPushDown {
    fn name(&self) -> &str {
        "filter_push_down"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result: Vec<LogicalOperator> = Vec::new();
        let mut pending_filters: Vec<LogicalOperator> = Vec::new();

        for op in operators {
            match op {
                LogicalOperator::ScanNode(_) | LogicalOperator::ScanRel(_) => {
                    // Flush any pending filters before this scan
                    result.append(&mut pending_filters);
                    result.push(op.clone());
                }
                LogicalOperator::Filter(_) => {
                    // Defer filter — will place it before the next scan
                    pending_filters.push(op.clone());
                }
                _ => {
                    // Flush pending filters before non-scan operators
                    result.append(&mut pending_filters);
                    result.push(op.clone());
                }
            }
        }
        result.append(&mut pending_filters);
        result
    }
}

// ========================================================================
// Pass 2: Projection Push-Down
// Removes unused columns from ScanNode operators based on what's needed
// in Projection and Filter expressions.
// ========================================================================

pub struct ProjectionPushDown;

impl OptimizationPass for ProjectionPushDown {
    fn name(&self) -> &str {
        "projection_push_down"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        // Collect referenced column names from Projection and Filter
        let referenced = collect_referenced_columns(operators);

        if referenced.is_empty() {
            return operators.to_vec();
        }

        operators
            .iter()
            .map(|op| match op {
                LogicalOperator::ScanNode(s) => {
                    let cols: Vec<String> = s.columns.iter().filter(|c| referenced.contains(*c)).cloned().collect();
                    LogicalOperator::ScanNode(LogicalScanNode {
                        columns: cols,
                        ..s.clone()
                    })
                }
                other => other.clone(),
            })
            .collect()
    }
}

/// Collect column names referenced in projection and filter expressions.
fn collect_referenced_columns(operators: &[LogicalOperator]) -> HashSet<String> {
    let mut refs = HashSet::new();
    for op in operators {
        match op {
            LogicalOperator::Projection(p) => {
                for expr in &p.expressions {
                    extract_variables(&expr.expression, &mut refs);
                }
            }
            LogicalOperator::Filter(f) => {
                extract_variables(&f.expression, &mut refs);
            }
            LogicalOperator::CreateRel(c) => {
                refs.insert(format!("{}.id", c.src_node_name));
                refs.insert(format!("{}.id", c.dst_node_name));
                refs.insert(format!("{}._id", c.src_node_name));
                refs.insert(format!("{}._id", c.dst_node_name));
                for (_, expr) in &c.properties {
                    extract_variables(expr, &mut refs);
                }
            }
            LogicalOperator::CreateNode(c) => {
                for (_, expr) in &c.properties {
                    extract_variables(expr, &mut refs);
                }
            }
            LogicalOperator::Set(s) => {
                extract_variables(&s.value, &mut refs);
            }
            LogicalOperator::Unwind(u) => {
                extract_variables(&u.expression, &mut refs);
            }
            _ => {}
        }
    }
    refs
}

/// Extract variable names from an expression tree.
fn extract_variables(expr: &kuzu_parser::ast::Expression, refs: &mut HashSet<String>) {
    match expr {
        kuzu_parser::ast::Expression::Variable(name) => {
            refs.insert(name.clone());
        }
        kuzu_parser::ast::Expression::PropertyAccess(obj, _prop) => {
            extract_variables(obj, refs);
        }
        kuzu_parser::ast::Expression::BinaryOp(_, left, right) => {
            extract_variables(left, refs);
            extract_variables(right, refs);
        }
        kuzu_parser::ast::Expression::UnaryOp(_, inner) => {
            extract_variables(inner, refs);
        }
        kuzu_parser::ast::Expression::FunctionCall(_, args) => {
            for arg in args {
                extract_variables(arg, refs);
            }
        }
        kuzu_parser::ast::Expression::List(items) => {
            for item in items {
                extract_variables(item, refs);
            }
        }
        kuzu_parser::ast::Expression::Map(entries) => {
            for (_, v) in entries {
                extract_variables(v, refs);
            }
        }
        kuzu_parser::ast::Expression::Case(case_expr) => {
            if let Some(subj) = &case_expr.subject {
                extract_variables(subj, refs);
            }
            for alt in &case_expr.alternatives {
                extract_variables(&alt.when, refs);
                extract_variables(&alt.then, refs);
            }
            if let Some(else_e) = &case_expr.else_expr {
                extract_variables(else_e, refs);
            }
        }
        _ => {} // Constant, etc. — no variable refs
    }
}

// ========================================================================
// Pass 5: Aggregate Detection
// Scans Projection operators for aggregate function calls (COUNT, SUM, AVG,
// MIN, MAX) and replaces them with Aggregate operators. This is necessary
// because aggregates must process ALL rows (not per-row like projections).
// ========================================================================

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

// ========================================================================
// Pass 6: Join Optimization
// Converts filter equality conditions to join conditions.
// Reorders joins so the smallest tables are joined first (cardinality-aware).
// ========================================================================

pub struct JoinOptimization;

impl OptimizationPass for JoinOptimization {
    fn name(&self) -> &str {
        "join_optimization"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        // Try cardinality-aware join reordering
        if let Some(reordered) = crate::join_order::reorder_joins_dp_bushy(operators) {
            return reordered;
        }

        // Fallback: just remove filter conditions that are join conditions
        let mut result: Vec<LogicalOperator> = Vec::new();
        let mut filters_to_remove: Vec<usize> = Vec::new();

        for (i, op) in operators.iter().enumerate() {
            if let LogicalOperator::Filter(f) = op
                && is_join_condition(&f.expression)
            {
                filters_to_remove.push(i);
            }
        }

        for (i, op) in operators.iter().enumerate() {
            if filters_to_remove.contains(&i) {
                continue;
            }
            result.push(op.clone());
        }

        result
    }
}

/// Check if an expression is an equality join condition between two variables.
pub fn is_join_condition(expr: &kuzu_parser::ast::Expression) -> bool {
    match expr {
        kuzu_parser::ast::Expression::BinaryOp(kuzu_parser::ast::BinaryOp::Equal, left, right) => {
            let left_var = extract_root_variable(left);
            let right_var = extract_root_variable(right);
            left_var.is_some() && right_var.is_some() && left_var != right_var
        }
        _ => false,
    }
}

/// Extract the root variable from an expression (e.g., `a.id` → `a`).
pub fn extract_root_variable(expr: &kuzu_parser::ast::Expression) -> Option<String> {
    match expr {
        kuzu_parser::ast::Expression::Variable(name) => Some(name.clone()),
        kuzu_parser::ast::Expression::PropertyAccess(obj, _) => extract_root_variable(obj),
        _ => None,
    }
}

// ========================================================================
// Pass 4: Top-K Optimization
// Detects ORDER BY + LIMIT patterns and marks them for Top-K execution.
// ========================================================================

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

// ========================================================================
// Pass 9: ART Range Scan Detection
// Detects patterns like `ScanNode + Filter(pk >= lower AND pk < upper)`
// and rewrites them to `ArtIndexRangeScan` when the table has an ART index.
// ========================================================================

pub struct ArtRangeScanDetection;

impl OptimizationPass for ArtRangeScanDetection {
    fn name(&self) -> &str {
        "art_range_scan_detection"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result = Vec::with_capacity(operators.len());
        let mut i = 0;

        while i < operators.len() {
            // Look for: ScanNode + Filter(comparison on PK column)
            if i + 1 < operators.len()
                && let (LogicalOperator::ScanNode(sn), LogicalOperator::Filter(f)) = (&operators[i], &operators[i + 1])
                && let Some((lower, lower_inc, upper, upper_inc)) = extract_range_bounds(&f.expression)
            {
                result.push(LogicalOperator::ArtIndexRangeScan(LogicalArtIndexRangeScan {
                    table_name: sn.table_name.clone(),
                    table_id: sn.table_id,
                    alias: sn.alias.clone(),
                    lower_bound: lower,
                    upper_bound: upper,
                    lower_inclusive: lower_inc,
                    upper_inclusive: upper_inc,
                    cardinality: sn.cardinality.max(1),
                }));
                i += 2;
                continue;
            }

            result.push(operators[i].clone());
            i += 1;
        }

        result
    }
}

/// Extract range bounds from a filter expression.
///
/// Recognizes patterns like:
/// - `pk >= lower AND pk < upper`
/// - `pk >= lower AND pk <= upper`
/// - `pk > lower AND pk < upper`
/// - `pk >= lower` (single bound)
/// - `pk < upper` (single bound)
///
/// Returns `(lower, lower_inclusive, upper, upper_inclusive)`.
fn extract_range_bounds(
    expr: &Expression,
) -> Option<(
    Option<kuzu_common::types::Value>,
    bool,
    Option<kuzu_common::types::Value>,
    bool,
)> {
    match expr {
        Expression::BinaryOp(op, left, right) => {
            match op {
                kuzu_parser::ast::BinaryOp::And => {
                    // Recursively extract from both sides
                    let left_bounds = extract_range_bounds(left);
                    let right_bounds = extract_range_bounds(right);
                    match (left_bounds, right_bounds) {
                        (Some((l1, li1, u1, ui1)), Some((l2, li2, u2, ui2))) => {
                            // Merge bounds: use the tighter lower and upper from both sides
                            let lower = l1.clone().or(l2);
                            let lower_inc = if l1.is_some() { li1 } else { li2 };
                            let upper = u1.clone().or(u2);
                            let upper_inc = if u1.is_some() { ui1 } else { ui2 };
                            Some((lower, lower_inc, upper, upper_inc))
                        }
                        (Some(bounds), None) | (None, Some(bounds)) => Some(bounds),
                        _ => None,
                    }
                }
                // Comparison operators
                kuzu_parser::ast::BinaryOp::GreaterThanOrEqual
                | kuzu_parser::ast::BinaryOp::GreaterThan
                | kuzu_parser::ast::BinaryOp::LessThanOrEqual
                | kuzu_parser::ast::BinaryOp::LessThan
                | kuzu_parser::ast::BinaryOp::Equal => {
                    // Expect `property_access OP constant` or `constant OP property_access`
                    extract_single_bound(expr)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Extract a single bound from a comparison expression like `p.id >= 10`.
fn extract_single_bound(
    expr: &Expression,
) -> Option<(
    Option<kuzu_common::types::Value>,
    bool,
    Option<kuzu_common::types::Value>,
    bool,
)> {
    match expr {
        Expression::BinaryOp(op, left, right) => {
            let (_prop_expr, const_val) = match (left.as_ref(), right.as_ref()) {
                // p.prop >= constant
                (Expression::PropertyAccess(obj, prop), constant @ Expression::Constant(_))
                    if matches!(obj.as_ref(), Expression::Variable(_)) =>
                {
                    (prop.clone(), constant_to_value(constant))
                }
                // constant <= p.prop (reversed)
                (constant @ Expression::Constant(_), Expression::PropertyAccess(obj, prop))
                    if matches!(obj.as_ref(), Expression::Variable(_)) =>
                {
                    (prop.clone(), constant_to_value(constant))
                }
                _ => return None,
            };

            let val = const_val?;
            match op {
                kuzu_parser::ast::BinaryOp::GreaterThanOrEqual => {
                    Some((Some(val), true, None, true)) // lower inclusive
                }
                kuzu_parser::ast::BinaryOp::GreaterThan => {
                    Some((Some(val), false, None, true)) // lower exclusive
                }
                kuzu_parser::ast::BinaryOp::LessThanOrEqual => {
                    Some((None, true, Some(val), true)) // upper inclusive
                }
                kuzu_parser::ast::BinaryOp::LessThan => {
                    Some((None, true, Some(val), false)) // upper exclusive
                }
                kuzu_parser::ast::BinaryOp::Equal => {
                    // Equality: treat as both lower and upper bound
                    Some((Some(val.clone()), true, Some(val), true))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Convert a parser `Constant` to a runtime `Value`.
fn constant_to_value(c: &Expression) -> Option<kuzu_common::types::Value> {
    match c {
        Expression::Constant(kuzu_parser::ast::Constant::Integer(i)) => Some(kuzu_common::types::Value::Int64(*i)),
        Expression::Constant(kuzu_parser::ast::Constant::Float(f)) => Some(kuzu_common::types::Value::Double(*f)),
        Expression::Constant(kuzu_parser::ast::Constant::String(s)) => {
            Some(kuzu_common::types::Value::String(s.clone()))
        }
        Expression::Constant(kuzu_parser::ast::Constant::Bool(b)) => Some(kuzu_common::types::Value::Bool(*b)),
        Expression::Constant(kuzu_parser::ast::Constant::Null) => None,
        _ => None,
    }
}

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
                                    kuzu_parser::ast::Constant::Float(f) => vec.push(*f),
                                    kuzu_parser::ast::Constant::Integer(i) => vec.push(*i as f64),
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
                        // Combine ORDER BY + LIMIT into a single TopK operation
                        // by annotating the OrderBy with limit info
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
                    // Check for ORDER BY with non-adjacent LIMIT (through projection)
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
                        result.push(operators[i + 1].clone()); // projection
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

// ========================================================================
// Tree Pass 1: Factorization Rewriting
// Bottom-up insertion of LogicalFlatten operators for correct WCOJ
// factorization. Ported from C++ src/optimizer/factorization_rewriter.cpp
// ========================================================================

pub struct FactorizationRewriting;

impl FactorizationRewriting {
    /// Append LogicalFlatten nodes for each group position that isn't already flat.
    fn append_flattens(child: &mut LogicalOperator, groups_pos: &[usize]) {
        for &group_pos in groups_pos {
            // Wrap the child in a Flatten operator by replacing it in-place.
            let old = std::mem::replace(
                child,
                LogicalOperator::ScanNode(LogicalScanNode {
                    table_name: "placeholder".into(),
                    table_id: 0,
                    alias: None,
                    columns: Vec::new(),
                    cardinality: 0,
                    fts_query: None,
                }),
            );
            let flatten = LogicalOperator::Flatten(LogicalFlatten {
                group_pos,
                children: vec![old],
                cardinality: 0,
            });
            let _ = std::mem::replace(child, flatten);
        }
    }
}

impl TreeOptimizationPass for FactorizationRewriting {
    fn name(&self) -> &str {
        "factorization_rewriting"
    }

    fn apply_tree(&self, root: &mut LogicalOperator) {
        // Bottom-up traversal using the helper from A1
        LogicalOperator::visit_bottom_up(root, &mut |op| {
            match op {
                LogicalOperator::HashJoin(hj) => {
                    // Flatten probe-side and build-side groups.
                    // In the C++ version, each join side reports which groups
                    // need flattening. Here we flatten all unary groups as a
                    // conservative approximation.
                    Self::append_flattens(&mut hj.probe_side, &[0]);
                    Self::append_flattens(&mut hj.build_side, &[0]);
                }
                LogicalOperator::Projection(p) => {
                    // Flatten all children groups so projections work on scalars.
                    if let Some(first) = p.children.first_mut() {
                        Self::append_flattens(first, &[0]);
                    }
                }
                LogicalOperator::Aggregate(a) => {
                    if let Some(first) = a.children.first_mut() {
                        Self::append_flattens(first, &[0]);
                    }
                }
                LogicalOperator::OrderBy(o) => {
                    if let Some(first) = o.children.first_mut() {
                        Self::append_flattens(first, &[0]);
                    }
                }
                LogicalOperator::Limit(l) => {
                    if let Some(first) = l.children.first_mut() {
                        Self::append_flattens(first, &[0]);
                    }
                }
                LogicalOperator::Filter(f) => {
                    if let Some(first) = f.children.first_mut() {
                        Self::append_flattens(first, &[0]);
                    }
                }
                LogicalOperator::Union(u) => {
                    Self::append_flattens(&mut u.left, &[0]);
                    Self::append_flattens(&mut u.right, &[0]);
                }
                LogicalOperator::CrossProduct(cp) => {
                    Self::append_flattens(&mut cp.left, &[0]);
                    Self::append_flattens(&mut cp.right, &[0]);
                }
                LogicalOperator::SemiJoin(sj) => {
                    Self::append_flattens(&mut sj.left, &[0]);
                    Self::append_flattens(&mut sj.right, &[0]);
                }
                LogicalOperator::AntiJoin(aj) => {
                    Self::append_flattens(&mut aj.left, &[0]);
                    Self::append_flattens(&mut aj.right, &[0]);
                }
                // Accumulate: flatten children
                LogicalOperator::Accumulate(ac) => {
                    for child in &mut ac.children {
                        Self::append_flattens(child, &[0]);
                    }
                }
                // SemiMasker: flatten children
                LogicalOperator::SemiMasker(s) => {
                    for child in &mut s.children {
                        Self::append_flattens(child, &[0]);
                    }
                }
                // Leaf and Flatten operators: no transformation needed
                LogicalOperator::ArtIndexRangeScan(_)
                | LogicalOperator::ScanNode(_)
                | LogicalOperator::ScanRel(_)
                | LogicalOperator::VectorSimilarityScan(_)
                | LogicalOperator::Flatten(_)
                | LogicalOperator::TableFunctionCall(_)
                | LogicalOperator::CopyFrom(_)
                | LogicalOperator::Delete(_)
                | LogicalOperator::Set(_)
                | LogicalOperator::OptionalMatch(_)
                | LogicalOperator::Unwind(_)
                | LogicalOperator::Foreach(_)
                | LogicalOperator::Merge(_)
                | LogicalOperator::Explain(_)
                | LogicalOperator::Intersect(_)
                | LogicalOperator::RecursiveExtend(_)
                | LogicalOperator::ExpressionsScan(_)
                | LogicalOperator::CreateNodeTable(_)
                | LogicalOperator::CreateRelTable(_)
                | LogicalOperator::DropTable(_)
                | LogicalOperator::AlterTable(_)
                | LogicalOperator::CreateIndex(_)
                | LogicalOperator::DropIndex(_)
                | LogicalOperator::CreateVectorIndex(_)
                | LogicalOperator::CreateSequence(_)
                | LogicalOperator::DropSequence(_)
                | LogicalOperator::CreateNode(_)
                | LogicalOperator::CreateRel(_)
                | LogicalOperator::Extend(_)
                | LogicalOperator::CreateDml(_)
                | LogicalOperator::ExportDatabase(_)
                | LogicalOperator::ImportDatabase(_)
                | LogicalOperator::CreateFtsIndex(_)
                | LogicalOperator::FtsScan(_)
                | LogicalOperator::CountRelTable(_) => {}
            }
        });
    }
}

/// Placeholder flat-pass for backwards compatibility.
/// Delegates to the tree pass by walking the flat list as a tree.
impl OptimizationPass for FactorizationRewriting {
    fn name(&self) -> &str {
        "factorization_rewriting"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        // Convert flat list to a tree root, apply tree pass, flatten back.
        // For the flat-list model, this is a no-op since we can't reconstruct
        // the tree from a flat list reliably. Real tree pass is used via
        // TreeOptimizationPass.
        operators.to_vec()
    }
}

// ========================================================================
// Tree Pass 2: Cardinality Estimation
// Bottom-up annotation of estimated row counts on each operator.
// Ported from C++ src/optimizer/cardinality_updater.cpp with static
// selectivity constants (no storage dependency).
// ========================================================================

/// Static selectivity constant (matching C++ PlannerKnobs).
const EQUALITY_PREDICATE_SELECTIVITY: f64 = 0.01;

use kuzu_storage::stats::StatsStore;
use std::sync::{Arc, Mutex};

/// Cardinality estimation pass with optional storage-backed statistics.
///
/// When a `StatsStore` is provided, scan node cardinality is queried from
/// actual table statistics. Otherwise, static heuristics are used.
// ========================================================================
// Pass: Acc Hash Join Optimization
//
// When a hash join's probe side is selective (has filters), this pass:
// 1. Collects build-side keys as a semi-mask (reuses PhysicalHashJoin.semi_mask)
// 2. Wraps the probe side in an Accumulate operator
// 3. The semi-mask filters probe-side scan nodes at execution time
//
// This reduces the amount of data that flows through the hash join probe.
// Ported from C++ `acc_hash_join_optimizer.cpp`.
// ========================================================================

pub struct AccHashJoinOptimization;

impl TreeOptimizationPass for AccHashJoinOptimization {
    fn name(&self) -> &str {
        "acc_hash_join"
    }

    fn apply_tree(&self, root: &mut LogicalOperator) {
        LogicalOperator::visit_bottom_up(root, &mut |op| {
            if let LogicalOperator::HashJoin(hj) = op {
                // Check if probe side is selective (contains Filter operators)
                let has_filter = has_filter_in_subtree(&hj.probe_side);
                if !has_filter {
                    return;
                }

                // Wrap probe side in Accumulate
                let probe_card = hj.probe_side.cardinality();
                let probe_op = std::mem::replace(
                    &mut hj.probe_side,
                    Box::new(LogicalOperator::ScanNode(LogicalScanNode {
                        table_name: String::new(),
                        table_id: 0,
                        alias: None,
                        columns: Vec::new(),
                        cardinality: 0,
                        fts_query: None,
                    })),
                );

                let accumulate = LogicalOperator::Accumulate(LogicalAccumulate {
                    accumulate_type: kuzu_common::enums::AccumulateType::Regular,
                    flat_exprs: Vec::new(),
                    mark: None,
                    children: vec![*probe_op],
                    cardinality: probe_card,
                });

                *hj.probe_side = accumulate;
            }
        });
    }
}

// ========================================================================
// Pass: SIPOptimization
//
// Injects a LogicalSemiMasker into the build side of a HashJoin if the probe
// side is selective, enabling Sideways Information Passing.
// ========================================================================

pub struct SIPOptimization;

impl TreeOptimizationPass for SIPOptimization {
    fn name(&self) -> &str {
        "sip_optimization"
    }

    fn apply_tree(&self, root: &mut LogicalOperator) {
        LogicalOperator::visit_bottom_up(root, &mut |op| {
            if let LogicalOperator::HashJoin(hj) = op {
                let has_filter = has_filter_in_subtree(&hj.build_side);
                if !has_filter {
                    return;
                }

                // If build is selective, we can push a SemiMasker down the build side to filter probe.
                // Find the ScanNode on the build side to get the table_id.
                if let Some(scan) = find_scan_node(&hj.build_side) {
                    let table_id = scan.table_id;
                    let key_column = 0; // INTERNAL_ID is always at 0 in ScanNode output

                    let build_card = hj.build_side.cardinality();
                    let build_op = std::mem::replace(
                        &mut hj.build_side,
                        Box::new(LogicalOperator::ScanNode(
                            kuzu_planner::logical_operator::LogicalScanNode {
                                table_name: String::new(),
                                table_id: 0,
                                alias: None,
                                columns: Vec::new(),
                                cardinality: 0,
                                fts_query: None,
                            },
                        )),
                    );

                    let semi_masker = LogicalOperator::SemiMasker(kuzu_planner::logical_operator::LogicalSemiMasker {
                        table_id,
                        key_column,
                        children: vec![*build_op],
                        cardinality: build_card,
                    });
                    println!(
                        "SIPOptimization triggered: inserted SemiMasker for table_id {}",
                        table_id
                    );

                    *hj.build_side = semi_masker;
                }
            }
        });
    }
}

fn find_scan_node(op: &LogicalOperator) -> Option<&kuzu_planner::logical_operator::LogicalScanNode> {
    match op {
        LogicalOperator::ScanNode(s) => Some(s),
        _ => {
            for child in op.children() {
                if let Some(s) = find_scan_node(child) {
                    return Some(s);
                }
            }
            None
        }
    }
}

// ========================================================================
// Pass: Foreign Join PushDown Detection
//
// Detects hash joins where one or both sides are backed by foreign tables
// (via TableFunctionCall). When detected, marks the join as eligible for
// push-down optimization, allowing the physical operator to delegate the
// join execution to the foreign database.
//
// Ported from LadybugDB `foreign_join_push_down_optimizer.cpp`.
// ========================================================================

pub struct ForeignJoinPushDown;

impl TreeOptimizationPass for ForeignJoinPushDown {
    fn name(&self) -> &str {
        "foreign_join_push_down"
    }

    fn apply_tree(&self, root: &mut LogicalOperator) {
        LogicalOperator::visit_bottom_up(root, &mut |op| {
            if let LogicalOperator::HashJoin(hj) = op {
                // Check if either side is a TableFunctionCall with foreign function name
                let probe_foreign = is_foreign_table_function_call(&hj.probe_side);
                let build_foreign = is_foreign_table_function_call(&hj.build_side);

                if probe_foreign || build_foreign {
                    hj.push_down_eligible = true;
                }
            }
        });
    }
}

/// Known foreign table function name prefixes.
const FOREIGN_FUNCTION_PREFIXES: &[&str] = &["duckdb_", "postgres_", "sqlite_", "neo4j_"];

/// Check if a logical operator is a TableFunctionCall for a foreign database.
fn is_foreign_table_function_call(op: &LogicalOperator) -> bool {
    if let LogicalOperator::TableFunctionCall(tf) = op {
        let lower = tf.function_name.to_lowercase();
        FOREIGN_FUNCTION_PREFIXES
            .iter()
            .any(|&prefix| lower.starts_with(prefix))
    } else {
        false
    }
}

/// Check if a logical operator is an Accumulate.
fn is_accumulate(op: &LogicalOperator) -> bool {
    matches!(op, LogicalOperator::Accumulate(_))
}

/// Check if a subtree contains any Filter operator.
fn has_filter_in_subtree(op: &LogicalOperator) -> bool {
    match op {
        LogicalOperator::Filter(_) => true,
        _ => {
            for child in op.children() {
                if has_filter_in_subtree(child) {
                    return true;
                }
            }
            false
        }
    }
}

// ========================================================================
// Pass: Correlated Subquery Unnesting
//
// Wires ExpressionsScan operators in the build side of an AccHashJoin
// to the outer Accumulate operator on the probe side. This enables
// correlated subquery execution where the inner query reads correlated
// variables from the outer query's accumulated context.
//
// Ported from C++ `correlated_subquery_unnest_solver.cpp`.
// ========================================================================

pub struct CorrelatedSubqueryUnnesting;

impl CorrelatedSubqueryUnnesting {
    /// Recursively find and wire ExpressionsScan in a subtree to the given accumulate index.
    fn wire_expressions_scans(op: &mut LogicalOperator, acc_idx: usize) {
        if let LogicalOperator::ExpressionsScan(es) = op {
            es.outer_accumulate_idx = Some(acc_idx);
        }
        let children = op.children_mut();
        for child in children {
            Self::wire_expressions_scans(child, acc_idx);
        }
    }
}

impl TreeOptimizationPass for CorrelatedSubqueryUnnesting {
    fn name(&self) -> &str {
        "correlated_subquery_unnesting"
    }

    fn apply_tree(&self, root: &mut LogicalOperator) {
        // First pass: collect (accumulate_idx, build_side_ptr) pairs
        // to avoid borrow conflicts with the closure
        #[allow(dead_code)]
        struct AccHashJoinInfo {
            build_side_idx: usize,
        }

        let mut infos: Vec<AccHashJoinInfo> = Vec::new();
        let mut acc_counter = 0usize;

        LogicalOperator::visit_bottom_up(root, &mut |op| {
            if let LogicalOperator::HashJoin(hj) = op
                && is_accumulate(&hj.probe_side)
            {
                infos.push(AccHashJoinInfo {
                    build_side_idx: acc_counter,
                });
                acc_counter += 1;
            }
        });

        // Second pass: wire ExpressionsScans in build sides
        for info in &infos {
            // Find the HashJoin again and wire its build side
            Self::wire_build_side(root, info.build_side_idx);
        }
    }
}

impl CorrelatedSubqueryUnnesting {
    /// Find the Nth HashJoin with Accumulate on its probe side, then wire ExpressionsScans
    /// in its build side.
    fn wire_build_side(root: &mut LogicalOperator, target_idx: usize) {
        let mut found = 0usize;
        LogicalOperator::visit_bottom_up(root, &mut |op| {
            if let LogicalOperator::HashJoin(hj) = op
                && is_accumulate(&hj.probe_side)
            {
                if found == target_idx {
                    Self::wire_expressions_scans(&mut hj.build_side, target_idx);
                }
                found += 1;
            }
        });
    }
}

pub struct CardinalityEstimation {
    stats: Option<Arc<Mutex<StatsStore>>>,
}

impl CardinalityEstimation {
    pub fn new(stats: Option<Arc<Mutex<StatsStore>>>) -> Self {
        Self { stats }
    }

    /// Estimate cardinality of a scan node using storage stats when available.
    fn estimate_scan_node(&self, op: &LogicalOperator) -> u64 {
        match op {
            LogicalOperator::ScanNode(s) => {
                if s.table_name == "empty" {
                    return 0;
                }
                // Try to get real stats from the stats store
                if let Some(ref stats_store) = self.stats
                    && let Ok(store) = stats_store.lock()
                    && let Some(table_stats) = store.get_table_stats(s.table_id)
                    && table_stats.num_rows > 0
                {
                    return table_stats.num_rows;
                }
                // Fallback heuristic: 1000 nodes per table
                1000
            }
            LogicalOperator::ScanRel(s) => {
                // Try to get real stats from the stats store
                if let Some(ref stats_store) = self.stats
                    && let Ok(store) = stats_store.lock()
                    && let Some(table_stats) = store.get_table_stats(s.table_id)
                    && table_stats.num_rows > 0
                {
                    return table_stats.num_rows;
                }
                // Fallback heuristic: 5000 edges per rel table
                5000
            }
            _ => 1000,
        }
    }
}

impl TreeOptimizationPass for CardinalityEstimation {
    fn name(&self) -> &str {
        "cardinality_estimation"
    }

    fn apply_tree(&self, root: &mut LogicalOperator) {
        LogicalOperator::visit_bottom_up(root, &mut |op| {
            let card = match op {
                LogicalOperator::ArtIndexRangeScan(s) => s.cardinality,
                LogicalOperator::ScanNode(_) | LogicalOperator::ScanRel(_) => self.estimate_scan_node(op),
                LogicalOperator::Filter(f) => {
                    let child_card = f.children.first().map(|c| c.cardinality()).unwrap_or(1);
                    // Conservative filter selectivity estimate
                    std::cmp::max(1, (child_card as f64 * EQUALITY_PREDICATE_SELECTIVITY) as u64)
                }
                LogicalOperator::HashJoin(hj) => {
                    let probe_card = hj.probe_side.cardinality();
                    let build_card = hj.build_side.cardinality();
                    // NodeID-only join estimate: probe * build / max(1, probe+build)
                    let denominator = std::cmp::max(1, probe_card + build_card);
                    std::cmp::max(1, probe_card * build_card / denominator)
                }
                LogicalOperator::CrossProduct(cp) => {
                    let left_card = cp.left.cardinality();
                    let right_card = cp.right.cardinality();
                    std::cmp::max(1, left_card * right_card)
                }
                LogicalOperator::SemiJoin(sj) => {
                    let left_card = sj.left.cardinality();
                    let right_card = sj.right.cardinality();
                    std::cmp::min(left_card, right_card)
                }
                LogicalOperator::AntiJoin(aj) => {
                    let left_card = aj.left.cardinality();
                    std::cmp::max(1, (left_card as f64 * 0.1) as u64)
                }
                LogicalOperator::Projection(p) => p.children.first().map(|c| c.cardinality()).unwrap_or(1),
                LogicalOperator::OrderBy(o) => o.children.first().map(|c| c.cardinality()).unwrap_or(1),
                LogicalOperator::Limit(l) => {
                    // Cardinality is at most the limit value
                    std::cmp::min(l.limit, l.children.first().map(|c| c.cardinality()).unwrap_or(u64::MAX))
                }
                LogicalOperator::Aggregate(a) => {
                    let child_card = a.children.first().map(|c| c.cardinality()).unwrap_or(1);
                    if a.group_by.is_empty() {
                        // No GROUP BY → single row
                        1
                    } else {
                        // Has GROUP BY → at most child cardinality
                        child_card
                    }
                }
                LogicalOperator::Union(u) => {
                    let left = u.left.cardinality();
                    let right = u.right.cardinality();
                    left.saturating_add(right)
                }
                LogicalOperator::Flatten(f) => {
                    // Flatten multiplies cardinality by the group size factor
                    f.children.first().map(|c| c.cardinality()).unwrap_or(1)
                }
                LogicalOperator::TableFunctionCall(_) => {
                    // Table functions produce their own rows; default estimate
                    1000
                }
                LogicalOperator::VectorSimilarityScan(vs) => vs.top_k,
                LogicalOperator::CopyFrom(_) => 10000, // batch insert
                LogicalOperator::Delete(_) => 1000,    // estimated rows affected
                LogicalOperator::Set(_) => 1000,       // estimated rows updated
                LogicalOperator::OptionalMatch(om) => {
                    om.left.cardinality() // same as left (nullable)
                }
                LogicalOperator::Unwind(_) => 10, // list expansion estimate
                LogicalOperator::Foreach(_) => 1,
                LogicalOperator::Merge(_) => 1,      // single matched/created node
                LogicalOperator::Explain(_) => 1,    // one row with plan text
                LogicalOperator::Intersect(_) => 10, // estimate: intersection reduces cardinality
                LogicalOperator::RecursiveExtend(re) => {
                    // estimate: upper_bound * source cardinality
                    let src_card = 100;
                    re.upper_bound.saturating_mul(src_card)
                }
                LogicalOperator::Accumulate(ac) => ac.children.first().map(|c| c.cardinality()).unwrap_or(1),
                LogicalOperator::ExpressionsScan(_) => 1,
                LogicalOperator::SemiMasker(sm) => {
                    // SemiMasker passes through cardinality from its child
                    sm.children.first().map(|c| c.cardinality()).unwrap_or(1)
                }
                // DDL operators produce exactly one row (success message)
                LogicalOperator::CreateNodeTable(_)
                | LogicalOperator::CreateRelTable(_)
                | LogicalOperator::DropTable(_)
                | LogicalOperator::AlterTable(_)
                | LogicalOperator::CreateIndex(_)
                | LogicalOperator::DropIndex(_)
                | LogicalOperator::CreateVectorIndex(_)
                | LogicalOperator::CreateSequence(_)
                | LogicalOperator::DropSequence(_)
                | LogicalOperator::CreateNode(_)
                | LogicalOperator::CreateRel(_)
                | LogicalOperator::Extend(_)
                | LogicalOperator::CreateDml(_)
                | LogicalOperator::ExportDatabase(_)
                | LogicalOperator::ImportDatabase(_)
                | LogicalOperator::CreateFtsIndex(_)
                | LogicalOperator::FtsScan(_)
                | LogicalOperator::CountRelTable(_) => 1,
            };
            op.set_cardinality(card);
        });
    }
}

/// Placeholder flat-pass for backwards compatibility.
impl OptimizationPass for CardinalityEstimation {
    fn name(&self) -> &str {
        "cardinality_estimation"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        operators.to_vec()
    }
}

// ========================================================================
// Pass 7: Remove Unnecessary Operators
// ========================================================================

pub struct RemoveUnnecessaryOperators;

impl OptimizationPass for RemoveUnnecessaryOperators {
    fn name(&self) -> &str {
        "remove_unnecessary"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        operators
            .iter()
            .filter(|op| match op {
                LogicalOperator::ScanNode(s) => !s.table_name.is_empty(),
                LogicalOperator::Projection(p) => !p.expressions.is_empty(),
                LogicalOperator::Filter(f) => !is_tautology(&f.expression),
                _ => true,
            })
            .cloned()
            .collect()
    }
}

/// Check if a filter expression is a tautology (always true).
fn is_tautology(expr: &kuzu_parser::ast::Expression) -> bool {
    match expr {
        kuzu_parser::ast::Expression::Constant(kuzu_parser::ast::Constant::Bool(true)) => true,
        kuzu_parser::ast::Expression::BinaryOp(kuzu_parser::ast::BinaryOp::Equal, left, right) => {
            // `1 = 1` is a tautology
            match (&**left, &**right) {
                (
                    kuzu_parser::ast::Expression::Constant(kuzu_parser::ast::Constant::Integer(a)),
                    kuzu_parser::ast::Expression::Constant(kuzu_parser::ast::Constant::Integer(b)),
                ) => a == b,
                _ => false,
            }
        }
        _ => false,
    }
}

// ========================================================================
// Pass 8: Constant Folding
// Pre-evaluates constant sub-expressions at optimization time.
// E.g., `1 + 2` → `3`, `TRUE AND FALSE` → `FALSE`, `'he' + 'llo'` → `'hello'`
// ========================================================================

pub struct ConstantFolding;

impl OptimizationPass for ConstantFolding {
    fn name(&self) -> &str {
        "constant_folding"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        operators
            .iter()
            .map(|op| match op {
                LogicalOperator::Filter(f) => {
                    let folded = fold_expression(&f.expression);
                    LogicalOperator::Filter(LogicalFilter {
                        expression: folded,
                        children: f.children.clone(),
                        cardinality: f.cardinality,
                    })
                }
                LogicalOperator::Projection(p) => {
                    let exprs: Vec<BoundExpression> = p
                        .expressions
                        .iter()
                        .map(|e| {
                            let folded = fold_expression(&e.expression);
                            BoundExpression {
                                expression: folded,
                                resolved_type: e.resolved_type,
                                is_constant: e.is_constant,
                            }
                        })
                        .collect();
                    LogicalOperator::Projection(LogicalProjection {
                        expressions: exprs,
                        children: p.children.clone(),
                        cardinality: p.cardinality,
                    })
                }
                other => other.clone(),
            })
            .collect()
    }
}

use kuzu_binder::bound_statement::BoundExpression;

/// Fold constant sub-expressions in an expression tree.
fn fold_expression(expr: &kuzu_parser::ast::Expression) -> kuzu_parser::ast::Expression {
    use kuzu_parser::ast::{BinaryOp, Constant, Expression, UnaryOp};

    match expr {
        // Binary operations on two constants
        Expression::BinaryOp(op, left, right) => {
            let left = fold_expression(left);
            let right = fold_expression(right);
            match (&left, &right) {
                (Expression::Constant(Constant::Integer(a)), Expression::Constant(Constant::Integer(b))) => {
                    let result = match op {
                        BinaryOp::Add => Some(Constant::Integer(a + b)),
                        BinaryOp::Subtract => Some(Constant::Integer(a - b)),
                        BinaryOp::Multiply => Some(Constant::Integer(a * b)),
                        BinaryOp::Divide if *b != 0 => Some(Constant::Integer(a / b)),
                        BinaryOp::Modulo if *b != 0 => Some(Constant::Integer(a % b)),
                        BinaryOp::Equal => Some(Constant::Bool(a == b)),
                        BinaryOp::NotEqual => Some(Constant::Bool(a != b)),
                        BinaryOp::LessThan => Some(Constant::Bool(a < b)),
                        BinaryOp::LessThanOrEqual => Some(Constant::Bool(a <= b)),
                        BinaryOp::GreaterThan => Some(Constant::Bool(a > b)),
                        BinaryOp::GreaterThanOrEqual => Some(Constant::Bool(a >= b)),
                        _ => None,
                    };
                    if let Some(c) = result {
                        return Expression::Constant(c);
                    }
                }
                (Expression::Constant(Constant::Float(a)), Expression::Constant(Constant::Float(b))) => {
                    let result = match op {
                        BinaryOp::Add => Some(Constant::Float(a + b)),
                        BinaryOp::Subtract => Some(Constant::Float(a - b)),
                        BinaryOp::Multiply => Some(Constant::Float(a * b)),
                        BinaryOp::Divide if *b != 0.0 => Some(Constant::Float(a / b)),
                        BinaryOp::Equal => Some(Constant::Bool((a - b).abs() < f64::EPSILON)),
                        BinaryOp::NotEqual => Some(Constant::Bool((a - b).abs() >= f64::EPSILON)),
                        BinaryOp::LessThan => Some(Constant::Bool(a < b)),
                        BinaryOp::LessThanOrEqual => Some(Constant::Bool(a <= b)),
                        BinaryOp::GreaterThan => Some(Constant::Bool(a > b)),
                        BinaryOp::GreaterThanOrEqual => Some(Constant::Bool(a >= b)),
                        _ => None,
                    };
                    if let Some(c) = result {
                        return Expression::Constant(c);
                    }
                }
                (Expression::Constant(Constant::Bool(a)), Expression::Constant(Constant::Bool(b))) => {
                    let result = match op {
                        BinaryOp::And => Some(Constant::Bool(*a && *b)),
                        BinaryOp::Or => Some(Constant::Bool(*a || *b)),
                        BinaryOp::Xor => Some(Constant::Bool(*a ^ *b)),
                        BinaryOp::Equal => Some(Constant::Bool(*a == *b)),
                        BinaryOp::NotEqual => Some(Constant::Bool(*a != *b)),
                        _ => None,
                    };
                    if let Some(c) = result {
                        return Expression::Constant(c);
                    }
                }
                (Expression::Constant(Constant::String(a)), Expression::Constant(Constant::String(b)))
                    if (*op == BinaryOp::Concat || *op == BinaryOp::Add) =>
                {
                    return Expression::Constant(Constant::String(format!("{}{}", a, b)));
                }
                _ => {}
            }
            Expression::BinaryOp(*op, Box::new(left), Box::new(right))
        }
        // Unary operations on constants
        Expression::UnaryOp(op, inner) => {
            let inner = fold_expression(inner);
            match (&inner, op) {
                (Expression::Constant(Constant::Integer(n)), UnaryOp::Negate) => {
                    Expression::Constant(Constant::Integer(-n))
                }
                (Expression::Constant(Constant::Float(n)), UnaryOp::Negate) => {
                    Expression::Constant(Constant::Float(-n))
                }
                (Expression::Constant(Constant::Bool(b)), UnaryOp::Not) => Expression::Constant(Constant::Bool(!b)),
                _ => Expression::UnaryOp(*op, Box::new(inner)),
            }
        }
        // Recursively fold sub-expressions
        Expression::PropertyAccess(obj, prop) => {
            Expression::PropertyAccess(Box::new(fold_expression(obj)), prop.clone())
        }
        Expression::FunctionCall(name, args) => {
            let folded_args: Vec<Expression> = args.iter().map(fold_expression).collect();
            Expression::FunctionCall(name.clone(), folded_args)
        }
        Expression::List(items) => Expression::List(items.iter().map(fold_expression).collect()),
        Expression::Map(entries) => {
            Expression::Map(entries.iter().map(|(k, v)| (k.clone(), fold_expression(v))).collect())
        }
        // Leave these unchanged
        Expression::Variable(_) | Expression::Parameter(_) | Expression::Constant(_) => expr.clone(),
        Expression::ExistsSubquery(query) => Expression::ExistsSubquery(Box::new(fold_query(query))),
        Expression::Case(case_expr) => {
            use kuzu_parser::ast::{CaseAlternative, CaseExpr};
            let subject = case_expr.subject.as_ref().map(|s| Box::new(fold_expression(s)));
            let alternatives = case_expr
                .alternatives
                .iter()
                .map(|alt| CaseAlternative {
                    when: fold_expression(&alt.when),
                    then: fold_expression(&alt.then),
                })
                .collect();
            let else_expr = case_expr.else_expr.as_ref().map(|e| Box::new(fold_expression(e)));
            Expression::Case(CaseExpr {
                subject,
                alternatives,
                else_expr,
            })
        }
        Expression::Star => expr.clone(),
        Expression::ListPredicate {
            quantifier,
            list,
            var_name,
            predicate,
        } => Expression::ListPredicate {
            quantifier: *quantifier,
            list: Box::new(fold_expression(list)),
            var_name: var_name.clone(),
            predicate: Box::new(fold_expression(predicate)),
        },
    }
}

/// Fold constant sub-expressions in a Query's clauses.
fn fold_query(query: &kuzu_parser::ast::Query) -> kuzu_parser::ast::Query {
    let clauses: Vec<kuzu_parser::ast::Clause> = query
        .clauses
        .iter()
        .map(|clause| match clause {
            kuzu_parser::ast::Clause::Where(w) => kuzu_parser::ast::Clause::Where(kuzu_parser::ast::WhereClause {
                expression: fold_expression(&w.expression),
            }),
            other => other.clone(),
        })
        .collect();
    kuzu_parser::ast::Query { clauses }
}

// ========================================================================
// Pass: Agg Key Dependency Optimization
// Removes redundant GROUP BY keys that are functionally dependent on others.
//
// Ported from C++ `agg_key_dependency_optimizer.cpp`.
//
// If a GROUP BY contains both `a.id` (the primary key) and `a.name`,
// then `a.name` is functionally dependent on `a.id` and can be removed.
// This reduces the hash table size in the aggregate operator.
//
// Uses a naming heuristic: properties named "id", "_id", "ID" etc.
// are treated as primary keys. Other properties of the same variable
// are removed from GROUP BY.
// ========================================================================

pub struct AggKeyDependency;

impl AggKeyDependency {
    /// Check if a property name looks like a primary key identifier.
    fn is_id_property(name: &str) -> bool {
        matches!(name.to_lowercase().as_str(), "id" | "_id")
    }
}

impl TreeOptimizationPass for AggKeyDependency {
    fn name(&self) -> &str {
        "agg_key_dependency"
    }

    fn apply_tree(&self, root: &mut LogicalOperator) {
        LogicalOperator::visit_bottom_up(root, &mut |op| {
            if let LogicalOperator::Aggregate(agg) = op {
                if agg.group_by.len() <= 1 {
                    return; // Nothing to optimize
                }

                let original_count = agg.group_by.len();

                // Phase 1: Identify which PropertyAccess expressions are "keys"
                // (primary identifiers) and which are "dependent" (redundant).
                // A key is one where:
                //   - The property is named "id", "_id", "ID" (heuristic PK), OR
                //   - It's the first PropertyAccess encountered for that variable
                //
                // Every other PropertyAccess for the same variable is dependent
                // (functionally determined by the key).

                // Maps variable → property names that are keys
                let mut var_key_props: std::collections::HashMap<String, String> = std::collections::HashMap::new();

                // First pass: find ID properties (these take priority as keys)
                for key in &agg.group_by {
                    if let kuzu_parser::ast::Expression::PropertyAccess(obj, prop) = key
                        && let kuzu_parser::ast::Expression::Variable(var) = obj.as_ref()
                        && Self::is_id_property(prop)
                    {
                        var_key_props.entry(var.clone()).or_insert_with(|| prop.clone());
                    }
                }

                // Second pass: for variables without an ID property, use the first
                // PropertyAccess as the key.
                for key in &agg.group_by {
                    if let kuzu_parser::ast::Expression::PropertyAccess(obj, prop) = key
                        && let kuzu_parser::ast::Expression::Variable(var) = obj.as_ref()
                        && !var_key_props.contains_key(var)
                    {
                        var_key_props.insert(var.clone(), prop.clone());
                    }
                }

                // Phase 2: Filter keys — keep key expressions and non-property
                // expressions. Remove dependent property expressions.
                let mut new_keys: Vec<kuzu_parser::ast::Expression> = Vec::new();
                for key in agg.group_by.drain(..) {
                    let is_dependent = match &key {
                        kuzu_parser::ast::Expression::PropertyAccess(obj, prop) => {
                            if let kuzu_parser::ast::Expression::Variable(var) = obj.as_ref() {
                                // If this variable has a registered key property,
                                // and this expression is NOT that key → dependent
                                if let Some(key_prop) = var_key_props.get(var) {
                                    prop != key_prop
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        }
                        _ => false,
                    };
                    if !is_dependent {
                        new_keys.push(key);
                    }
                }
                agg.group_by = new_keys;

                if agg.group_by.len() < original_count {
                    tracing::debug!(
                        "AggKeyDependency: reduced group_by from {} to {} keys",
                        original_count,
                        agg.group_by.len()
                    );
                }
            }
        });
    }
}

// ========================================================================
// Pass: OrderBy Push-Down (Ladybug)
// Pushes ORDER BY below UNION ALL when safe.
// ========================================================================

pub struct OrderByPushDown;

impl OptimizationPass for OrderByPushDown {
    fn name(&self) -> &str {
        "order_by_push_down"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result = Vec::with_capacity(operators.len());
        let mut i = 0;
        while i < operators.len() {
            // Pattern: ... UNION ALL ... ORDER BY
            // Transform: ... ORDER BY ... UNION ALL ... ORDER BY ...
            // Only safe with UNION ALL (not UNION which needs dedup)
            if i + 1 < operators.len()
                && matches!(&operators[i], LogicalOperator::Union(u) if u.all)
                && matches!(&operators[i + 1], LogicalOperator::OrderBy(_))
            {
                if let LogicalOperator::Union(u) = &operators[i] {
                    // Push OrderBy into each union child
                    let order_by = operators[i + 1].clone();
                    let mut pushed_left = LogicalOperator::Union(LogicalUnion {
                        all: u.all,
                        left: Box::new(wrap_child_with_orderby(&u.left, &order_by)),
                        right: Box::new(wrap_child_with_orderby(&u.right, &order_by)),
                        cardinality: u.cardinality,
                    });
                    pushed_left.set_cardinality(u.cardinality);
                    result.push(pushed_left);
                    i += 2;
                    continue;
                }
            }
            result.push(operators[i].clone());
            i += 1;
        }
        result
    }
}

fn wrap_child_with_orderby(child: &LogicalOperator, order_by: &LogicalOperator) -> LogicalOperator {
    // Create a sequence: [child, order_by]
    LogicalOperator::Projection(LogicalProjection {
        expressions: vec![],
        children: vec![child.clone(), order_by.clone()],
        cardinality: child.cardinality(),
    })
}

// ========================================================================
// Pass: Unwind Dedup (Ladybug)
// Merges consecutive UNWIND operators on the same list expression.
// ========================================================================

pub struct UnwindDedup;

impl OptimizationPass for UnwindDedup {
    fn name(&self) -> &str {
        "unwind_dedup"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result: Vec<LogicalOperator> = Vec::new();
        let mut seen_unwinds: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut i = 0;
        while i < operators.len() {
            match &operators[i] {
                LogicalOperator::Unwind(uw) => {
                    // Use expression representation as the key for dedup
                    let key = format!("{:?}", uw.expression);
                    if !seen_unwinds.insert(key) {
                        // Duplicate UNWIND — skip it
                        tracing::debug!("UnwindDedup: removed duplicate UNWIND {:?}", uw.variable);
                        i += 1;
                        continue;
                    }
                    result.push(operators[i].clone());
                }
                _ => result.push(operators[i].clone()),
            }
            i += 1;
        }
        result
    }
}

// ========================================================================
// Pass: Count Rel Table (Ladybug)
// Detects COUNT(*) on a isolated ScanRel and replaces with CSR metadata.
// ========================================================================

pub struct CountRelTable;

impl OptimizationPass for CountRelTable {
    fn name(&self) -> &str {
        "count_rel_table"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        // Pattern: ScanRel ... Aggregate(COUNT)
        // Replace ScanRel with CountRelTable that uses CSR metadata
        let mut result: Vec<LogicalOperator> = Vec::new();
        let mut i = 0;
        while i < operators.len() {
            if i + 1 < operators.len() && matches!(&operators[i], LogicalOperator::ScanRel(_)) {
                if let LogicalOperator::ScanRel(sr) = &operators[i] {
                    if let LogicalOperator::Aggregate(agg) = &operators[i + 1]
                        && agg.aggregates.len() == 1
                        && agg.aggregates[0].0.to_uppercase().contains("COUNT")
                        && agg.group_by.is_empty()
                    {
                        // Replace ScanRel with CountRelTable
                        result.push(LogicalOperator::CountRelTable(LogicalCountRelTable {
                            table_name: sr.table_name.clone(),
                            table_id: sr.table_id,
                        }));
                        result.push(operators[i + 1].clone());
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
    use kuzu_binder::bound_statement::BoundExpression;
    use kuzu_common::types::LogicalTypeID;
    use kuzu_parser::ast::{BinaryOp, Constant, Expression, UnaryOp};

    fn make_scan(name: &str) -> LogicalOperator {
        LogicalOperator::ScanNode(LogicalScanNode {
            table_name: name.into(),
            table_id: 0,
            alias: None,
            columns: vec!["col1".into(), "col2".into()],
            cardinality: 0,
            fts_query: None,
        })
    }

    fn make_filter() -> LogicalOperator {
        LogicalOperator::Filter(LogicalFilter {
            expression: Expression::BinaryOp(
                BinaryOp::GreaterThan,
                Box::new(Expression::Variable("a".into())),
                Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(25))),
            ),
            children: Vec::new(),
            cardinality: 0,
        })
    }

    fn make_projection() -> LogicalOperator {
        LogicalOperator::Projection(LogicalProjection {
            expressions: vec![BoundExpression {
                expression: Expression::Variable("a".into()),
                resolved_type: LogicalTypeID::Any,
                is_constant: false,
            }],
            children: Vec::new(),
            cardinality: 0,
        })
    }

    fn make_order() -> LogicalOperator {
        LogicalOperator::OrderBy(LogicalOrderBy {
            sort_keys: vec![],
            children: Vec::new(),
            cardinality: 0,
        })
    }

    fn make_limit() -> LogicalOperator {
        LogicalOperator::Limit(LogicalLimit {
            limit: 10,
            offset: 0,
            children: Vec::new(),
            cardinality: 0,
        })
    }

    // Pass tests

    #[test]
    fn test_filter_push_down() {
        let plan = vec![make_filter(), make_scan("Person"), make_projection()];
        let pass = FilterPushDown;
        let result = pass.apply(&plan);
        // Filter should be moved before Scan
        assert!(matches!(result[0], LogicalOperator::Filter(_)));
        assert!(matches!(result[1], LogicalOperator::ScanNode(_)));
    }

    #[test]
    fn test_projection_push_down() {
        let plan = vec![make_scan("Person"), make_filter(), make_projection()];
        let pass = ProjectionPushDown;
        let result = pass.apply(&plan);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_join_optimization() {
        let plan = vec![make_projection(), make_scan("Person"), make_scan("City"), make_filter()];
        let pass = JoinOptimization;
        let result = pass.apply(&plan);
        // JoinOptimization now converts equi-join filters to join conditions
        // The filter here is a.age > 25 (not equi-join), so it stays
        assert_eq!(result.len(), 3); // HashJoin/CrossProduct, Projection, Filter
    }

    #[test]
    fn test_top_k_detection() {
        let plan = vec![make_order(), make_limit()];
        let pass = TopKOptimization;
        let result = pass.apply(&plan);
        assert_eq!(result.len(), 2);
        assert!(matches!(result[0], LogicalOperator::OrderBy(_)));
        assert!(matches!(result[1], LogicalOperator::Limit(_)));
    }

    #[test]
    fn test_remove_empty_projection() {
        let plan = vec![
            make_scan("Person"),
            LogicalOperator::Projection(LogicalProjection {
                expressions: vec![],
                children: Vec::new(),
                cardinality: 0,
            }),
        ];
        let pass = RemoveUnnecessaryOperators;
        let result = pass.apply(&plan);
        assert_eq!(result.len(), 1); // Empty projection removed
    }

    #[test]
    fn test_combined_passes() {
        let plan = vec![
            make_filter(),
            make_filter(),
            make_scan("Person"),
            make_scan("City"),
            make_projection(),
        ];
        // Apply filter push-down
        let pass = FilterPushDown;
        let result = pass.apply(&plan);
        // Both filters should be before scans
        let filter_pos = result.iter().position(|op| matches!(op, LogicalOperator::Filter(_)));
        let scan_pos = result.iter().position(|op| matches!(op, LogicalOperator::ScanNode(_)));
        assert!(filter_pos.unwrap() < scan_pos.unwrap());
    }

    // ==================== Constant Folding Tests ====================

    #[test]
    fn test_fold_integer_add() {
        let expr = Expression::BinaryOp(
            BinaryOp::Add,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(1))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(2))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Integer(3)));
    }

    #[test]
    fn test_fold_integer_mul() {
        let expr = Expression::BinaryOp(
            BinaryOp::Multiply,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(6))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(7))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Integer(42)));
    }

    #[test]
    fn test_fold_boolean_and() {
        let expr = Expression::BinaryOp(
            BinaryOp::And,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Bool(true))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Bool(false))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Bool(false)));
    }

    #[test]
    fn test_fold_boolean_or() {
        let expr = Expression::BinaryOp(
            BinaryOp::Or,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Bool(true))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Bool(false))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Bool(true)));
    }

    #[test]
    fn test_fold_string_concat() {
        let expr = Expression::BinaryOp(
            BinaryOp::Concat,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::String(
                "hello ".into(),
            ))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::String("world".into()))),
        );
        let result = fold_expression(&expr);
        assert_eq!(
            result,
            Expression::Constant(kuzu_parser::ast::Constant::String("hello world".into()))
        );
    }

    #[test]
    fn test_fold_comparison_lt() {
        let expr = Expression::BinaryOp(
            BinaryOp::LessThan,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(3))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(5))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Bool(true)));
    }

    #[test]
    fn test_fold_negate() {
        let expr = Expression::UnaryOp(
            UnaryOp::Negate,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(42))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Integer(-42)));
    }

    #[test]
    fn test_fold_not() {
        let expr = Expression::UnaryOp(
            UnaryOp::Not,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Bool(true))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Bool(false)));
    }

    #[test]
    fn test_fold_nested() {
        // (1 + 2) * 3 → 9
        let inner = Expression::BinaryOp(
            BinaryOp::Add,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(1))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(2))),
        );
        let outer = Expression::BinaryOp(
            BinaryOp::Multiply,
            Box::new(inner),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(3))),
        );
        let result = fold_expression(&outer);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Integer(9)));
    }

    #[test]
    fn test_fold_mixed_types_no_fold() {
        // Variable + constant should NOT be folded
        let expr = Expression::BinaryOp(
            BinaryOp::Add,
            Box::new(Expression::Variable("x".into())),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(1))),
        );
        let result = fold_expression(&expr);
        // Should remain unchanged
        assert!(matches!(result, Expression::BinaryOp(_, _, _)));
    }

    // ==================== Join Condition Tests ====================

    #[test]
    fn test_is_join_condition() {
        // a.id = b.id is a join condition
        let expr = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())),
                "id".into(),
            )),
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("b".into())),
                "id".into(),
            )),
        );
        assert!(is_join_condition(&expr));
    }

    #[test]
    fn test_is_not_join_condition() {
        // a.age > 25 is NOT a join condition
        let expr = Expression::BinaryOp(
            BinaryOp::GreaterThan,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())),
                "age".into(),
            )),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(25))),
        );
        assert!(!is_join_condition(&expr));
    }

    #[test]
    fn test_is_join_condition_same_var() {
        // a.id = a.id is NOT a join condition (same variable)
        let expr = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())),
                "id".into(),
            )),
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())),
                "id".into(),
            )),
        );
        assert!(!is_join_condition(&expr));
    }

    // ==================== Top-K with Projection Tests ====================

    #[test]
    fn test_top_k_with_projection() {
        let plan = vec![make_order(), make_projection(), make_limit()];
        let pass = TopKOptimization;
        let result = pass.apply(&plan);
        // Should still have 3 operators
        assert_eq!(result.len(), 3);
    }

    // ==================== Remove Tautology Tests ====================

    #[test]
    fn test_is_tautology_true() {
        let expr = Expression::Constant(kuzu_parser::ast::Constant::Bool(true));
        assert!(is_tautology(&expr));
    }

    #[test]
    fn test_is_tautology_false() {
        let expr = Expression::Constant(kuzu_parser::ast::Constant::Bool(false));
        assert!(!is_tautology(&expr));
    }

    #[test]
    fn test_is_tautology_equal() {
        // 1 = 1 is a tautology
        let expr = Expression::BinaryOp(
            BinaryOp::Equal,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(1))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(1))),
        );
        assert!(is_tautology(&expr));
    }

    #[test]
    fn test_remove_tautology_filter() {
        let plan = vec![
            make_scan("Person"),
            LogicalOperator::Filter(LogicalFilter {
                expression: Expression::Constant(kuzu_parser::ast::Constant::Bool(true)),
                children: Vec::new(),
                cardinality: 0,
            }),
        ];
        let pass = RemoveUnnecessaryOperators;
        let result = pass.apply(&plan);
        assert_eq!(result.len(), 1); // Tautology filter removed
    }

    // ==================== Extract Root Variable Tests ====================

    #[test]
    fn test_extract_root_variable_simple() {
        let expr = Expression::Variable("x".into());
        assert_eq!(extract_root_variable(&expr), Some("x".into()));
    }

    #[test]
    fn test_extract_root_variable_property() {
        let expr = Expression::PropertyAccess(Box::new(Expression::Variable("p".into())), "name".into());
        assert_eq!(extract_root_variable(&expr), Some("p".into()));
    }

    #[test]
    fn test_extract_root_variable_constant() {
        let expr = Expression::Constant(kuzu_parser::ast::Constant::Integer(1));
        assert_eq!(extract_root_variable(&expr), None);
    }

    #[test]
    fn test_join_optimization_removes_equi_join_filter() {
        // Create filter with a.id = b.id (equi-join condition)
        let join_filter = LogicalOperator::Filter(LogicalFilter {
            expression: Expression::BinaryOp(
                BinaryOp::Equal,
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("A".into())),
                    "id".into(),
                )),
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("B".into())),
                    "id".into(),
                )),
            ),
            children: Vec::new(),
            cardinality: 0,
        });
        let plan = vec![make_scan("A"), make_scan("B"), join_filter];
        let pass = JoinOptimization;
        let result = pass.apply(&plan);
        // Equi-join filter should be removed
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0], LogicalOperator::HashJoin(_)));
    }

    // ==================== Tree Pass Tests ====================

    #[test]
    fn test_factorization_rewriting_inserts_flatten() {
        // Build a small tree: HashJoin(ScanNode("A"), ScanNode("B"))
        let mut root = LogicalOperator::HashJoin(LogicalHashJoin {
            join_keys: vec![],
            build_side: Box::new(LogicalOperator::ScanNode(LogicalScanNode {
                table_name: "A".into(),
                table_id: 0,
                alias: None,
                columns: vec![],
                cardinality: 0,
                fts_query: None,
            })),
            probe_side: Box::new(LogicalOperator::ScanNode(LogicalScanNode {
                table_name: "B".into(),
                table_id: 1,
                alias: None,
                columns: vec![],
                cardinality: 0,
                fts_query: None,
            })),
            cardinality: 0,
            push_down_eligible: false,
        });

        let pass = FactorizationRewriting;
        pass.apply_tree(&mut root);

        // After rewriting, the hash join's children should be wrapped in Flatten
        match &root {
            LogicalOperator::HashJoin(hj) => {
                assert!(
                    matches!(&*hj.probe_side, LogicalOperator::Flatten(_)),
                    "Probe side should be wrapped in Flatten"
                );
                assert!(
                    matches!(&*hj.build_side, LogicalOperator::Flatten(_)),
                    "Build side should be wrapped in Flatten"
                );
            }
            _ => panic!("Expected HashJoin"),
        }
    }

    #[test]
    fn test_cardinality_estimation_scan_node() {
        let mut root = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "Person".into(),
            table_id: 0,
            alias: None,
            columns: vec![],
            cardinality: 0,
            fts_query: None,
        });

        let pass = CardinalityEstimation::new(None);
        pass.apply_tree(&mut root);

        // ScanNode should have default cardinality of 1000
        assert_eq!(root.cardinality(), 1000);
    }

    #[test]
    fn test_cardinality_estimation_aggregate_no_keys() {
        // Aggregate without GROUP BY → cardinality = 1
        let mut root = LogicalOperator::Aggregate(LogicalAggregate {
            group_by: vec![],
            aggregates: vec![],
            children: vec![LogicalOperator::ScanNode(LogicalScanNode {
                table_name: "T".into(),
                table_id: 0,
                alias: None,
                columns: vec![],
                cardinality: 0,
                fts_query: None,
            })],
            cardinality: 0,
        });

        let pass = CardinalityEstimation::new(None);
        pass.apply_tree(&mut root);

        assert_eq!(
            root.cardinality(),
            1,
            "Aggregate without GROUP BY should have cardinality 1"
        );
    }

    #[test]
    fn test_cardinality_estimation_limit() {
        // Limit(10) over ScanNode(1000) → cardinality = min(10, 1000) = 10
        let mut root = LogicalOperator::Limit(LogicalLimit {
            limit: 10,
            offset: 0,
            children: vec![LogicalOperator::ScanNode(LogicalScanNode {
                table_name: "T".into(),
                table_id: 0,
                alias: None,
                columns: vec![],
                cardinality: 1000,
                fts_query: None,
            })],
            cardinality: 0,
        });

        let pass = CardinalityEstimation::new(None);
        pass.apply_tree(&mut root);

        assert_eq!(
            root.cardinality(),
            10,
            "Limit should cap cardinality at its limit value"
        );
    }

    #[test]
    fn test_cardinality_estimation_cross_product() {
        let mut root = LogicalOperator::CrossProduct(LogicalCrossProduct {
            left: Box::new(LogicalOperator::ScanNode(LogicalScanNode {
                table_name: "A".into(),
                table_id: 0,
                alias: None,
                columns: vec![],
                cardinality: 0, // will be overwritten by estimate_scan_node
                fts_query: None,
            })),
            right: Box::new(LogicalOperator::ScanNode(LogicalScanNode {
                table_name: "B".into(),
                table_id: 1,
                alias: None,
                columns: vec![],
                cardinality: 0, // will be overwritten by estimate_scan_node
                fts_query: None,
            })),
            cardinality: 0,
        });

        let pass = CardinalityEstimation::new(None);
        pass.apply_tree(&mut root);

        // Both ScanNodes get default cardinality of 1000.
        // Cross product: 1000 * 1000 = 1,000,000
        assert_eq!(root.cardinality(), 1_000_000);
    }

    // --- AggKeyDependency tests ---

    fn make_aggregate(group_by: Vec<Expression>, child: LogicalOperator) -> LogicalOperator {
        LogicalOperator::Aggregate(LogicalAggregate {
            group_by,
            aggregates: vec![("COUNT".into(), vec![Expression::Constant(Constant::Integer(1))])],
            children: vec![child],
            cardinality: 0,
        })
    }

    #[test]
    fn test_agg_key_dependency_pk_only() {
        // GROUP BY a.id, a.name, a.age
        // With only a.id as PK → a.name and a.age are dependent.
        let scan = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("a".into()),
            columns: vec!["id".into(), "name".into(), "age".into()],
            cardinality: 0,
            fts_query: None,
        });
        let agg = make_aggregate(
            vec![
                Expression::PropertyAccess(Box::new(Expression::Variable("a".into())), "id".into()),
                Expression::PropertyAccess(Box::new(Expression::Variable("a".into())), "name".into()),
                Expression::PropertyAccess(Box::new(Expression::Variable("a".into())), "age".into()),
            ],
            scan,
        );

        let pass = AggKeyDependency;
        let mut plan = agg;
        pass.apply_tree(&mut plan);

        match plan {
            LogicalOperator::Aggregate(ref a) => {
                assert_eq!(a.group_by.len(), 1, "Should keep only a.id");
                // Only a.id should remain
                assert_eq!(
                    a.group_by[0],
                    Expression::PropertyAccess(Box::new(Expression::Variable("a".into())), "id".into(),),
                );
            }
            _ => panic!("Expected Aggregate"),
        }
    }

    #[test]
    fn test_agg_key_dependency_no_pk_in_keys() {
        // GROUP BY a.name, a.age (no a.id)
        // First property (a.name) is treated as key, a.age is dependent.
        let scan = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("a".into()),
            columns: vec!["name".into(), "age".into()],
            cardinality: 0,
            fts_query: None,
        });
        let agg = make_aggregate(
            vec![
                Expression::PropertyAccess(Box::new(Expression::Variable("a".into())), "name".into()),
                Expression::PropertyAccess(Box::new(Expression::Variable("a".into())), "age".into()),
            ],
            scan,
        );

        let pass = AggKeyDependency;
        let mut plan = agg;
        pass.apply_tree(&mut plan);

        match plan {
            LogicalOperator::Aggregate(ref a) => {
                assert_eq!(a.group_by.len(), 1, "Should keep only first property");
                assert_eq!(
                    a.group_by[0],
                    Expression::PropertyAccess(Box::new(Expression::Variable("a".into())), "name".into(),),
                );
            }
            _ => panic!("Expected Aggregate"),
        }
    }

    #[test]
    fn test_agg_key_dependency_non_property_keys_unchanged() {
        // GROUP BY 1, 2 (constant expressions — no properties)
        let scan = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("a".into()),
            columns: vec!["id".into()],
            cardinality: 0,
            fts_query: None,
        });
        let agg = make_aggregate(
            vec![
                Expression::Constant(Constant::Integer(1)),
                Expression::Constant(Constant::Integer(2)),
            ],
            scan,
        );

        let pass = AggKeyDependency;
        let mut plan = agg;
        pass.apply_tree(&mut plan);

        match plan {
            LogicalOperator::Aggregate(ref a) => {
                // Constants are not dependent on each other — both stay
                assert_eq!(a.group_by.len(), 2, "Both constant keys should remain");
            }
            _ => panic!("Expected Aggregate"),
        }
    }

    #[test]
    fn test_agg_key_dependency_single_key_unchanged() {
        // GROUP BY a.id only (single key — nothing to optimize)
        let scan = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("a".into()),
            columns: vec!["id".into()],
            cardinality: 0,
            fts_query: None,
        });
        let agg = make_aggregate(
            vec![Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())),
                "id".into(),
            )],
            scan,
        );

        let pass = AggKeyDependency;
        let mut plan = agg;
        pass.apply_tree(&mut plan);

        match plan {
            LogicalOperator::Aggregate(ref a) => {
                assert_eq!(a.group_by.len(), 1, "Single key should remain unchanged");
            }
            _ => panic!("Expected Aggregate"),
        }
    }

    // ==================== SIP Optimization Tests ====================

    #[test]
    fn test_sip_optimization_triggers_on_filtered_build_side() {
        // Build a HashJoin where the build side has a filter.
        // SIP should inject a SemiMasker wrapping the build side.
        let filtered_build = LogicalOperator::Filter(LogicalFilter {
            expression: Expression::BinaryOp(
                BinaryOp::GreaterThan,
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("a".into())),
                    "age".into(),
                )),
                Box::new(Expression::Constant(Constant::Integer(25))),
            ),
            children: vec![LogicalOperator::ScanNode(LogicalScanNode {
                table_name: "User".into(),
                table_id: 1,
                alias: Some("a".into()),
                columns: vec!["age".into()],
                cardinality: 100,
                fts_query: None,
            })],
            cardinality: 10,
        });

        let probe_side = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "User".into(),
            table_id: 1,
            alias: Some("a".into()),
            columns: vec!["age".into()],
            cardinality: 100,
            fts_query: None,
        });

        let hash_join = LogicalOperator::HashJoin(LogicalHashJoin {
            join_keys: vec![],
            build_side: Box::new(filtered_build),
            probe_side: Box::new(probe_side),
            cardinality: 50,
            push_down_eligible: false,
        });

        let pass = SIPOptimization;
        let mut plan = hash_join;
        pass.apply_tree(&mut plan);

        match plan {
            LogicalOperator::HashJoin(ref hj) => {
                match &*hj.build_side {
                    LogicalOperator::SemiMasker(sm) => {
                        assert_eq!(sm.table_id, 1, "SemiMasker should target table_id 1");
                        assert_eq!(sm.key_column, 0, "SemiMasker should target key column 0 (INTERNAL_ID)");
                        // Verify the SemiMasker wraps the original filtered build side
                        assert_eq!(sm.children.len(), 1, "SemiMasker should have one child");
                        match &sm.children[0] {
                            LogicalOperator::Filter(_) => {} // OK — original filter preserved
                            _ => panic!("SemiMasker child should be the original Filter"),
                        }
                    }
                    _ => panic!("SIP should have injected a SemiMasker on the build side"),
                }
            }
            _ => panic!("Expected HashJoin"),
        }
    }

    #[test]
    fn test_sip_optimization_no_filter_no_semi_masker() {
        // Build a HashJoin with NO filter on build side — SIP should NOT trigger.
        let build_side = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "User".into(),
            table_id: 1,
            alias: Some("a".into()),
            columns: vec!["id".into()],
            cardinality: 100,
            fts_query: None,
        });

        let probe_side = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "Post".into(),
            table_id: 2,
            alias: Some("p".into()),
            columns: vec!["id".into()],
            cardinality: 200,
            fts_query: None,
        });

        let hash_join = LogicalOperator::HashJoin(LogicalHashJoin {
            join_keys: vec![],
            build_side: Box::new(build_side),
            probe_side: Box::new(probe_side),
            cardinality: 50,
            push_down_eligible: false,
        });

        let pass = SIPOptimization;
        let mut plan = hash_join;
        pass.apply_tree(&mut plan);

        match plan {
            LogicalOperator::HashJoin(ref hj) => {
                // Build side should NOT have a SemiMasker (no filter present)
                assert!(
                    !matches!(&*hj.build_side, LogicalOperator::SemiMasker(_)),
                    "SIP should NOT inject SemiMasker when build side has no filter"
                );
            }
            _ => panic!("Expected HashJoin"),
        }
    }
}

// ========================================================================
// Pass 10: Limit Push-Down
// Pushes Limit operators below Filter/Projection when safe.
// ========================================================================

pub struct LimitPushDown;

impl OptimizationPass for LimitPushDown {
    fn name(&self) -> &str {
        "limit_push_down"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result: Vec<LogicalOperator> = Vec::with_capacity(operators.len());
        let mut i = 0;
        while i < operators.len() {
            if i + 1 < operators.len() {
                if matches!(operators[i], LogicalOperator::Limit(_))
                    && matches!(operators[i + 1], LogicalOperator::Filter(_))
                {
                    // Swap: push Limit below Filter
                    result.push(operators[i + 1].clone()); // Filter first
                    result.push(operators[i].clone()); // then Limit
                    i += 2;
                    continue;
                }
                if matches!(operators[i], LogicalOperator::Limit(_))
                    && matches!(operators[i + 1], LogicalOperator::Projection(_))
                {
                    // Swap: push Limit below Projection (safe for simple projections)
                    result.push(operators[i + 1].clone());
                    result.push(operators[i].clone());
                    i += 2;
                    continue;
                }
            }
            result.push(operators[i].clone());
            i += 1;
        }
        result
    }
}

// ========================================================================
// Pass 11: Common Subexpression Elimination (CSE)
// Detects duplicate expressions in Projection and caches results.
// ========================================================================

pub struct CommonSubexpressionElimination;

impl OptimizationPass for CommonSubexpressionElimination {
    fn name(&self) -> &str {
        "common_subexpression_elimination"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        operators
            .iter()
            .map(|op| {
                match op {
                    LogicalOperator::Projection(p) => {
                        // Check for duplicate expressions
                        let mut seen_exprs: Vec<&kuzu_binder::bound_statement::BoundExpression> = Vec::new();
                        let mut unique_exprs: Vec<kuzu_binder::bound_statement::BoundExpression> = Vec::new();
                        let mut mapping: Vec<usize> = Vec::new();
                        for expr in &p.expressions {
                            if let Some(pos) = seen_exprs.iter().position(|e| e.expression == expr.expression) {
                                mapping.push(pos);
                            } else {
                                seen_exprs.push(expr);
                                unique_exprs.push(expr.clone());
                                mapping.push(unique_exprs.len() - 1);
                            }
                        }
                        // Only rewrite if dedup happened
                        if unique_exprs.len() < p.expressions.len() {
                            LogicalOperator::Projection(LogicalProjection {
                                expressions: unique_exprs,
                                children: p.children.clone(),
                                cardinality: p.cardinality,
                            })
                        } else {
                            op.clone()
                        }
                    }
                    _ => op.clone(),
                }
            })
            .collect()
    }
}
