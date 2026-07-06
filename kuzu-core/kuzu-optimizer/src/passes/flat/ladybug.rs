//! Ladybug optimizer passes: OrderByPushDown, UnwindDedup, CountRelTable.

use crate::passes::OptimizationPass;
use kuzu_planner::logical_operator::*;
use std::collections::HashSet;

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
        let mut seen_unwinds: HashSet<String> = HashSet::new();
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
