// ========================================================================
// Tree Pass: Cardinality Estimation
// Bottom-up annotation of estimated row counts on each operator.
// Ported from C++ src/optimizer/cardinality_updater.cpp with static
// selectivity constants (no storage dependency).
// ========================================================================

use crate::passes::{OptimizationPass, TreeOptimizationPass};
use akar_planner::logical_operator::*;
use akar_storage::stats::StatsStore;
use std::sync::{Arc, Mutex};

/// Static selectivity constant (matching C++ PlannerKnobs).
const EQUALITY_PREDICATE_SELECTIVITY: f64 = 0.01;

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
                LogicalOperator::TopK(tk) => std::cmp::min(
                    tk.limit,
                    tk.children.first().map(|c| c.cardinality()).unwrap_or(u64::MAX),
                ),
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
                LogicalOperator::BatchInsert(bi) => bi.rows.len() as u64,
                LogicalOperator::IndexLookup(_) => 1, // point lookup = at most 1 row
                LogicalOperator::Delete(_) => 1000,   // estimated rows affected
                LogicalOperator::Set(_) => 1000,      // estimated rows updated
                LogicalOperator::OptionalMatch(om) => {
                    om.left.cardinality() // same as left (nullable)
                }
                LogicalOperator::Unwind(_) => 10, // list expansion estimate
                LogicalOperator::Foreach(_) => 1,
                LogicalOperator::Merge(_) => 1,      // single matched/created node
                LogicalOperator::MergeRel(_) => 1,   // single matched/created edge
                LogicalOperator::Explain(_) => 1,    // one row with plan text
                LogicalOperator::Intersect(_) => 10, // estimate: intersection reduces cardinality
                LogicalOperator::RecursiveExtend(re) => {
                    // estimate: upper_bound * source cardinality
                    let src_card = 100;
                    re.upper_bound.saturating_mul(src_card)
                }
                LogicalOperator::Accumulate(ac) => ac.children.first().map(|c| c.cardinality()).unwrap_or(1),
                LogicalOperator::ExpressionsScan(_) => 1,
                LogicalOperator::Partitioner(p) => p.children.first().map(|c| c.cardinality()).unwrap_or(1),
                LogicalOperator::Skip(s) => s.children.first().map(|c| c.cardinality()).unwrap_or(1),
                LogicalOperator::MultiplicityReducer(mr) => mr.children.first().map(|c| c.cardinality()).unwrap_or(1),
                LogicalOperator::EmptyResult(_) => 0,
                LogicalOperator::Insert(i) => i.values.len() as u64,
                LogicalOperator::ExtensionClause(_) => 1,
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
                | LogicalOperator::PathPropertyProbe(_)
                | LogicalOperator::CountRelTable(_)
                | LogicalOperator::StandaloneCall(_) => 1,
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
