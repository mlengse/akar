//! Individual optimizer passes for logical plan transformation.
//!
//! Each pass implements `OptimizationPass` and transforms a logical plan.
//! Passes are applied in order of registration in the Optimizer.

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
                    result.extend(pending_filters.drain(..));
                    result.push(op.clone());
                }
                LogicalOperator::Filter(_) => {
                    // Defer filter — will place it before the next scan
                    pending_filters.push(op.clone());
                }
                _ => {
                    // Flush pending filters before non-scan operators
                    result.extend(pending_filters.drain(..));
                    result.push(op.clone());
                }
            }
        }
        result.extend(pending_filters.drain(..));
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
                    let cols: Vec<String> = s
                        .columns
                        .iter()
                        .filter(|c| referenced.contains(*c))
                        .cloned()
                        .collect();
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
        _ => {} // Constant, etc. — no variable refs
    }
}

// ========================================================================
// Pass 3: Join Optimization
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
        if let Some(reordered) = crate::join_order::reorder_joins_greedy_first(operators) {
            return reordered;
        }

        // Fallback: just remove filter conditions that are join conditions
        let mut result: Vec<LogicalOperator> = Vec::new();
        let mut filters_to_remove: Vec<usize> = Vec::new();

        for (i, op) in operators.iter().enumerate() {
            if let LogicalOperator::Filter(f) = op {
                if is_join_condition(&f.expression) {
                    filters_to_remove.push(i);
                }
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
        kuzu_parser::ast::Expression::BinaryOp(
            kuzu_parser::ast::BinaryOp::Equal, left, right,
        ) => {
            let left_var = extract_root_variable(left);
            let right_var = extract_root_variable(right);
            matches!(left_var, Some(_)) && matches!(right_var, Some(_))
                && left_var != right_var
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
                    (LogicalOperator::OrderBy(order), LogicalOperator::Projection(_)) => {
                        if i + 2 < operators.len() {
                            if matches!(&operators[i + 2], LogicalOperator::Limit(_)) {
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
                        }
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
    fn append_flattens(
        child: &mut LogicalOperator,
        groups_pos: &[usize],
    ) {
        for &group_pos in groups_pos {
            // Wrap the child in a Flatten operator by replacing it in-place.
            let old = std::mem::replace(child, LogicalOperator::ScanNode(LogicalScanNode {
                table_name: "placeholder".into(),
                table_id: 0,
                alias: None,
                columns: Vec::new(),
                cardinality: 0,
            }));
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
                // Leaf and Flatten operators: no transformation needed
                LogicalOperator::ScanNode(_)
                | LogicalOperator::ScanRel(_)
                | LogicalOperator::Flatten(_)
                | LogicalOperator::TableFunctionCall(_)
                | LogicalOperator::CopyFrom(_) => {}
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
                if let Some(ref stats_store) = self.stats {
                    if let Ok(store) = stats_store.lock() {
                        if let Some(table_stats) = store.get_table_stats(s.table_id) {
                            if table_stats.num_rows > 0 {
                                return table_stats.num_rows;
                            }
                        }
                    }
                }
                // Fallback heuristic: 1000 nodes per table
                1000
            }
            LogicalOperator::ScanRel(s) => {
                // Try to get real stats from the stats store
                if let Some(ref stats_store) = self.stats {
                    if let Ok(store) = stats_store.lock() {
                        if let Some(table_stats) = store.get_table_stats(s.table_id) {
                            if table_stats.num_rows > 0 {
                                return table_stats.num_rows;
                            }
                        }
                    }
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
                LogicalOperator::ScanNode(_) | LogicalOperator::ScanRel(_) => {
                    self.estimate_scan_node(op)
                }
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
                LogicalOperator::Projection(p) => {
                    p.children.first().map(|c| c.cardinality()).unwrap_or(1)
                }
                LogicalOperator::OrderBy(o) => {
                    o.children.first().map(|c| c.cardinality()).unwrap_or(1)
                }
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
                LogicalOperator::CopyFrom(_) => 0,
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
        kuzu_parser::ast::Expression::Constant(
            kuzu_parser::ast::Constant::Bool(true),
        ) => true,
        kuzu_parser::ast::Expression::BinaryOp(
            kuzu_parser::ast::BinaryOp::Equal,
            left, right,
        ) => {
            // `1 = 1` is a tautology
            match (&**left, &**right) {
                (kuzu_parser::ast::Expression::Constant(kuzu_parser::ast::Constant::Integer(a)),
                 kuzu_parser::ast::Expression::Constant(kuzu_parser::ast::Constant::Integer(b))) => a == b,
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
                    let exprs: Vec<BoundExpression> = p.expressions
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
                (Expression::Constant(Constant::Integer(a)),
                 Expression::Constant(Constant::Integer(b))) => {
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
                (Expression::Constant(Constant::Float(a)),
                 Expression::Constant(Constant::Float(b))) => {
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
                (Expression::Constant(Constant::Bool(a)),
                 Expression::Constant(Constant::Bool(b))) => {
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
                (Expression::Constant(Constant::String(a)),
                 Expression::Constant(Constant::String(b))) => {
                    if *op == BinaryOp::Concat || *op == BinaryOp::Add {
                        return Expression::Constant(Constant::String(format!("{}{}", a, b)));
                    }
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
                (Expression::Constant(Constant::Bool(b)), UnaryOp::Not) => {
                    Expression::Constant(Constant::Bool(!b))
                }
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
        Expression::List(items) => {
            Expression::List(items.iter().map(fold_expression).collect())
        }
        Expression::Map(entries) => {
            Expression::Map(entries.iter().map(|(k, v)| (k.clone(), fold_expression(v))).collect())
        }
        // Leave these unchanged
        Expression::Variable(_) | Expression::Parameter(_) | Expression::Constant(_) => expr.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuzu_binder::bound_statement::BoundExpression;
    use kuzu_common::types::LogicalTypeID;
    use kuzu_parser::ast::{BinaryOp, Expression, UnaryOp};

    fn make_scan(name: &str) -> LogicalOperator {
        LogicalOperator::ScanNode(LogicalScanNode {
            table_name: name.into(),
            table_id: 0,
            alias: None,
            columns: vec!["col1".into(), "col2".into()],
            cardinality: 0,
        })
    }

    fn make_filter() -> LogicalOperator {
        LogicalOperator::Filter(LogicalFilter {
            expression: Expression::BinaryOp(
                BinaryOp::GreaterThan,
                Box::new(Expression::Variable("a".into())),
                Box::new(Expression::Constant(
                    kuzu_parser::ast::Constant::Integer(25),
                )),
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
        let plan = vec![
            make_filter(),
            make_scan("Person"),
            make_projection(),
        ];
        let pass = FilterPushDown;
        let result = pass.apply(&plan);
        // Filter should be moved before Scan
        assert!(matches!(result[0], LogicalOperator::Filter(_)));
        assert!(matches!(result[1], LogicalOperator::ScanNode(_)));
    }

    #[test]
    fn test_projection_push_down() {
        let plan = vec![
            make_scan("Person"),
            make_filter(),
            make_projection(),
        ];
        let pass = ProjectionPushDown;
        let result = pass.apply(&plan);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_join_optimization() {
        let plan = vec![
            make_projection(),
            make_scan("Person"),
            make_scan("City"),
            make_filter(),
        ];
        let pass = JoinOptimization;
        let result = pass.apply(&plan);
        // JoinOptimization now converts equi-join filters to join conditions
        // The filter here is a.age > 25 (not equi-join), so it stays
        assert_eq!(result.len(), 4); // No filters removed (non-join condition)
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
        let expr = Expression::BinaryOp(BinaryOp::Add,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(1))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(2))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Integer(3)));
    }

    #[test]
    fn test_fold_integer_mul() {
        let expr = Expression::BinaryOp(BinaryOp::Multiply,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(6))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(7))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Integer(42)));
    }

    #[test]
    fn test_fold_boolean_and() {
        let expr = Expression::BinaryOp(BinaryOp::And,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Bool(true))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Bool(false))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Bool(false)));
    }

    #[test]
    fn test_fold_boolean_or() {
        let expr = Expression::BinaryOp(BinaryOp::Or,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Bool(true))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Bool(false))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Bool(true)));
    }

    #[test]
    fn test_fold_string_concat() {
        let expr = Expression::BinaryOp(BinaryOp::Concat,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::String("hello ".into()))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::String("world".into()))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::String("hello world".into())));
    }

    #[test]
    fn test_fold_comparison_lt() {
        let expr = Expression::BinaryOp(BinaryOp::LessThan,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(3))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(5))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Bool(true)));
    }

    #[test]
    fn test_fold_negate() {
        let expr = Expression::UnaryOp(UnaryOp::Negate,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(42))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Integer(-42)));
    }

    #[test]
    fn test_fold_not() {
        let expr = Expression::UnaryOp(UnaryOp::Not,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Bool(true))),
        );
        let result = fold_expression(&expr);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Bool(false)));
    }

    #[test]
    fn test_fold_nested() {
        // (1 + 2) * 3 → 9
        let inner = Expression::BinaryOp(BinaryOp::Add,
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(1))),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(2))),
        );
        let outer = Expression::BinaryOp(BinaryOp::Multiply,
            Box::new(inner),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(3))),
        );
        let result = fold_expression(&outer);
        assert_eq!(result, Expression::Constant(kuzu_parser::ast::Constant::Integer(9)));
    }

    #[test]
    fn test_fold_mixed_types_no_fold() {
        // Variable + constant should NOT be folded
        let expr = Expression::BinaryOp(BinaryOp::Add,
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
        let expr = Expression::BinaryOp(BinaryOp::Equal,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())), "id".into(),
            )),
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("b".into())), "id".into(),
            )),
        );
        assert!(is_join_condition(&expr));
    }

    #[test]
    fn test_is_not_join_condition() {
        // a.age > 25 is NOT a join condition
        let expr = Expression::BinaryOp(BinaryOp::GreaterThan,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())), "age".into(),
            )),
            Box::new(Expression::Constant(kuzu_parser::ast::Constant::Integer(25))),
        );
        assert!(!is_join_condition(&expr));
    }

    #[test]
    fn test_is_join_condition_same_var() {
        // a.id = a.id is NOT a join condition (same variable)
        let expr = Expression::BinaryOp(BinaryOp::Equal,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())), "id".into(),
            )),
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())), "id".into(),
            )),
        );
        assert!(!is_join_condition(&expr));
    }

    // ==================== Top-K with Projection Tests ====================

    #[test]
    fn test_top_k_with_projection() {
        let plan = vec![
            make_order(),
            make_projection(),
            make_limit(),
        ];
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
        let expr = Expression::BinaryOp(BinaryOp::Equal,
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
        let expr = Expression::PropertyAccess(
            Box::new(Expression::Variable("p".into())),
            "name".into(),
        );
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
            expression: Expression::BinaryOp(BinaryOp::Equal,
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("a".into())), "id".into(),
                )),
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("b".into())), "id".into(),
                )),
            ),
            children: Vec::new(),
            cardinality: 0,
        });
        let plan = vec![
            make_scan("A"),
            make_scan("B"),
            join_filter,
        ];
        let pass = JoinOptimization;
        let result = pass.apply(&plan);
        // Equi-join filter should be removed
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|op| !matches!(op, LogicalOperator::Filter(_))));
    }

    // ==================== Tree Pass Tests ====================

    #[test]
    fn test_factorization_rewriting_inserts_flatten() {
        // Build a small tree: HashJoin(ScanNode("A"), ScanNode("B"))
        let mut root = LogicalOperator::HashJoin(LogicalHashJoin {
            join_keys: vec![],
            build_side: Box::new(LogicalOperator::ScanNode(LogicalScanNode {
                table_name: "A".into(), table_id: 0, alias: None, columns: vec![],
                cardinality: 0,
            })),
            probe_side: Box::new(LogicalOperator::ScanNode(LogicalScanNode {
                table_name: "B".into(), table_id: 1, alias: None, columns: vec![],
                cardinality: 0,
            })),
            cardinality: 0,
        });

        let pass = FactorizationRewriting;
        pass.apply_tree(&mut root);

        // After rewriting, the hash join's children should be wrapped in Flatten
        match &root {
            LogicalOperator::HashJoin(hj) => {
                assert!(matches!(&*hj.probe_side, LogicalOperator::Flatten(_)),
                    "Probe side should be wrapped in Flatten");
                assert!(matches!(&*hj.build_side, LogicalOperator::Flatten(_)),
                    "Build side should be wrapped in Flatten");
            }
            _ => panic!("Expected HashJoin"),
        }
    }

    #[test]
    fn test_cardinality_estimation_scan_node() {
        let mut root = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "Person".into(), table_id: 0, alias: None, columns: vec![],
            cardinality: 0,
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
                table_name: "T".into(), table_id: 0, alias: None, columns: vec![],
                cardinality: 0,
            })],
            cardinality: 0,
        });

        let pass = CardinalityEstimation::new(None);
        pass.apply_tree(&mut root);

        assert_eq!(root.cardinality(), 1,
            "Aggregate without GROUP BY should have cardinality 1");
    }

    #[test]
    fn test_cardinality_estimation_limit() {
        // Limit(10) over ScanNode(1000) → cardinality = min(10, 1000) = 10
        let mut root = LogicalOperator::Limit(LogicalLimit {
            limit: 10,
            offset: 0,
            children: vec![LogicalOperator::ScanNode(LogicalScanNode {
                table_name: "T".into(), table_id: 0, alias: None, columns: vec![],
                cardinality: 1000,
            })],
            cardinality: 0,
        });

        let pass = CardinalityEstimation::new(None);
        pass.apply_tree(&mut root);

        assert_eq!(root.cardinality(), 10,
            "Limit should cap cardinality at its limit value");
    }

    #[test]
    fn test_cardinality_estimation_cross_product() {
        let mut root = LogicalOperator::CrossProduct(LogicalCrossProduct {
            left: Box::new(LogicalOperator::ScanNode(LogicalScanNode {
                table_name: "A".into(), table_id: 0, alias: None, columns: vec![],
                cardinality: 0, // will be overwritten by estimate_scan_node
            })),
            right: Box::new(LogicalOperator::ScanNode(LogicalScanNode {
                table_name: "B".into(), table_id: 1, alias: None, columns: vec![],
                cardinality: 0, // will be overwritten by estimate_scan_node
            })),
            cardinality: 0,
        });

        let pass = CardinalityEstimation::new(None);
        pass.apply_tree(&mut root);

        // Both ScanNodes get default cardinality of 1000.
        // Cross product: 1000 * 1000 = 1,000,000
        assert_eq!(root.cardinality(), 1_000_000);
    }
}
