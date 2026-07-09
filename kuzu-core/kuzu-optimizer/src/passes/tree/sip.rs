// ========================================================================
// Pass: SIPOptimization
//
// Injects a LogicalSemiMasker into the build side of a HashJoin if the probe
// side is selective, enabling Sideways Information Passing.
// ========================================================================

use crate::passes::tree::acc_hash_join::has_filter_in_subtree;
use crate::passes::TreeOptimizationPass;
use kuzu_planner::logical_operator::*;

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
                        Box::new(LogicalOperator::ScanNode(LogicalScanNode {
                            table_name: String::new(),
                            table_id: 0,
                            alias: None,
                            columns: Vec::new(),
                            cardinality: 0,
                            fts_query: None,
                            predicate: None,
                        })),
                    );

                    let semi_masker = LogicalOperator::SemiMasker(LogicalSemiMasker {
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

fn find_scan_node(op: &LogicalOperator) -> Option<&LogicalScanNode> {
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
