//! Join order enumeration — cardinality-aware greedy join reordering.
//!
//! Provides a tree-based optimization pass that collects all leaf scans
//! from a join tree, sorts them by estimated cardinality (ascending),
//! and rebuilds the join tree so the smallest relations are joined first.
//!
//! This pass runs after `CardinalityEstimation` has annotated operators
//! with row counts.

use kuzu_planner::logical_operator::*;
use std::collections::HashMap;

/// Collect all leaf scan operators from the join tree with their cardinalities.
///
/// Returns a list of (cardinality, operator) pairs sorted ascending.
fn collect_scans_sorted(op: &LogicalOperator) -> Vec<(u64, LogicalOperator)> {
    let mut scans = Vec::new();
    collect_scans_recursive(op, &mut scans);
    // Sort by cardinality ascending (smallest first)
    scans.sort_by_key(|(card, _)| *card);
    scans
}

/// Recursively collect all leaf scan operators from the tree.
fn collect_scans_recursive(op: &LogicalOperator, scans: &mut Vec<(u64, LogicalOperator)>) {
    match op {
        LogicalOperator::ScanNode(s) => {
            scans.push((s.cardinality, op.clone()));
        }
        LogicalOperator::ScanRel(s) => {
            scans.push((s.cardinality, op.clone()));
        }
        LogicalOperator::TableFunctionCall(tf) => {
            scans.push((tf.cardinality, op.clone()));
        }
        // Non-leaf operators: recurse into children
        LogicalOperator::Filter(f) => {
            for child in &f.children {
                collect_scans_recursive(child, scans);
            }
        }
        LogicalOperator::Projection(p) => {
            for child in &p.children {
                collect_scans_recursive(child, scans);
            }
        }
        LogicalOperator::HashJoin(hj) => {
            collect_scans_recursive(&hj.probe_side, scans);
            collect_scans_recursive(&hj.build_side, scans);
        }
        LogicalOperator::CrossProduct(cp) => {
            collect_scans_recursive(&cp.left, scans);
            collect_scans_recursive(&cp.right, scans);
        }
        LogicalOperator::OrderBy(o) => {
            for child in &o.children {
                collect_scans_recursive(child, scans);
            }
        }
        LogicalOperator::Limit(l) => {
            for child in &l.children {
                collect_scans_recursive(child, scans);
            }
        }
        LogicalOperator::Aggregate(a) => {
            for child in &a.children {
                collect_scans_recursive(child, scans);
            }
        }
        LogicalOperator::Union(u) => {
            collect_scans_recursive(&u.left, scans);
            collect_scans_recursive(&u.right, scans);
        }
        LogicalOperator::Flatten(f) => {
            for child in &f.children {
                collect_scans_recursive(child, scans);
            }
        }
        LogicalOperator::SemiJoin(sj) => {
            collect_scans_recursive(&sj.left, scans);
            collect_scans_recursive(&sj.right, scans);
        }
        LogicalOperator::AntiJoin(aj) => {
            collect_scans_recursive(&aj.left, scans);
            collect_scans_recursive(&aj.right, scans);
        }
        LogicalOperator::SemiMasker(s) => {
            for child in &s.children {
                collect_scans_recursive(child, scans);
            }
        }
        LogicalOperator::ArtIndexRangeScan(_)
        | LogicalOperator::VectorSimilarityScan(_)
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
        | LogicalOperator::Accumulate(_)
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
        | LogicalOperator::CreateDml(_)
        | LogicalOperator::ExportDatabase(_)
        | LogicalOperator::ImportDatabase(_) => {
            // Leaf operator with no children — nothing to recurse into.
        }
    }
}

/// Extract join conditions from the plan tree.
///
/// Looks for equality comparisons between different table aliases
/// in Filter operators embedded in the tree.
fn extract_join_conditions_from_tree(op: &LogicalOperator) -> Vec<(String, String)> {
    let mut conditions = Vec::new();
    extract_conditions_recursive(op, &mut conditions);
    conditions
}

/// Recursively find equality join conditions in Filter operators.
fn extract_conditions_recursive(op: &LogicalOperator, conditions: &mut Vec<(String, String)>) {
    match op {
        LogicalOperator::Filter(f) => {
            if let Some((left, right)) = extract_equality_join(&f.expression) {
                conditions.push((left, right));
            }
            for child in &f.children {
                extract_conditions_recursive(child, conditions);
            }
        }
        LogicalOperator::HashJoin(hj) => {
            extract_conditions_recursive(&hj.probe_side, conditions);
            extract_conditions_recursive(&hj.build_side, conditions);
        }
        LogicalOperator::CrossProduct(cp) => {
            extract_conditions_recursive(&cp.left, conditions);
            extract_conditions_recursive(&cp.right, conditions);
        }
        LogicalOperator::Projection(p) => {
            for child in &p.children {
                extract_conditions_recursive(child, conditions);
            }
        }
        _ => {}
    }
}

/// Try to extract an equality join condition between two table aliases.
fn extract_equality_join(expr: &kuzu_parser::ast::Expression) -> Option<(String, String)> {
    match expr {
        kuzu_parser::ast::Expression::BinaryOp(kuzu_parser::ast::BinaryOp::Equal, left, right) => {
            let left_var = extract_root_var(left);
            let right_var = extract_root_var(right);
            if let (Some(lv), Some(rv)) = (&left_var, &right_var)
                && lv != rv {
                    return Some((lv.clone(), rv.clone()));
                }
            None
        }
        _ => None,
    }
}

/// Extract root variable from a property access chain (e.g., `a.id` → `a`).
fn extract_root_var(expr: &kuzu_parser::ast::Expression) -> Option<String> {
    match expr {
        kuzu_parser::ast::Expression::Variable(name) => Some(name.clone()),
        kuzu_parser::ast::Expression::PropertyAccess(obj, _) => extract_root_var(obj),
        _ => None,
    }
}

/// Extract table alias from a scan operator.
fn get_scan_alias(op: &LogicalOperator) -> Option<String> {
    match op {
        LogicalOperator::ScanNode(s) => s.alias.clone().or_else(|| Some(s.table_name.clone())),
        LogicalOperator::ScanRel(s) => Some(s.table_name.clone()),
        LogicalOperator::TableFunctionCall(tf) => Some(tf.function_name.clone()),
        _ => None,
    }
}

/// Rebuild a join tree using greedy ordering based on cardinality.
///
/// Algorithm:
/// 1. Collect all leaf scan operators with their cardinalities
/// 2. Sort by cardinality ascending (smallest first)
/// 3. Build join tree greedily: start with smallest, join next smallest
/// 4. Use existing join conditions when available
///
/// Returns `None` if the plan doesn't contain joinable scans.
pub fn reorder_joins_greedy(root: &LogicalOperator) -> Option<Vec<LogicalOperator>> {
    let scans = collect_scans_sorted(root);
    if scans.len() < 2 {
        return None; // No joins to reorder
    }

    let join_conditions = extract_join_conditions_from_tree(root);
    let _alias_map: HashMap<Option<String>, usize> = scans
        .iter()
        .enumerate()
        .map(|(i, (_, op))| (get_scan_alias(op), i))
        .collect();

    let mut result_ops: Vec<LogicalOperator> = scans.into_iter().map(|(_, op)| op).collect();

    // Add join conditions as filters on top
    for (left_alias, right_alias) in &join_conditions {
        let expr = kuzu_parser::ast::Expression::BinaryOp(
            kuzu_parser::ast::BinaryOp::Equal,
            Box::new(kuzu_parser::ast::Expression::Variable(left_alias.clone())),
            Box::new(kuzu_parser::ast::Expression::Variable(right_alias.clone())),
        );
        result_ops.push(LogicalOperator::Filter(kuzu_planner::logical_operator::LogicalFilter {
            expression: expr,
            children: Vec::new(),
            cardinality: 0,
        }));
    }

    Some(result_ops)
}

const EQUALITY_PREDICATE_SELECTIVITY: f64 = 0.1;

#[derive(Clone, Debug)]
struct DpState {
    cost: f64,
    cardinality: f64,
    prev_mask: usize,
    last_idx: usize,
}

pub fn reorder_joins_dp_first(operators: &[LogicalOperator]) -> Option<Vec<LogicalOperator>> {
    let mut scans_with_pos: Vec<(usize, u64, LogicalOperator)> = operators
        .iter()
        .enumerate()
        .filter(|(_, op)| {
            matches!(
                op,
                LogicalOperator::ScanNode(_) | LogicalOperator::ScanRel(_) | LogicalOperator::TableFunctionCall(_)
            )
        })
        .map(|(i, op)| {
            let card = op.cardinality();
            (i, card, op.clone())
        })
        .collect();

    let n = scans_with_pos.len();
    if n < 2 {
        return None;
    }

    // Fallback to greedy if N > 32 to prevent bitmask overflow / exponential explosion
    if n > 32 {
        scans_with_pos.sort_by_key(|(_, card, _)| *card);
        let mut result: Vec<LogicalOperator> = operators.to_vec();
        let original_positions: Vec<usize> = scans_with_pos.iter().map(|(pos, _, _)| *pos).collect();
        for (new_idx, &old_pos) in original_positions.iter().enumerate() {
            result[old_pos] = scans_with_pos[new_idx].2.clone();
        }
        result.retain(|op| {
            !matches!(op, LogicalOperator::Filter(f) if crate::passes::is_join_condition(&f.expression))
        });
        return Some(result);
    }

    // Extract join conditions to build adjacency matrix
    let mut conditions = Vec::new();
    for op in operators {
        if let LogicalOperator::Filter(f) = op
            && let Some((left, right)) = extract_equality_join(&f.expression) {
                conditions.push((left, right));
            }
    }

    // Map aliases to scan indices
    let mut alias_to_idx = HashMap::new();
    for (i, (_, _, op)) in scans_with_pos.iter().enumerate() {
        if let Some(alias) = get_scan_alias(op) {
            alias_to_idx.insert(alias, i);
        }
    }

    // Adjacency matrix for joins
    let mut adj_mask = vec![0usize; n];
    for (left, right) in &conditions {
        if let (Some(&i), Some(&j)) = (alias_to_idx.get(left), alias_to_idx.get(right)) {
            adj_mask[i] |= 1 << j;
            adj_mask[j] |= 1 << i;
        }
    }

    // DP over subsets
    let max_mask = 1 << n;
    let mut dp = vec![None; max_mask];

    // Base cases (size 1)
    for i in 0..n {
        dp[1 << i] = Some(DpState {
            cost: 0.0,
            cardinality: scans_with_pos[i].1 as f64,
            prev_mask: 0,
            last_idx: i,
        });
    }

    // DP transitions
    for mask in 1..max_mask {
        let k = mask.count_ones();
        if k < 2 {
            continue;
        }
        
        let mut best_state: Option<DpState> = None;

        for i in 0..n {
            if (mask & (1 << i)) != 0 {
                let prev_mask = mask ^ (1 << i);
                if let Some(prev_state) = &dp[prev_mask] {
                    // Check if there is a join condition between prev_mask and i
                    let mut has_edge = false;
                    for j in 0..n {
                        if (prev_mask & (1 << j)) != 0 && (adj_mask[j] & (1 << i)) != 0 {
                            has_edge = true;
                            break;
                        }
                    }

                    let card_i = scans_with_pos[i].1 as f64;
                    let (new_card, new_cost) = if has_edge {
                        let c = prev_state.cardinality * card_i * EQUALITY_PREDICATE_SELECTIVITY;
                        (c, prev_state.cost + c)
                    } else {
                        // Cross product (heavily penalized)
                        let c = prev_state.cardinality * card_i;
                        (c, prev_state.cost + c + 1e9)
                    };

                    if best_state.as_ref().is_none_or(|b| new_cost < b.cost) {
                        best_state = Some(DpState {
                            cost: new_cost,
                            cardinality: new_card,
                            prev_mask,
                            last_idx: i,
                        });
                    }
                }
            }
        }
        dp[mask] = best_state;
    }

    // Reconstruct optimal order
    let mut order = Vec::with_capacity(n);
    let mut curr_mask = max_mask - 1;
    while curr_mask != 0 {
        let state = dp[curr_mask].as_ref().unwrap();
        order.push(state.last_idx);
        curr_mask = state.prev_mask;
    }
    order.reverse(); // Left-deep tree, so start from the smallest base mask

    // Check if the order is the same as the original (ascending)
    let already_ordered = order.windows(2).all(|w| w[0] <= w[1]);
    
    let original_positions: Vec<usize> = scans_with_pos.iter().map(|(pos, _, _)| *pos).collect();

    let mut result: Vec<LogicalOperator> = operators.to_vec();
    for (new_idx, &old_pos) in original_positions.iter().enumerate() {
        let ordered_idx = order[new_idx];
        result[old_pos] = scans_with_pos[ordered_idx].2.clone();
    }

    result.retain(|op| {
        !matches!(op, LogicalOperator::Filter(f)
            if crate::passes::is_join_condition(&f.expression)
        )
    });

    if already_ordered && result.len() == operators.len() {
        return None;
    }

    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_scans_empty() {
        let scans = collect_scans_sorted(&LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "A".into(),
            table_id: 0,
            alias: None,
            columns: vec![],
            cardinality: 100,
        }));
        assert_eq!(scans.len(), 1);
        assert_eq!(scans[0].0, 100);
    }

    #[test]
    fn test_collect_scans_sorted_by_cardinality() {
        let small = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "Small".into(),
            table_id: 0,
            alias: None,
            columns: vec![],
            cardinality: 10,
        });
        let large = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "Large".into(),
            table_id: 1,
            alias: None,
            columns: vec![],
            cardinality: 1000,
        });
        let join = LogicalOperator::HashJoin(LogicalHashJoin {
            join_keys: vec![],
            probe_side: Box::new(large),
            build_side: Box::new(small),
            cardinality: 0,
            push_down_eligible: false,
        });

        let scans = collect_scans_sorted(&join);
        assert_eq!(scans.len(), 2);
        // Smallest should be first
        assert_eq!(scans[0].0, 10);
        assert_eq!(scans[1].0, 1000);
    }

    #[test]
    fn test_reorder_no_join_needed() {
        let single = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "A".into(),
            table_id: 0,
            alias: None,
            columns: vec![],
            cardinality: 100,
        });
        assert!(reorder_joins_greedy(&single).is_none());
    }

    #[test]
    fn test_extract_join_conditions() {
        let expr = kuzu_parser::ast::Expression::BinaryOp(
            kuzu_parser::ast::BinaryOp::Equal,
            Box::new(kuzu_parser::ast::Expression::PropertyAccess(
                Box::new(kuzu_parser::ast::Expression::Variable("a".into())),
                "id".into(),
            )),
            Box::new(kuzu_parser::ast::Expression::PropertyAccess(
                Box::new(kuzu_parser::ast::Expression::Variable("b".into())),
                "id".into(),
            )),
        );
        let result = extract_equality_join(&expr);
        assert!(result.is_some());
        let (left, right) = result.unwrap();
        assert_eq!(left, "a");
        assert_eq!(right, "b");
    }

    #[test]
    fn test_extract_join_same_var_not_join() {
        let expr = kuzu_parser::ast::Expression::BinaryOp(
            kuzu_parser::ast::BinaryOp::Equal,
            Box::new(kuzu_parser::ast::Expression::Variable("a".into())),
            Box::new(kuzu_parser::ast::Expression::Variable("a".into())),
        );
        assert!(extract_equality_join(&expr).is_none());
    }

    #[test]
    fn test_get_scan_alias() {
        let scan = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("p".into()),
            columns: vec![],
            cardinality: 100,
        });
        assert_eq!(get_scan_alias(&scan), Some("p".into()));
    }

    #[test]
    fn test_reorder_joins_dp_prefers_join_over_cross_product() {
        let scan_a = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "A".into(),
            table_id: 0,
            alias: Some("a".into()),
            columns: vec![],
            cardinality: 100,
        });
        let scan_b = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "B".into(),
            table_id: 1,
            alias: Some("b".into()),
            columns: vec![],
            cardinality: 200,
        });
        let scan_c = LogicalOperator::ScanNode(LogicalScanNode {
            table_name: "C".into(),
            table_id: 2,
            alias: Some("c".into()),
            columns: vec![],
            cardinality: 300,
        });
        
        let filter_ac = LogicalOperator::Filter(kuzu_planner::logical_operator::LogicalFilter {
            expression: kuzu_parser::ast::Expression::BinaryOp(
                kuzu_parser::ast::BinaryOp::Equal,
                Box::new(kuzu_parser::ast::Expression::Variable("a".into())),
                Box::new(kuzu_parser::ast::Expression::Variable("c".into())),
            ),
            children: vec![],
            cardinality: 0,
        });
        
        let filter_bc = LogicalOperator::Filter(kuzu_planner::logical_operator::LogicalFilter {
            expression: kuzu_parser::ast::Expression::BinaryOp(
                kuzu_parser::ast::BinaryOp::Equal,
                Box::new(kuzu_parser::ast::Expression::Variable("b".into())),
                Box::new(kuzu_parser::ast::Expression::Variable("c".into())),
            ),
            children: vec![],
            cardinality: 0,
        });
        
        let operators = vec![scan_a, scan_b, scan_c, filter_ac, filter_bc];
        let result = reorder_joins_dp_first(&operators);
        assert!(result.is_some());
        
        let reordered = result.unwrap();
        // The scans could be C, A, B or A, C, B since both have same cost
        assert_eq!(reordered.len(), 3); // filters removed
        let first = get_scan_alias(&reordered[0]).unwrap();
        let second = get_scan_alias(&reordered[1]).unwrap();
        let third = get_scan_alias(&reordered[2]).unwrap();
        
        assert!((first == "a" && second == "c") || (first == "c" && second == "a"));
        assert_eq!(third, "b");
    }
}
