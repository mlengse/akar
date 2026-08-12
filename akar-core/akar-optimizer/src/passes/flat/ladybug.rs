//! Ladybug optimizer passes: OrderByPushDown, UnwindDedup, CountRelTable.

use crate::passes::OptimizationPass;
use akar_planner::logical_operator::*;

// ========================================================================
// Pass: OrderBy Push-Down (Ladybug)
// ========================================================================
//
// This pass is a NO-OP. The original implementation replaced `[Union, OrderBy]`
// with a Union whose branches were each wrapped in a malformed nested
// Projection (`expressions: vec![]` + two children) and DROPPED the top-level
// ORDER BY. Because UNION execution is a plain concatenation (no merge), sorting
// each branch independently does NOT produce a globally sorted result â€” the
// query silently lost its ordering. Correct push-down would require a
// merge-sorted UNION; without it the global ORDER BY is kept untouched.

pub struct OrderByPushDown;

impl OptimizationPass for OrderByPushDown {
    fn name(&self) -> &str {
        "order_by_push_down"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        operators.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_order_by_push_down_keeps_global_order_by() {
        let pass = OrderByPushDown;
        let plan = vec![
            LogicalOperator::Union(LogicalUnion {
                left: Box::new(LogicalOperator::ScanRel(LogicalScanRel {
                    table_name: "R1".into(),
                    table_id: 0,
                    direction: akar_parser::ast::EdgeDirection::LeftToRight,
                    cardinality: 10,
                })),
                right: Box::new(LogicalOperator::ScanRel(LogicalScanRel {
                    table_name: "R2".into(),
                    table_id: 1,
                    direction: akar_parser::ast::EdgeDirection::LeftToRight,
                    cardinality: 10,
                })),
                all: true,
                cardinality: 20,
            }),
            LogicalOperator::OrderBy(LogicalOrderBy {
                sort_keys: vec![(akar_parser::ast::Expression::Variable("x".into()), true)],
                children: vec![],
                cardinality: 20,
            }),
        ];
        let result = pass.apply(&plan);
        assert_eq!(result.len(), 2, "global ORDER BY after UNION must be preserved");
        assert!(matches!(result[0], LogicalOperator::Union(_)));
        assert!(matches!(result[1], LogicalOperator::OrderBy(_)));
    }

    fn unwind_op(var: &str) -> LogicalOperator {
        LogicalOperator::Unwind(LogicalUnwind {
            expression: akar_parser::ast::Expression::List(vec![
                akar_parser::ast::Expression::Constant(akar_parser::ast::Constant::Integer(1)),
                akar_parser::ast::Expression::Constant(akar_parser::ast::Constant::Integer(2)),
            ]),
            variable: var.to_string(),
            cardinality: 0,
        })
    }

    #[test]
    fn test_unwind_dedup_keeps_different_variables() {
        // P52.54: `UNWIND [1,2] AS a` then `UNWIND [1,2] AS b` — same list, different
        // variables. The old expression-only key collapsed them, losing column b.
        let pass = UnwindDedup;
        let plan = vec![unwind_op("a"), unwind_op("b")];
        let result = pass.apply(&plan);
        assert_eq!(result.len(), 2, "different UNWIND variables must both be kept");
    }

    #[test]
    fn test_unwind_dedup_merges_consecutive_same_key() {
        let pass = UnwindDedup;
        let plan = vec![unwind_op("a"), unwind_op("a")];
        let result = pass.apply(&plan);
        assert_eq!(result.len(), 1, "consecutive identical UNWINDs are merged");
    }

    #[test]
    fn test_unwind_dedup_does_not_collapse_non_adjacent() {
        // P52.54: two identical UNWINDs separated by another operator are genuine
        // separate executions and must not be deduped (the old HashSet-based dedup
        // removed the non-consecutive one).
        let pass = UnwindDedup;
        let plan = vec![
            unwind_op("a"),
            LogicalOperator::Limit(LogicalLimit {
                limit: 1,
                offset: 0,
                children: vec![],
                cardinality: 0,
            }),
            unwind_op("a"),
        ];
        let result = pass.apply(&plan);
        assert_eq!(result.len(), 3, "non-adjacent UNWINDs must both be kept");
    }
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
        let mut i = 0;
        while i < operators.len() {
            match &operators[i] {
                LogicalOperator::Unwind(uw) => {
                    // Only merge CONSECUTIVE UNWINDs on the same (variable,
                    // expression) pair. The key must include the variable:
                    // two `UNWIND [1,2]` into different variables are NOT
                    // duplicates (dropping one would remove a column a consumer
                    // needs). Using a global HashSet also wrongly collapsed
                    // non-adjacent UNWINDs (P52.54).
                    let key = (uw.variable.clone(), format!("{:?}", uw.expression));
                    let is_adjacent_duplicate = matches!(
                        result.last(),
                        Some(LogicalOperator::Unwind(prev))
                            if (prev.variable.clone(), format!("{:?}", prev.expression)) == key
                    );
                    if is_adjacent_duplicate {
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
