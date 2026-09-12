//! FTS predicate push-down — ensure a `LogicalScanNode` carrying an FTS query
//! filters the scan of the table the index was built on, never a sibling scan
//! of a different table (P108.1).
//!
//! The `USING FTS INDEX` clause is bound onto the whole MATCH and the planner
//! attaches it to a scan via table-name routing. This pass is the defence-in-depth
//! guarantee: it walks the logical tree, detects any scan whose `fts_query`
//! targets a different table than the node's own, and re-routes the query to the
//! scan of the correct table — so document-id filtering runs at the correct leaf,
//! i.e. before any join or graph traversal above it. If no scan of the target
//! table exists in the plan, the misplaced query is detached rather than applied
//! to the wrong rows.

use crate::passes::TreeOptimizationPass;
use akar_planner::logical_operator::{LogicalFtsScan, LogicalOperator};

/// Re-routes misplaced FTS queries to the scan of the index's base table.
pub struct FtsPredicatePushdown;

impl TreeOptimizationPass for FtsPredicatePushdown {
    fn name(&self) -> &str {
        "fts_predicate_pushdown"
    }

    fn apply_tree(&self, root: &mut LogicalOperator) {
        // Detach any FTS queries sitting on scans of the wrong table.
        let mut displaced = Vec::new();
        collect_displaced_fts(root, &mut displaced);

        for (_, target_table, fts) in displaced {
            if !attach_fts(root, &target_table, &fts) {
                // No scan of the FTS table exists (e.g. the table is only reachable
                // as an edge destination produced by an Extend). Drop the query
                // rather than filter the wrong table's rows.
                tracing::debug!(
                    "FtsPredicatePushdown: dropping FTS on index `{}` — no scan of table `{}` in plan",
                    fts.index_name,
                    target_table
                );
            }
        }
    }
}

/// Collect `(scan_table, fts_table, fts)` for every scan whose `fts_query`
/// targets a different table (the query is detached in the process).
fn collect_displaced_fts(op: &mut LogicalOperator, out: &mut Vec<(String, String, LogicalFtsScan)>) {
    match op {
        LogicalOperator::ScanNode(s) => {
            if let Some(fq) = s.fts_query.take() {
                if s.table_name == fq.table_name {
                    // Already on the correct scan — keep it.
                    s.fts_query = Some(fq);
                } else {
                    out.push((s.table_name.clone(), fq.table_name.clone(), fq));
                }
            }
        }
        _ => {
            for child in op.children_mut() {
                collect_displaced_fts(child, out);
            }
        }
    }
}

/// Attach `fts` to the first scan of `target_table` that does not already carry
/// an FTS query. Returns `true` when a target scan was found.
fn attach_fts(op: &mut LogicalOperator, target_table: &str, fts: &LogicalFtsScan) -> bool {
    if let LogicalOperator::ScanNode(s) = op {
        if s.table_name == target_table && s.fts_query.is_none() {
            s.fts_query = Some(fts.clone());
            return true;
        }
        return false;
    }
    for child in op.children_mut() {
        if attach_fts(child, target_table, fts) {
            return true;
        }
    }
    false
}
