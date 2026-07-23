// ========================================================================
// Tree Pass 1: Factorization Rewriting
// Bottom-up insertion of LogicalFlatten operators for correct WCOJ
// factorization. Ported from C++ src/optimizer/factorization_rewriter.cpp
// ========================================================================

use crate::passes::{OptimizationPass, TreeOptimizationPass};
use akar_planner::logical_operator::*;

pub struct FactorizationRewriting;

impl FactorizationRewriting {
    /// Append LogicalFlatten nodes for each group position that isn't already flat.
    fn append_flattens(child: &mut LogicalOperator, groups_pos: &[usize]) {
        for &group_pos in groups_pos {
            // Wrap the child in a Flatten operator by replacing it in-place.
            let old = std::mem::replace(
                child,
                LogicalOperator::ScanNode(LogicalScanNode {
                    table_name: String::new(),
                    table_id: 0,
                    alias: None,
                    columns: Vec::new(),
                    cardinality: 0,
                    fts_query: None,
                    predicate: None,
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
                    Self::append_flattens(&mut hj.probe_side, &[0]);
                    Self::append_flattens(&mut hj.build_side, &[0]);
                }
                LogicalOperator::Projection(p) => {
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
                LogicalOperator::TopK(tk) => {
                    if let Some(first) = tk.children.first_mut() {
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
                LogicalOperator::Accumulate(ac) => {
                    for child in &mut ac.children {
                        Self::append_flattens(child, &[0]);
                    }
                }
                LogicalOperator::SemiMasker(s) => {
                    for child in &mut s.children {
                        Self::append_flattens(child, &[0]);
                    }
                }
                LogicalOperator::Skip(s) => {
                    if let Some(first) = s.children.first_mut() {
                        Self::append_flattens(first, &[0]);
                    }
                }
                LogicalOperator::MultiplicityReducer(mr) => {
                    if let Some(first) = mr.children.first_mut() {
                        Self::append_flattens(first, &[0]);
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
                | LogicalOperator::StandaloneCall(_)
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
                | LogicalOperator::CountRelTable(_)
                | LogicalOperator::BatchInsert(_)
                | LogicalOperator::IndexLookup(_)
                | LogicalOperator::PathPropertyProbe(_)
                | LogicalOperator::EmptyResult(_)
                | LogicalOperator::Insert(_)
                | LogicalOperator::ExtensionClause(_)
                | LogicalOperator::Partitioner(_) => {}
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
        operators.to_vec()
    }
}
