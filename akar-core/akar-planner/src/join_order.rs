//! Join order enumeration — builds optimal join trees from query patterns.
//!
//! Uses a simple greedy heuristic: join the smallest tables first.

use crate::logical_operator::*;
use akar_binder::bound_statement::{BoundExpression, BoundPattern};
use akar_parser::ast::{BinaryOp, EdgeDirection, Expression};
use std::collections::HashSet;

/// A join plan tree representing how to combine scan operators.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum JoinPlan {
    /// A single leaf operator (ScanNode or ScanRel).
    Leaf(LogicalOperator),
    /// Hash join of two sub-plans with join keys.
    HashJoin {
        keys: Vec<Expression>,
        left: Box<JoinPlan>,
        right: Box<JoinPlan>,
    },
    /// Cross product of two sub-plans.
    CrossProduct { left: Box<JoinPlan>, right: Box<JoinPlan> },
}

/// Build a join tree from a list of scan operators and an optional filter expression.
///
/// Uses a greedy heuristic:
/// 1. Start with the first scan as the base
/// 2. For each remaining scan, find any join conditions from the filter
/// 3. If join conditions exist → HashJoin, otherwise → CrossProduct
///
/// P48.3: before joining, single-variable WHERE conjuncts (e.g. `b.id >= 0`)
/// are pushed into the predicate of the scan whose alias matches that variable.
/// This lets `PhysicalScan` prune rows before the cross product is materialized.
pub fn build_join_tree(scans: Vec<LogicalOperator>, filter_expr: Option<&BoundExpression>) -> JoinPlan {
    if scans.is_empty() {
        return JoinPlan::Leaf(LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "empty".into(),
            table_id: 0,
            alias: None,
            columns: Vec::new(),
            cardinality: 0,
            fts_query: None,
            predicate: None,
        }));
    }

    let mut scans = scans;
    if scans.len() == 1 {
        // P48.3: also push single-variable predicates for the single-scan case
        // so the scan can prune rows before any Extend/Projection downstream.
        if let Some(filter) = filter_expr {
            push_single_var_predicates(&mut scans, &filter.expression);
        }
        return JoinPlan::Leaf(scans.into_iter().next().unwrap());
    }

    if let Some(filter) = filter_expr {
        push_single_var_predicates(&mut scans, &filter.expression);
    }

    // Extract table aliases from scans for join condition matching

    // Extract potential join conditions from the filter
    let join_conditions = filter_expr.map_or(Vec::new(), |f| extract_join_conditions(&f.expression));

    // Greedy join ordering: start with the first scan, then join each subsequent one
    let mut scans_iter = scans.into_iter();
    let first = scans_iter.next().unwrap();
    let mut result = JoinPlan::Leaf(first);

    for scan in scans_iter {
        let alias = get_scan_alias(&scan);

        // Try to find a join condition matching this scan's alias
        let matching_conditions: Vec<Expression> = join_conditions
            .iter()
            .filter(|(left_alias, right_alias, _expr)| left_alias == &alias || right_alias == &alias)
            .map(|(_, _, expr)| expr.clone())
            .collect();

        if matching_conditions.is_empty() {
            // No join condition found — use cross product
            result = JoinPlan::CrossProduct {
                left: Box::new(result),
                right: Box::new(JoinPlan::Leaf(scan)),
            };
        } else {
            // Use the first matching join condition
            result = JoinPlan::HashJoin {
                keys: matching_conditions,
                left: Box::new(result),
                right: Box::new(JoinPlan::Leaf(scan)),
            };
        }
    }

    result
}

/// Split an expression into a list of top-level AND conjuncts.
fn split_and_conjuncts(expr: &Expression) -> Vec<Expression> {
    match expr {
        Expression::BinaryOp(BinaryOp::And, left, right) => {
            let mut out = split_and_conjuncts(left);
            out.extend(split_and_conjuncts(right));
            out
        }
        other => vec![other.clone()],
    }
}

/// Collect the set of variable names referenced by an expression.
fn collect_variables(expr: &Expression, out: &mut HashSet<String>) {
    match expr {
        Expression::Variable(v) => {
            out.insert(v.clone());
        }
        Expression::PropertyAccess(base, _) => collect_variables(base, out),
        Expression::FunctionCall(_, args) => {
            for a in args {
                collect_variables(a, out);
            }
        }
        Expression::BinaryOp(_, left, right) => {
            collect_variables(left, out);
            collect_variables(right, out);
        }
        Expression::UnaryOp(_, inner) => collect_variables(inner, out),
        Expression::List(items) => {
            for item in items {
                collect_variables(item, out);
            }
        }
        Expression::Map(items) => {
            for (_, e) in items {
                collect_variables(e, out);
            }
        }
        Expression::Case(c) => {
            if let Some(s) = &c.subject {
                collect_variables(s, out);
            }
            for alt in &c.alternatives {
                collect_variables(&alt.when, out);
                collect_variables(&alt.then, out);
            }
            if let Some(e) = &c.else_expr {
                collect_variables(e, out);
            }
        }
        Expression::ListPredicate { list, predicate, .. } => {
            collect_variables(list, out);
            collect_variables(predicate, out);
        }
        Expression::Lambda { body, .. } => collect_variables(body, out),
        _ => {}
    }
}

/// Push single-variable WHERE conjuncts into the matching scan's predicate.
///
/// A conjunct (e.g. `b.id >= 0`) that references exactly one variable is folded
/// into the `ScanNode` whose `alias` matches that variable, allowing the scan to
/// prune rows before the join. The conjunct is AND-combined with any existing
/// scan predicate. Conjuncts referencing multiple variables (join conditions)
/// or variables with no backing scan are left untouched for the top-level Filter.
fn push_single_var_predicates(scans: &mut [LogicalOperator], filter_expr: &Expression) {
    let conjuncts = split_and_conjuncts(filter_expr);

    for scan in scans.iter_mut() {
        let LogicalOperator::ScanNode(node) = scan else {
            continue;
        };
        let Some(alias) = node.alias.clone() else { continue };

        let mut pushable: Vec<Expression> = Vec::new();
        for c in &conjuncts {
            let mut vars = HashSet::new();
            collect_variables(c, &mut vars);
            if vars.len() == 1 && vars.contains(&alias) {
                pushable.push(c.clone());
            }
        }
        if pushable.is_empty() {
            continue;
        }

        let mut combined = pushable.remove(0);
        for c in pushable {
            combined = Expression::BinaryOp(BinaryOp::And, Box::new(combined), Box::new(c));
        }
        node.predicate = Some(match node.predicate.take() {
            Some(existing) => Expression::BinaryOp(BinaryOp::And, Box::new(existing), Box::new(combined)),
            None => combined,
        });
    }
}

/// Detect a WCOJ star (`MATCH (a)-[:r1]->(b), (a)-[:r2]->(c), ...`) and build an
/// `Intersect` operator whose build sides enumerate each edge pattern from the
/// shared node.
///
/// Ports the `planWCOJoin` semantics: the shared node is probed once and its
/// key value is intersected across N build hash tables (one per pattern), instead
/// of cross-joining duplicated scans of the shared node.
///
/// Triangle/cycle patterns (`MATCH (a)-[:r1]->(b), (a)-[:r2]->(c), (b)-[:r3]->(c)`)
/// additionally produce closure-edge `Extend` + `Filter` operators (returned as
/// the trailing ops) that verify the edges connecting the star's leaves.
///
/// Returns `None` when the patterns do not form a clean star (chains, var-length
/// edges, backward edges, or any leftover patterns) — callers fall back to the
/// regular join ordering.
pub fn build_wcoj_intersect(patterns: &[BoundPattern]) -> Option<(LogicalOperator, Vec<LogicalOperator>)> {
    let rel_indices: Vec<usize> = patterns
        .iter()
        .enumerate()
        .filter(|(_, p)| p.edge.is_some())
        .map(|(i, _)| i)
        .collect();
    if rel_indices.len() < 2 {
        return None;
    }

    // Categorize each rel pattern: source var, and whether it is a simple LTR
    // edge with a valid destination pattern.
    let mut src_of: Vec<Option<String>> = Vec::with_capacity(rel_indices.len());
    let mut star_dst_vars: std::collections::HashSet<String> = std::collections::HashSet::new();
    for &i in &rel_indices {
        let pattern = &patterns[i];
        let edge = pattern.edge.as_ref()?;
        let is_simple_edge = edge.lower_bound.is_none() && edge.upper_bound.is_none();
        let is_fwd = matches!(edge.direction, EdgeDirection::LeftToRight);
        if !is_simple_edge || !is_fwd {
            return None;
        }
        let dst = patterns.get(i + 1)?;
        if dst.node_variable.as_ref()? == pattern.node_variable.as_ref()? {
            // Self-loop — not a supported edge.
            return None;
        }
        src_of.push(pattern.node_variable.clone());
    }

    // Find the shared source variable with the most star edges (≥ 2).
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for s in src_of.iter().flatten() {
        *counts.entry(s).or_insert(0) += 1;
    }
    let shared_var = counts
        .iter()
        .filter(|(_, c)| **c >= 2)
        .max_by_key(|(_, c)| **c)?
        .0
        .to_string();

    // Split rel patterns into star (shared source) vs closure (source is a star leaf).
    let mut star_idxs: Vec<usize> = Vec::new();
    let mut closure_idxs: Vec<usize> = Vec::new();
    for (k, &i) in rel_indices.iter().enumerate() {
        if src_of[k].as_deref() == Some(shared_var.as_str()) {
            star_idxs.push(i);
        } else {
            closure_idxs.push(i);
        }
    }

    let mut consumed: Vec<usize> = Vec::with_capacity(patterns.len());
    let mut build_sides: Vec<Vec<LogicalOperator>> = Vec::with_capacity(star_idxs.len());
    let mut shared_label: Option<&str> = None;
    let mut shared_table_id = 0u64;

    for &i in &star_idxs {
        let pattern = &patterns[i];
        let edge = pattern.edge.as_ref()?;
        let node_var = pattern.node_variable.as_ref()?;
        let node_label = pattern.node_label.as_ref()?;
        let node_table_id = pattern.node_table_id?;
        let rel_label = edge.label.as_ref()?;
        let rel_table_id = edge.rel_table_id?;

        if shared_label.is_none() {
            shared_label = Some(node_label);
            shared_table_id = node_table_id;
        } else if shared_label != Some(node_label) || shared_table_id != node_table_id {
            return None;
        }

        let dst = patterns.get(i + 1)?;
        let dst_var = dst.node_variable.as_ref()?.clone();
        let dst_label = dst.node_label.as_ref()?.clone();
        let dst_table_id = dst.node_table_id?;
        star_dst_vars.insert(dst_var.clone());
        consumed.push(i);
        consumed.push(i + 1);

        let pipeline: Vec<LogicalOperator> = vec![
            LogicalOperator::ScanNode(LogicalScanNode {
                table_name: node_label.clone(),
                table_id: node_table_id,
                alias: Some(node_var.clone()),
                columns: Vec::new(),
                cardinality: 0,
                fts_query: None,
                predicate: None,
            }),
            LogicalOperator::Extend(LogicalExtend {
                rel_table_name: rel_label.clone(),
                rel_table_id,
                rel_var: edge.variable.clone().unwrap_or_default(),
                bound_node_var: node_var.clone(),
                direction: edge.direction.clone(),
                dst_node_var: dst_var,
                dst_table_name: dst_label,
                dst_table_id,
                cardinality: 0,
            }),
        ];
        build_sides.push(pipeline);
    }

    // Closure edges: each must connect two distinct star leaves and be the only
    // remaining rel patterns (no leftover chains).
    let mut trailing: Vec<LogicalOperator> = Vec::new();
    for &i in &closure_idxs {
        let pattern = &patterns[i];
        let edge = pattern.edge.as_ref()?;
        let src_var = pattern.node_variable.as_ref()?;
        let dst = patterns.get(i + 1)?;
        let dst_var = dst.node_variable.as_ref()?;
        if !star_dst_vars.contains(src_var) || !star_dst_vars.contains(dst_var) {
            return None;
        }
        consumed.push(i);
        consumed.push(i + 1);

        let rel_label = edge.label.as_ref()?;
        let rel_table_id = edge.rel_table_id?;
        let dst_label = dst.node_label.as_ref()?;
        let dst_table_id = dst.node_table_id?;

        // Rename the closure destination so the filter can compare it against
        // the star's copy of the same variable.
        let closure_var = format!("__wcoj_closure_{rel_label}_{dst_var}");
        trailing.push(LogicalOperator::Extend(LogicalExtend {
            rel_table_name: rel_label.clone(),
            rel_table_id,
            rel_var: edge.variable.clone().unwrap_or_default(),
            bound_node_var: src_var.clone(),
            direction: edge.direction.clone(),
            dst_node_var: closure_var.clone(),
            dst_table_name: dst_label.clone(),
            dst_table_id,
            cardinality: 0,
        }));
        trailing.push(LogicalOperator::Filter(LogicalFilter {
            expression: Expression::BinaryOp(
                BinaryOp::Equal,
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable(closure_var)),
                    "id".into(),
                )),
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable(dst_var.clone())),
                    "id".into(),
                )),
            ),
            children: Vec::new(),
            cardinality: 0,
        }));
    }

    // Only enumerate when the whole MATCH is the star + its closures — no
    // leftover patterns for the regular loop to process.
    consumed.sort_unstable();
    consumed.dedup();
    if consumed.len() != patterns.len() {
        return None;
    }

    let shared_label = shared_label?;

    // Probe side: scan the shared node once.
    let probe = LogicalOperator::ScanNode(LogicalScanNode {
        table_name: shared_label.to_string(),
        table_id: shared_table_id,
        alias: Some(shared_var.clone()),
        columns: Vec::new(),
        cardinality: 0,
        fts_query: None,
        predicate: None,
    });

    let wrap_side = |pipeline: Vec<LogicalOperator>| {
        LogicalOperator::Projection(LogicalProjection {
            expressions: Vec::new(),
            children: pipeline,
            cardinality: 0,
        })
    };

    // Build side: union of per-pattern pipelines (ScanNode(shared) → Extend).
    let mut sides = build_sides.into_iter();
    let mut left = wrap_side(sides.next()?);
    for side in sides {
        left = LogicalOperator::Union(LogicalUnion {
            left: Box::new(left),
            right: Box::new(wrap_side(side)),
            all: true,
            cardinality: 0,
        });
    }

    let key_exprs: Vec<Expression> = star_idxs
        .iter()
        .map(|_| Expression::Variable(shared_var.clone()))
        .collect();

    let root = LogicalOperator::Intersect(LogicalIntersect {
        num_build_sides: star_idxs.len() as u32,
        build_key_exprs: key_exprs,
        left: Box::new(left),
        right: Box::new(probe),
        cardinality: 0,
    });

    Some((root, trailing))
}

/// Extract table alias from a logical operator.
fn get_scan_alias(op: &LogicalOperator) -> Option<String> {
    match op {
        LogicalOperator::ScanNode(s) => s.alias.clone(),
        LogicalOperator::ScanRel(s) => {
            // Rel scans don't have aliases; use table_name
            Some(s.table_name.clone())
        }
        _ => None,
    }
}

/// Extract potential join conditions from a filter expression.
///
/// Looks for equality comparisons between variables (e.g., `a.id = b.id`).
/// Returns tuples of (left_alias, right_alias, condition_expression).
fn extract_join_conditions(expr: &Expression) -> Vec<(Option<String>, Option<String>, Expression)> {
    let mut conditions = Vec::new();
    collect_equality_conditions(expr, &mut conditions);
    conditions
}

/// Recursively collect equality conditions that reference different variables.
fn collect_equality_conditions(expr: &Expression, conditions: &mut Vec<(Option<String>, Option<String>, Expression)>) {
    match expr {
        Expression::BinaryOp(BinaryOp::Equal, left, right) => {
            let left_var = extract_variable_alias(left);
            let right_var = extract_variable_alias(right);
            if let (Some(lv), Some(rv)) = (&left_var, &right_var)
                && lv != rv
            {
                // This is a potential join condition between two different variables
                conditions.push((left_var, right_var, expr.clone()));
            }
            // Fall through to check children
        }
        Expression::BinaryOp(BinaryOp::And, left, right) => {
            collect_equality_conditions(left, conditions);
            collect_equality_conditions(right, conditions);
        }
        _ => {}
    }
}

/// Extract the variable alias from an expression.
/// e.g., `a.id` → `"a"`, `b` → `"b"`
fn extract_variable_alias(expr: &Expression) -> Option<String> {
    match expr {
        Expression::Variable(name) => Some(name.clone()),
        Expression::PropertyAccess(obj, _) => extract_variable_alias(obj),
        _ => None,
    }
}

/// Convert a JoinPlan tree to a flat Vec<LogicalOperator> for the processor.
///
/// The flattening order ensures scans appear before joins, and
/// joins appear before filters/projections.
pub fn flatten_join_plan(plan: &JoinPlan) -> Vec<LogicalOperator> {
    let mut ops = Vec::new();
    flatten_plan(plan, &mut ops);
    ops
}

fn flatten_plan(plan: &JoinPlan, ops: &mut Vec<LogicalOperator>) {
    match plan {
        JoinPlan::Leaf(op) => {
            ops.push(op.clone());
        }
        JoinPlan::HashJoin { keys, left, right } => {
            let mut left_ops = Vec::new();
            flatten_plan(left, &mut left_ops);
            let mut right_ops = Vec::new();
            flatten_plan(right, &mut right_ops);

            ops.push(LogicalOperator::HashJoin(LogicalHashJoin {
                join_keys: keys.clone(),
                build_side: Box::new(LogicalOperator::Projection(
                    crate::logical_operator::LogicalProjection {
                        expressions: Vec::new(),
                        children: left_ops,
                        cardinality: 0,
                    },
                )),
                probe_side: Box::new(LogicalOperator::Projection(
                    crate::logical_operator::LogicalProjection {
                        expressions: Vec::new(),
                        children: right_ops,
                        cardinality: 0,
                    },
                )),
                cardinality: 0,
                push_down_eligible: false,
            }));
        }
        JoinPlan::CrossProduct { left, right } => {
            let mut left_ops = Vec::new();
            flatten_plan(left, &mut left_ops);
            let mut right_ops = Vec::new();
            flatten_plan(right, &mut right_ops);

            ops.push(LogicalOperator::CrossProduct(LogicalCrossProduct {
                left: Box::new(LogicalOperator::Projection(
                    crate::logical_operator::LogicalProjection {
                        expressions: Vec::new(),
                        children: left_ops,
                        cardinality: 0,
                    },
                )),
                right: Box::new(LogicalOperator::Projection(
                    crate::logical_operator::LogicalProjection {
                        expressions: Vec::new(),
                        children: right_ops,
                        cardinality: 0,
                    },
                )),
                cardinality: 0,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_binder::bound_statement::BoundEdgePattern;

    #[test]
    fn test_single_scan_leaf() {
        let scan = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("a".into()),
            columns: Vec::new(),
            cardinality: 0,
            fts_query: None,
            predicate: None,
        });
        let plan = build_join_tree(vec![scan], None);
        match plan {
            JoinPlan::Leaf(_) => {}
            _ => panic!("Expected Leaf"),
        }
    }

    #[test]
    fn test_two_scans_cross_product() {
        let scan1 = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("a".into()),
            columns: Vec::new(),
            cardinality: 0,
            fts_query: None,
            predicate: None,
        });
        let scan2 = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "City".into(),
            table_id: 1,
            alias: Some("c".into()),
            columns: Vec::new(),
            cardinality: 0,
            fts_query: None,
            predicate: None,
        });
        let plan = build_join_tree(vec![scan1, scan2], None);
        match plan {
            JoinPlan::CrossProduct { .. } => {}
            _ => panic!("Expected CrossProduct"),
        }
    }

    #[test]
    fn test_join_condition_extraction() {
        use akar_parser::ast::Expression;
        // a.id = b.id
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
        let conditions = extract_join_conditions(&expr);
        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].0, Some("a".into()));
        assert_eq!(conditions[0].1, Some("b".into()));
    }

    #[test]
    fn test_no_join_condition() {
        use akar_parser::ast::Constant;
        let expr = Expression::BinaryOp(
            BinaryOp::GreaterThan,
            Box::new(Expression::PropertyAccess(
                Box::new(Expression::Variable("a".into())),
                "age".into(),
            )),
            Box::new(Expression::Constant(Constant::Integer(25))),
        );
        let conditions = extract_join_conditions(&expr);
        assert!(conditions.is_empty());
    }

    #[test]
    fn test_and_condition_extraction() {
        use akar_parser::ast::Expression;
        // a.id = b.id AND a.age > 25
        let expr = Expression::BinaryOp(
            BinaryOp::And,
            Box::new(Expression::BinaryOp(
                BinaryOp::Equal,
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("a".into())),
                    "id".into(),
                )),
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("b".into())),
                    "id".into(),
                )),
            )),
            Box::new(Expression::BinaryOp(
                BinaryOp::GreaterThan,
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("a".into())),
                    "age".into(),
                )),
                Box::new(Expression::Constant(akar_parser::ast::Constant::Integer(25))),
            )),
        );
        let conditions = extract_join_conditions(&expr);
        assert_eq!(conditions.len(), 1, "Should find 1 join condition");
        // The age > 25 is NOT a join condition
    }

    #[test]
    fn test_extract_variable_alias() {
        let expr = Expression::PropertyAccess(Box::new(Expression::Variable("p".into())), "name".into());
        assert_eq!(extract_variable_alias(&expr), Some("p".into()));

        let expr = Expression::Variable("x".into());
        assert_eq!(extract_variable_alias(&expr), Some("x".into()));
    }

    #[test]
    fn test_flatten_join_plan() {
        let scan = LogicalOperator::ScanNode(LogicalScanNode {
            predicate: None,
            table_name: "T".into(),
            table_id: 0,
            alias: None,
            columns: Vec::new(),
            cardinality: 0,
            fts_query: None,
        });
        let plan = JoinPlan::Leaf(scan.clone());
        let flat = flatten_join_plan(&plan);
        assert_eq!(flat.len(), 1);
    }

    #[test]
    fn test_single_var_predicate_pushdown() {
        use akar_parser::ast::Constant;
        // MATCH (a:Person), (b:Person) WHERE b.id >= 0 AND b.id <= 100
        // The conjuncts referencing only `b` should be folded into scan b's
        // predicate; scan a's predicate stays empty. The `a.id = b.id` join
        // condition references two variables so it must NOT be pushed.
        let scan_a = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("a".into()),
            columns: Vec::new(),
            cardinality: 0,
            fts_query: None,
            predicate: None,
        });
        let scan_b = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("b".into()),
            columns: Vec::new(),
            cardinality: 0,
            fts_query: None,
            predicate: None,
        });
        let filter = Expression::BinaryOp(
            BinaryOp::And,
            Box::new(Expression::BinaryOp(
                BinaryOp::And,
                Box::new(Expression::BinaryOp(
                    BinaryOp::GreaterThanOrEqual,
                    Box::new(Expression::PropertyAccess(
                        Box::new(Expression::Variable("b".into())),
                        "id".into(),
                    )),
                    Box::new(Expression::Constant(Constant::Integer(0))),
                )),
                Box::new(Expression::BinaryOp(
                    BinaryOp::LessThanOrEqual,
                    Box::new(Expression::PropertyAccess(
                        Box::new(Expression::Variable("b".into())),
                        "id".into(),
                    )),
                    Box::new(Expression::Constant(Constant::Integer(100))),
                )),
            )),
            Box::new(Expression::BinaryOp(
                BinaryOp::Equal,
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("a".into())),
                    "id".into(),
                )),
                Box::new(Expression::PropertyAccess(
                    Box::new(Expression::Variable("b".into())),
                    "id".into(),
                )),
            )),
        );

        let mut scans = vec![scan_a, scan_b];
        push_single_var_predicates(&mut scans, &filter);

        match &scans[0] {
            LogicalOperator::ScanNode(s) => assert!(
                s.predicate.is_none(),
                "scan a must not receive b-only conjuncts, got: {:?}",
                s.predicate
            ),
            _ => panic!("expected ScanNode"),
        }
        match &scans[1] {
            LogicalOperator::ScanNode(s) => {
                let pred = s.predicate.as_ref().expect("scan b should have a predicate");
                // b.id >= 0 AND b.id <= 100 — two conjuncts AND-combined.
                match pred {
                    Expression::BinaryOp(BinaryOp::And, _, _) => {}
                    other => panic!("expected AND-combined predicate, got: {other:?}"),
                }
            }
            _ => panic!("expected ScanNode"),
        }
    }

    #[test]
    fn test_flatten_cross_product() {
        let scan1 = LogicalOperator::ScanNode(LogicalScanNode {
            predicate: None,
            table_name: "A".into(),
            table_id: 0,
            alias: None,
            columns: Vec::new(),
            cardinality: 0,
            fts_query: None,
        });
        let scan2 = LogicalOperator::ScanNode(LogicalScanNode {
            predicate: None,
            table_name: "B".into(),
            table_id: 1,
            alias: None,
            columns: Vec::new(),
            cardinality: 0,
            fts_query: None,
        });
        let plan = JoinPlan::CrossProduct {
            left: Box::new(JoinPlan::Leaf(scan1)),
            right: Box::new(JoinPlan::Leaf(scan2)),
        };
        let flat = flatten_join_plan(&plan);
        assert_eq!(flat.len(), 1); // 1 cross product root
        assert!(matches!(flat[0], LogicalOperator::CrossProduct(_)));
    }

    fn mk_pattern(var: &str, label: &str, tid: u64, rel: Option<(&str, u64)>) -> BoundPattern {
        BoundPattern {
            node_variable: Some(var.into()),
            node_label: Some(label.into()),
            node_table_id: Some(tid),
            properties: Vec::new(),
            edge: rel.map(|(l, id)| BoundEdgePattern {
                variable: None,
                label: Some(l.into()),
                rel_table_id: Some(id),
                direction: EdgeDirection::LeftToRight,
                properties: Vec::new(),
                lower_bound: None,
                upper_bound: None,
            }),
        }
    }

    #[test]
    fn test_wcoj_star_detection() {
        // (a)-[:r1]->(b), (a)-[:r2]->(c)
        let patterns = vec![
            mk_pattern("a", "N", 1, Some(("r1", 10))),
            mk_pattern("b", "N", 1, None),
            mk_pattern("a", "N", 1, Some(("r2", 11))),
            mk_pattern("c", "N", 1, None),
        ];
        let (root, trailing) = build_wcoj_intersect(&patterns).expect("expected WCOJ intersect");
        assert!(matches!(&root, LogicalOperator::Intersect(i) if i.num_build_sides == 2));
        assert_eq!(root.cardinality(), 0);
        assert!(trailing.is_empty(), "no closure edges for a fan-out");
    }

    #[test]
    fn test_wcoj_triangle_detection() {
        // (a)-[:r1]->(b), (a)-[:r2]->(c), (b)-[:r3]->(c)
        let patterns = vec![
            mk_pattern("a", "N", 1, Some(("r1", 10))),
            mk_pattern("b", "N", 1, None),
            mk_pattern("a", "N", 1, Some(("r2", 11))),
            mk_pattern("c", "N", 1, None),
            mk_pattern("b", "N", 1, Some(("r3", 12))),
            mk_pattern("c", "N", 1, None),
        ];
        let (root, trailing) = build_wcoj_intersect(&patterns).expect("expected triangle WCOJ");
        assert!(matches!(&root, LogicalOperator::Intersect(i) if i.num_build_sides == 2));
        assert_eq!(trailing.len(), 2, "closure Extend + Filter expected");
    }

    #[test]
    fn test_wcoj_chain_falls_back() {
        // (a)-[:r1]->(b), (b)-[:r2]->(c) — a chain, not a star
        let patterns = vec![
            mk_pattern("a", "N", 1, Some(("r1", 10))),
            mk_pattern("b", "N", 1, None),
            mk_pattern("b", "N", 1, Some(("r2", 11))),
            mk_pattern("c", "N", 1, None),
        ];
        assert!(build_wcoj_intersect(&patterns).is_none());
    }

    #[test]
    fn test_wcoj_single_edge_falls_back() {
        // A single edge is not a WCOJ star.
        let patterns = vec![mk_pattern("a", "N", 1, Some(("r1", 10))), mk_pattern("b", "N", 1, None)];
        assert!(build_wcoj_intersect(&patterns).is_none());
    }
}
