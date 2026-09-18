// ========================================================================
// Pass 0b: Extend-Aware Filter Push-Down
//
// `FilterPushDown` only folds a filter into a scan when the filter is
// *adjacent* to that scan. A pattern like
//
//     MATCH (a:Memory {id: 1})-[r:Connected]->(b:Memory)
//
// is planned as `ScanNode(a) -> Extend -> Filter(a.id = 1) -> ...`: the
// `Extend` flushes the pending scan, so the anchor predicate is stranded
// *after* the hop and every input row is expanded through the whole
// relationship table before being discarded (F3 — "anchor doesn't help,
// cost ∝ rel size").
//
// This pass hoists each `Filter` that follows an `Extend` and references
// none of the variables the `Extend` introduces (`dst_node_var`/`rel_var`)
// to *before* that `Extend`. The subsequent `FilterPushDown` /
// `PredicatePushDown` / `ArtRangeScanDetection` passes can then fold the
// predicate into the scan and use the primary-key index, making an anchored
// hop cost O(degree) instead of O(rel).
//
// Soundness: the `Extend` only *adds* the `dst_node_var`/`rel_var` columns
// to each source row (it never removes or renames source columns), so a
// predicate over the pre-extend columns selects exactly the same source rows
// whether it runs before or after the expansion; dropping a source row early
// is equivalent to dropping all of its expansions.
// ========================================================================

use crate::passes::OptimizationPass;
use akar_planner::logical_operator::*;

use super::filter_pushdown::FilterPushDown;

pub struct ExtendFilterPushDown;

impl OptimizationPass for ExtendFilterPushDown {
    fn name(&self) -> &str {
        "extend_filter_push_down"
    }

    fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator> {
        let mut result: Vec<LogicalOperator> = Vec::with_capacity(operators.len());
        let mut i = 0;
        while i < operators.len() {
            if let LogicalOperator::Extend(extend) = &operators[i] {
                // Split the run of filters immediately after the Extend into
                // those that only touch pre-extend variables (safe to hoist)
                // and those that reference the hop's new columns (must stay).
                let mut hoisted: Vec<LogicalOperator> = Vec::new();
                let mut remaining: Vec<LogicalOperator> = Vec::new();
                let mut j = i + 1;
                while let Some(LogicalOperator::Filter(filter)) = operators.get(j) {
                    let vars = FilterPushDown::get_variables(&filter.expression);
                    let touches_new = vars.iter().any(|v| v == &extend.dst_node_var || v == &extend.rel_var);
                    if touches_new {
                        remaining.push(operators[j].clone());
                    } else {
                        hoisted.push(operators[j].clone());
                    }
                    j += 1;
                }

                // Only reorder when it actually buys something; otherwise keep
                // the original order untouched.
                if hoisted.is_empty() {
                    result.push(operators[i].clone());
                    i += 1;
                } else {
                    result.append(&mut hoisted);
                    result.push(operators[i].clone());
                    result.append(&mut remaining);
                    i = j;
                }
            } else {
                result.push(operators[i].clone());
                i += 1;
            }
        }
        result
    }
}
