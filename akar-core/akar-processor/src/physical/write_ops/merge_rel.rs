//! Physical operator for edge MERGE (P53.20): `MERGE (a)-[r:R {..}]->(b)`.

use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::physical::write_ops::set::{PhysicalSet, evaluate_expression_for_row};
use akar_common::error::ProcessorError;
use akar_common::types::{PhysicalTypeID, Value};
use akar_common::vector::{DataChunk, ValueVector};
use akar_parser::ast::{EdgeDirection, Expression};
use akar_storage::table::TableCatalog;
use akar_transaction::UndoRecord;
use std::sync::{Arc, Mutex};

/// Physical operator for MERGE on an edge pattern.
///
/// For each input row (a bound `src`/`dst` node pair, resolved via the
/// `<src>._id` / `<dst>._id` columns), it matches an existing edge on the rel
/// table whose endpoints and pattern properties match. If none exists, a new
/// edge is inserted. Emits one output column `<edge_var>._id` carrying the
/// matched/inserted edge index per row, so a following `SET <edge_var>.x`
/// clause can target the right rows.
pub struct PhysicalMergeRel {
    pub rel_table_name: String,
    pub rel_table_id: u64,
    pub edge_var: String,
    pub src_node_var: String,
    pub dst_node_var: String,
    pub direction: EdgeDirection,
    pub properties: Vec<(String, Expression)>,
    pub on_match: Vec<PhysicalSet>,
    pub on_create: Vec<PhysicalSet>,
    pub table_catalog: Arc<TableCatalog>,
    /// Active transaction id (P52.18).
    pub txn_id: Option<u64>,
    /// Undo sink for rollback records (P52.18).
    pub undo_sink: Option<Arc<Mutex<Vec<UndoRecord>>>>,
}

impl PhysicalOperatorExec for PhysicalMergeRel {
    fn operator_type(&self) -> &str {
        "merge_rel"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        if input.is_empty() || input.iter().all(|c| c.size == 0) {
            return Ok(input);
        }

        // Owned snapshot of the rel table adjacency + columns (P53.14 style).
        let (fwd_adj, rev_adj, cols) = {
            let rel_table = self
                .table_catalog
                .get_rel_table_by_name(&self.rel_table_name)
                .ok_or_else(|| format!("Rel table '{}' not found", self.rel_table_name))?;
            (
                rel_table.fwd_adj.clone(),
                rel_table.rev_adj.clone(),
                rel_table.columns.clone(),
            )
        };

        let mut output = Vec::with_capacity(input.len());

        for chunk in input {
            let src_col = chunk
                .field_names
                .iter()
                .position(|n| n == &format!("{}.{}", self.src_node_var, "_id"))
                .ok_or_else(|| {
                    format!(
                        "Bound node '{}' not found in MERGE input. Available fields: {:?}",
                        self.src_node_var, chunk.field_names
                    )
                })?;
            let dst_col = chunk
                .field_names
                .iter()
                .position(|n| n == &format!("{}.{}", self.dst_node_var, "_id"))
                .ok_or_else(|| {
                    format!(
                        "Bound node '{}' not found in MERGE input. Available fields: {:?}",
                        self.dst_node_var, chunk.field_names
                    )
                })?;

            let mut matched_idx: Vec<Option<u64>> = Vec::with_capacity(chunk.size);
            let mut matched: Vec<u64> = Vec::new();
            let mut created: Vec<u64> = Vec::new();

            for row in 0..chunk.size {
                let src_id = match chunk.get_value(src_col, row) {
                    Some(Value::Int64(v)) => v as u64,
                    _ => continue,
                };
                let dst_id = match chunk.get_value(dst_col, row) {
                    Some(Value::Int64(v)) => v as u64,
                    _ => continue,
                };

                let candidates: Vec<(u64, usize)> = match self.direction {
                    EdgeDirection::LeftToRight => fwd_adj.get(&src_id).cloned().unwrap_or_default(),
                    EdgeDirection::RightToLeft => rev_adj.get(&src_id).cloned().unwrap_or_default(),
                    EdgeDirection::Both => {
                        let mut all = fwd_adj.get(&src_id).cloned().unwrap_or_default();
                        if let Some(rev) = rev_adj.get(&src_id) {
                            all.extend(rev.iter().cloned());
                        }
                        all
                    }
                };

                let mut found: Option<usize> = None;
                for &(dst_offset, edge_idx) in &candidates {
                    if dst_offset != dst_id {
                        continue;
                    }
                    if self.props_match(edge_idx, &cols, &chunk, row)? {
                        found = Some(edge_idx);
                        break;
                    }
                }

                match found {
                    Some(edge_idx) => {
                        matched.push(edge_idx as u64);
                        matched_idx.push(Some(edge_idx as u64));
                    }
                    None => {
                        let mut values: Vec<Value> = vec![Value::Null; cols.len()];
                        for (prop_name, prop_expr) in &self.properties {
                            if let Some(col_idx) = cols.iter().position(|c| c.name == *prop_name) {
                                values[col_idx] = evaluate_expression_for_row(prop_expr, &chunk, row);
                            }
                        }
                        let edge_idx = self.insert_rel(src_id, dst_id, values)?;
                        created.push(edge_idx);
                        matched_idx.push(Some(edge_idx));
                    }
                }
            }

            // Emit `<edge_var>._id` so a following SET targets these edges.
            let edge_count = matched_idx.len();
            let mut v = ValueVector::new(PhysicalTypeID::Int64, edge_count);
            v.resize(edge_count);
            for (i, e) in matched_idx.iter().enumerate() {
                if let Some(idx) = e {
                    v.set_i64(i, *idx as i64);
                } else {
                    v.set_null(i, true);
                }
            }
            let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&v).array;
            let out = DataChunk::new(vec![arr], vec![PhysicalTypeID::Int64])
                .with_names(vec![format!("{}.{}", self.edge_var, "_id")]);
            output.push(out);

            if !matched.is_empty() {
                self.apply_on_clause(&self.on_match, &matched)?;
            }
            if !created.is_empty() {
                self.apply_on_clause(&self.on_create, &created)?;
            }
        }

        Ok(output)
    }
}

impl PhysicalMergeRel {
    /// Evaluate the pattern's inline properties against the row and compare
    /// them with the candidate edge's stored property values.
    fn props_match(
        &self,
        edge_idx: usize,
        cols: &[akar_storage::table::ColumnDefinition],
        chunk: &DataChunk,
        row: usize,
    ) -> Result<bool, ProcessorError> {
        let rel_table = self
            .table_catalog
            .get_rel_table_by_name(&self.rel_table_name)
            .ok_or_else(|| format!("Rel table '{}' not found", self.rel_table_name))?;
        let props = rel_table.get_edge_properties(edge_idx);
        for (prop_name, prop_expr) in &self.properties {
            let col_idx = cols
                .iter()
                .position(|c| c.name == *prop_name)
                .ok_or_else(|| format!("Rel column '{prop_name}' not found"))?;
            let expected = evaluate_expression_for_row(prop_expr, chunk, row);
            let actual = props.get(col_idx).cloned().unwrap_or(Value::Null);
            if expected != actual {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Insert a new edge, recording an undo record.
    fn insert_rel(&self, src_id: u64, dst_id: u64, values: Vec<Value>) -> Result<u64, ProcessorError> {
        let (edge_idx, table_id) = {
            let mut rel_table = self
                .table_catalog
                .get_rel_table_by_name_mut(&self.rel_table_name)
                .ok_or_else(|| format!("Rel table '{}' not found", self.rel_table_name))?;
            let edge_idx = rel_table.edges.len() as u64;
            rel_table
                .insert_rel(src_id, dst_id, values)
                .map_err(|e| format!("MERGE CREATE edge failed: {e}"))?;
            (edge_idx, rel_table.table_id)
        };
        if let Some(sink) = self.undo_sink.as_ref()
            && let Ok(mut u) = sink.lock()
        {
            u.push(UndoRecord::insert(table_id, edge_idx));
        }
        Ok(edge_idx)
    }

    /// Apply `ON MATCH SET` / `ON CREATE SET` operations against the given
    /// edge indices. Each SET op reads the `<edge_var>._id` column, so only
    /// the edge rows emitted by this merge are touched.
    fn apply_on_clause(&self, set_ops: &[PhysicalSet], edge_ids: &[u64]) -> Result<(), ProcessorError> {
        if set_ops.is_empty() || edge_ids.is_empty() {
            return Ok(());
        }
        let n = edge_ids.len();
        let mut v = ValueVector::new(PhysicalTypeID::Int64, n);
        v.resize(n);
        for (i, e) in edge_ids.iter().enumerate() {
            v.set_i64(i, *e as i64);
        }
        let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&v).array;
        let chunk = DataChunk::new(vec![arr], vec![PhysicalTypeID::Int64])
            .with_names(vec![format!("{}.{}", self.edge_var, "_id")]);
        for set_op in set_ops {
            let _ = set_op.execute(vec![chunk.clone()])?;
        }
        Ok(())
    }
}
