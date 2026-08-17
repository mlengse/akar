//! Auto-extracted from physical_operator.rs
use crate::expression_evaluator::ExpressionEvaluator;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::physical::write_ops::delete::{ast_constant_to_value, row_id_column_index};
use akar_common::arrow_vector::VectorAccess;
use akar_common::types::Value;
use akar_common::types::{PhysicalTypeID, physical_type_from_logical};
use akar_common::vector::{DataChunk, ValueVector};
use akar_function::registry::FunctionRegistry;
use akar_function::scalar::evaluate_scalar;
use akar_parser::ast::{BinaryOp, Expression};
use akar_planner::logical_operator::SetItem;
use akar_storage::table::{ColumnDefinition, TableCatalog};
use akar_transaction::UndoRecord;
use std::sync::{Arc, Mutex};

// ==================== Set ====================

/// Physical operator for SET — updates properties on matched rows.
///
/// All items of a SET clause are evaluated against the SAME pre-update
/// snapshot of the table, then written (atomic semantics, P53.17). This means
/// `SET n.a = n.a + 1, n.b = n.a * 10` computes both RHS values from the
/// pre-update `n.a`, matching Cypher/Neo4j behavior.
pub struct PhysicalSet {
    pub table_name: String,
    pub table_id: u64,
    pub is_node: bool,
    pub items: Vec<SetItem>,
    pub table_catalog: Arc<TableCatalog>,
    /// Active transaction id (P52.18).
    pub txn_id: Option<u64>,
    /// Undo sink for rollback records (P52.18).
    pub undo_sink: Option<Arc<Mutex<Vec<UndoRecord>>>>,
    /// Function registry for evaluating non-constant SET value expressions
    /// (arithmetic, property reads, function calls) against old row data (P53.17).
    pub function_registry: Option<Arc<Mutex<FunctionRegistry>>>,
}

impl PhysicalOperatorExec for PhysicalSet {
    fn operator_type(&self) -> &str {
        "set"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        // Collect row indices from input chunks. The scan emits the physical
        // row index as the `<alias>._id` column (last column); reading column 0
        // would treat the first *property* value as a row index.
        let mut rows_to_update: Vec<u64> = Vec::new();
        // For each target row, remember which input (chunk, row) it came from so
        // the snapshot can carry pipeline-only columns (e.g. an UNWIND variable)
        // into the SET value expressions (P53.26).
        let mut source_rows: Vec<(usize, usize)> = Vec::new();

        for (ci, chunk) in input.iter().enumerate() {
            let row_id_col = row_id_column_index(chunk);
            for row in 0..chunk.size {
                if !chunk.fields.is_empty()
                    && let Some(Value::Int64(val)) = chunk.get_value(row_id_col.unwrap_or(0), row)
                {
                    rows_to_update.push(val as u64);
                    source_rows.push((ci, row));
                }
            }
        }

        if rows_to_update.is_empty() {
            return Ok(vec![count_chunk(0)]);
        }

        // Build ONE pre-update snapshot chunk from the table for all target rows.
        let snapshot = self.build_snapshot_chunk(&rows_to_update, &input, &source_rows)?;

        // Evaluate every item against the SAME snapshot (P53.17). This reads
        // true pre-write values: `SET n.x = n.x + 1` increments, and a
        // multi-item `SET n.a = ..., n.b = n.a * 10` computes `n.b` from the
        // pre-update `n.a`, matching Cypher/Neo4j semantics.
        let mut all_values: Vec<Vec<Value>> = Vec::with_capacity(self.items.len());
        for item in &self.items {
            all_values.push(self.evaluate_item(item, &snapshot)?);
        }

        // Apply updates to the table
        let mut updated = 0u64;
        if self.is_node {
            if let Some(mut table) = self.table_catalog.get_node_table_by_name_mut(&self.table_name) {
                for (item_idx, item) in self.items.iter().enumerate() {
                    // The binder hardcodes `column_idx: 0` ("resolved by catalog
                    // lookup") but never resolves it — resolve by name at runtime
                    // so `SET n.prop = v` writes to `prop`, not column 0.
                    let col_idx = table
                        .columns
                        .iter()
                        .position(|c| c.name == item.column_name)
                        .unwrap_or(item.column_idx);
                    for (i, row_idx) in rows_to_update.iter().enumerate() {
                        // Capture the pre-update cell for rollback (P52.18).
                        if let Some(sink) = self.undo_sink.as_ref()
                            && let Ok(mut u) = sink.lock()
                        {
                            let old_data = table.cell_undo_bytes(*row_idx, col_idx);
                            u.push(UndoRecord::update(self.table_id, *row_idx, col_idx as u32, old_data));
                        }
                        if table
                            .update_cell(*row_idx, col_idx, all_values[item_idx][i].clone())
                            .is_ok()
                        {
                            updated += 1;
                        }
                    }
                }
            } else {
                return Err(format!("Node table '{}' not found for SET", self.table_name).into());
            }
        } else {
            if let Some(mut table) = self.table_catalog.get_rel_table_by_name_mut(&self.table_name) {
                for (item_idx, item) in self.items.iter().enumerate() {
                    let col_idx = table
                        .columns
                        .iter()
                        .position(|c| c.name == item.column_name)
                        .unwrap_or(item.column_idx);
                    for (i, edge_idx) in rows_to_update.iter().enumerate() {
                        if let Some(sink) = self.undo_sink.as_ref()
                            && let Ok(mut u) = sink.lock()
                        {
                            let old_data = table.edge_cell_undo_bytes(*edge_idx as usize, col_idx);
                            u.push(UndoRecord::update(self.table_id, *edge_idx, col_idx as u32, old_data));
                        }
                        if table
                            .update_cell(*edge_idx as usize, col_idx, all_values[item_idx][i].clone())
                            .is_ok()
                        {
                            updated += 1;
                        }
                    }
                }
            } else {
                return Err(format!("Rel table '{}' not found for SET", self.table_name).into());
            }
        }

        tracing::info!("SET: updated {updated} rows in '{}'", self.table_name);

        // Carry the updated rows forward (P53.30): [count, <table columns>,
        // <pipeline columns>, <_id>]. Column 0 keeps the updated count so
        // `get_i64(0, 0)` checks stay valid; a following RETURN resolves
        // `<alias>.prop` against the named table columns instead of the count.
        let mut output = self.build_output_chunk(&rows_to_update, &input, &source_rows)?;
        let mut count_v = ValueVector::new(PhysicalTypeID::Int64, rows_to_update.len());
        count_v.resize(rows_to_update.len());
        for i in 0..rows_to_update.len() {
            count_v.set_i64(i, updated as i64);
        }
        output
            .fields
            .insert(0, akar_common::arrow_vector::ArrowVector::from_legacy(&count_v).array);
        output.field_types.insert(0, PhysicalTypeID::Int64);
        output.field_names.insert(0, String::new());
        Ok(vec![output])
    }
}

fn count_chunk(count: u64) -> DataChunk {
    let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
    v.resize(1);
    v.set_i64(0, count as i64);
    let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&v).array;
    DataChunk::new(vec![arr], vec![PhysicalTypeID::Int64])
}

impl PhysicalSet {
    /// Build a `DataChunk` of the target rows' pre-update cell values, one
    /// column per table column, keyed by the physical row offsets in `rows`.
    /// Pipeline-only columns from the input chunks (e.g. an UNWIND variable)
    /// are appended so SET value expressions can reference them (P53.26).
    fn build_snapshot_chunk(
        &self,
        rows: &[u64],
        input: &[DataChunk],
        source_rows: &[(usize, usize)],
    ) -> Result<DataChunk, String> {
        let mut snapshot = if self.is_node {
            let table = self
                .table_catalog
                .get_node_table_by_name(&self.table_name)
                .ok_or_else(|| format!("Node table '{}' not found for SET", self.table_name))?;
            build_old_row_chunk(&table.columns, rows, &|row, col| {
                table.get_value(row as usize, col).cloned()
            })?
        } else {
            let table = self
                .table_catalog
                .get_rel_table_by_name(&self.table_name)
                .ok_or_else(|| format!("Rel table '{}' not found for SET", self.table_name))?;
            build_old_row_chunk(&table.columns, rows, &|row, col| {
                table.get_edge_properties(row as usize).get(col).cloned()
            })?
        };
        append_pipeline_columns(&mut snapshot, input, source_rows)?;
        Ok(snapshot)
    }
}

/// Append input-chunk columns that are not table columns (nor the internal
/// `_id` pseudo-column) to the chunk, aligned by the source row index of each
/// target row. This makes UNWIND variables visible to later clauses, e.g.
/// `UNWIND $ids AS iid ... SET n.x = iid` (P53.26).
pub(crate) fn append_pipeline_columns(
    snapshot: &mut DataChunk,
    input: &[DataChunk],
    source_rows: &[(usize, usize)],
) -> Result<(), String> {
    let n = source_rows.len();
    for (ci, chunk) in input.iter().enumerate() {
        let mut appended: Vec<(usize, String)> = Vec::new();
        for (col_idx, name) in chunk.field_names.iter().enumerate() {
            if name == "_id" || name.ends_with("._id") {
                continue;
            }
            if snapshot.field_names.iter().any(|existing| existing == name) {
                continue;
            }
            // Qualified copies (r.weight) whose base matches a plain snapshot
            // column (weight) are pre-write stale reads from the pipeline; the
            // evaluator's bare-name fallback resolves to the fresh table column
            // instead, so a following `RETURN r.weight` sees the written value
            // (P53.37a).
            if let Some((_, base)) = name.rsplit_once('.') {
                if snapshot.field_names.iter().any(|existing| existing == base) {
                    continue;
                }
            }
            appended.push((col_idx, name.clone()));
        }
        if appended.is_empty() {
            continue;
        }

        for (col_idx, name) in appended {
            // Gather each target row's value from this input chunk.
            let mut col_values: Vec<Value> = vec![Value::Null; n];
            for (i, &(cci, rowi)) in source_rows.iter().enumerate() {
                if cci == ci {
                    col_values[i] = chunk.get_value(col_idx, rowi).unwrap_or(Value::Null);
                }
            }
            // Build via Arrow so complex values (map/struct) survive the
            // round-trip; `ValueVector::set_value` rejects them.
            let phys_type = col_values
                .iter()
                .find(|v| !matches!(v, Value::Null))
                .map(|v| v.physical_type())
                .unwrap_or(chunk.field_types[col_idx]);
            let arr = crate::expression_evaluator::build_arrow_from_values(&col_values, phys_type, n)
                .map_err(|e| e.to_string())?;
            snapshot.fields.push(arr.array);
            snapshot.field_types.push(arr.physical_type);
            snapshot.field_names.push(name);
        }
    }
    Ok(())
}

impl PhysicalSet {
    /// Build the SET operator's output chunk: the post-update table columns
    /// (named, so a following RETURN can resolve `<alias>.<prop>`), the input
    /// pipeline columns (e.g. UNWIND variables), and the `_id` pseudo-column
    /// with the physical row indices (so a following write op can re-target the
    /// same rows). Previously SET returned only a count chunk, so `MATCH ... SET
    /// ... RETURN n.prop` evaluated the projection against the count (P53.30).
    fn build_output_chunk(
        &self,
        rows: &[u64],
        input: &[DataChunk],
        source_rows: &[(usize, usize)],
    ) -> Result<DataChunk, String> {
        let mut chunk = if self.is_node {
            let table = self
                .table_catalog
                .get_node_table_by_name(&self.table_name)
                .ok_or_else(|| format!("Node table '{}' not found for SET", self.table_name))?;
            build_old_row_chunk(&table.columns, rows, &|row, col| {
                table.get_value(row as usize, col).cloned()
            })?
        } else {
            let table = self
                .table_catalog
                .get_rel_table_by_name(&self.table_name)
                .ok_or_else(|| format!("Rel table '{}' not found for SET", self.table_name))?;
            build_old_row_chunk(&table.columns, rows, &|row, col| {
                table.get_edge_properties(row as usize).get(col).cloned()
            })?
        };
        append_pipeline_columns(&mut chunk, input, source_rows)?;
        // Append the `_id` pseudo-column (physical row indices) so a following
        // write op can target the same rows.
        let mut v = ValueVector::new(PhysicalTypeID::Int64, rows.len());
        v.resize(rows.len());
        for (i, r) in rows.iter().enumerate() {
            v.set_i64(i, *r as i64);
        }
        chunk
            .fields
            .push(akar_common::arrow_vector::ArrowVector::from_legacy(&v).array);
        chunk.field_types.push(PhysicalTypeID::Int64);
        chunk.field_names.push("_id".to_string());
        Ok(chunk)
    }

    /// Evaluate one SET item's value expression for every row of the snapshot
    /// chunk, returning one `Value` per row.
    fn evaluate_item(&self, item: &SetItem, chunk: &DataChunk) -> Result<Vec<Value>, String> {
        if let Some(registry) = self.function_registry.as_ref() {
            let evaluator = ExpressionEvaluator::new(registry.clone());
            // Arrow-native evaluation so complex literals (list/map) survive the
            // round-trip: `evaluate` builds a legacy ValueVector with no List
            // storage, which silently produces `Value::Null` (P53.29).
            let vec = evaluator
                .evaluate_to_arrow(&item.value, chunk)
                .map_err(|e| e.to_string())?;
            Ok((0..chunk.size)
                .map(|i| vec.get_value(i).unwrap_or(Value::Null))
                .collect())
        } else {
            Ok((0..chunk.size)
                .map(|i| evaluate_expression_for_row(&item.value, chunk, i))
                .collect())
        }
    }
}

/// Build a `DataChunk` of the target rows' cell values, one column per table
/// column, using the plain column names as `field_names` so the evaluator can
/// resolve `<alias>.<prop>` and `<prop>` references.
///
/// The chunk is built via Arrow (`build_arrow_from_values`) rather than
/// `ValueVector::set_value` so complex values (map/struct/list, e.g. FLOAT[]
/// embeddings) survive the round-trip — the legacy vector has no List arm and
/// silently produced `Value::Null` (P53.29).
pub(crate) fn build_old_row_chunk(
    columns: &[ColumnDefinition],
    rows: &[u64],
    get_cell: &dyn Fn(u64, usize) -> Option<Value>,
) -> Result<DataChunk, String> {
    let n = rows.len();
    let mut fields = Vec::with_capacity(columns.len());
    let mut field_types = Vec::with_capacity(columns.len());
    let mut field_names = Vec::with_capacity(columns.len());
    for (col_idx, col) in columns.iter().enumerate() {
        let phys_type = physical_type_from_logical(col.logical_type);
        let col_values: Vec<Value> = rows
            .iter()
            .map(|row| get_cell(*row, col_idx).unwrap_or(Value::Null))
            .collect();
        let arr = crate::expression_evaluator::build_arrow_from_values(&col_values, phys_type, n)
            .map_err(|e| e.to_string())?;
        fields.push(arr.array);
        field_types.push(phys_type);
        field_names.push(col.name.clone());
    }
    Ok(DataChunk::new(fields, field_types).with_names(field_names))
}

/// Simple expression evaluator for SET value expressions against a DataChunk row.
pub fn evaluate_expression_for_row(
    expr: &akar_parser::ast::Expression,
    chunk: &DataChunk,
    row: usize,
) -> akar_common::types::Value {
    match expr {
        akar_parser::ast::Expression::Constant(c) => match c {
            akar_parser::ast::Constant::Null => akar_common::types::Value::Null,
            akar_parser::ast::Constant::Bool(b) => akar_common::types::Value::Bool(*b),
            akar_parser::ast::Constant::Integer(i) => akar_common::types::Value::Int64(*i),
            akar_parser::ast::Constant::Float(f) => akar_common::types::Value::Double(*f),
            akar_parser::ast::Constant::String(s) => akar_common::types::Value::String(s.clone()),
        },
        akar_parser::ast::Expression::Variable(name) => chunk
            .field_names
            .iter()
            .position(|n| n == name)
            .and_then(|i| chunk.get_value(i, row))
            .unwrap_or(akar_common::types::Value::Null),
        akar_parser::ast::Expression::PropertyAccess(obj, prop) => {
            let qualified = match obj.as_ref() {
                akar_parser::ast::Expression::Variable(var) => format!("{var}.{prop}"),
                _ => prop.clone(),
            };
            if let Some(i) = chunk.field_names.iter().position(|n| *n == qualified) {
                chunk.get_value(i, row).unwrap_or(akar_common::types::Value::Null)
            } else if let akar_parser::ast::Expression::Variable(var) = obj.as_ref()
                && let Some(i) = chunk.field_names.iter().position(|n| n == var)
            {
                // P53.26: `row.id` where `row` is a map/struct column — extract
                // the key. Runs before the bare-name match so a plain `content`
                // table column does not shadow an UNWIND map variable.
                let obj_val = chunk.get_value(i, row).unwrap_or(akar_common::types::Value::Null);
                crate::expression_evaluator::map_property_value(&obj_val, prop)
            } else if let Some(i) = chunk.field_names.iter().position(|n| *n == *prop) {
                chunk.get_value(i, row).unwrap_or(akar_common::types::Value::Null)
            } else {
                akar_common::types::Value::Null
            }
        }
        akar_parser::ast::Expression::UnaryOp(op, inner) => {
            let v = evaluate_expression_for_row(inner, chunk, row);
            match op {
                akar_parser::ast::UnaryOp::Negate => match v {
                    akar_common::types::Value::Int64(n) => akar_common::types::Value::Int64(-n),
                    akar_common::types::Value::Double(n) => akar_common::types::Value::Double(-n),
                    _ => akar_common::types::Value::Null,
                },
                _ => akar_common::types::Value::Null,
            }
        }
        akar_parser::ast::Expression::BinaryOp(op, left, right) => {
            let l = evaluate_expression_for_row(left, chunk, row);
            let r = evaluate_expression_for_row(right, chunk, row);
            binary_value_op(op, &l, &r)
        }
        akar_parser::ast::Expression::List(items) => {
            let vals = items
                .iter()
                .map(|i| evaluate_expression_for_row(i, chunk, row))
                .collect();
            akar_common::types::Value::List(vals)
        }
        akar_parser::ast::Expression::Map(items) => {
            let entries = items
                .iter()
                .map(|(k, v)| {
                    (
                        akar_common::types::Value::String(k.clone()),
                        evaluate_expression_for_row(v, chunk, row),
                    )
                })
                .collect();
            akar_common::types::Value::Map(entries)
        }
        _ => akar_common::types::Value::Null,
    }
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Int64(n) => Some(*n as f64),
        Value::Int32(n) => Some(*n as f64),
        Value::Int128(n) => Some(*n as f64),
        Value::Double(n) => Some(*n),
        Value::Float(n) => Some(*n as f64),
        _ => None,
    }
}

/// Minimal binary-operator evaluation used when no function registry is
/// available (unit-test path). The full evaluator is preferred when a registry
/// is present.
fn binary_value_op(op: &BinaryOp, l: &Value, r: &Value) -> Value {
    match op {
        BinaryOp::Add => match (l, r) {
            (Value::String(a), Value::String(b)) => Value::String(format!("{a}{b}")),
            _ => match (as_f64(l), as_f64(r)) {
                (Some(a), Some(b)) => {
                    if matches!(l, Value::Int64(_)) && matches!(r, Value::Int64(_)) {
                        Value::Int64(a as i64 + b as i64)
                    } else {
                        Value::Double(a + b)
                    }
                }
                _ => Value::Null,
            },
        },
        BinaryOp::Subtract => match (as_f64(l), as_f64(r)) {
            (Some(a), Some(b)) => {
                if matches!(l, Value::Int64(_)) && matches!(r, Value::Int64(_)) {
                    Value::Int64(a as i64 - b as i64)
                } else {
                    Value::Double(a - b)
                }
            }
            _ => Value::Null,
        },
        BinaryOp::Multiply => match (as_f64(l), as_f64(r)) {
            (Some(a), Some(b)) => {
                if matches!(l, Value::Int64(_)) && matches!(r, Value::Int64(_)) {
                    Value::Int64(a as i64 * b as i64)
                } else {
                    Value::Double(a * b)
                }
            }
            _ => Value::Null,
        },
        BinaryOp::Divide => match (as_f64(l), as_f64(r)) {
            (Some(a), Some(b)) if b != 0.0 => {
                if matches!(l, Value::Int64(_)) && matches!(r, Value::Int64(_)) {
                    Value::Int64(a as i64 / b as i64)
                } else {
                    Value::Double(a / b)
                }
            }
            _ => Value::Null,
        },
        BinaryOp::Modulo => match (l, r) {
            (Value::Int64(a), Value::Int64(b)) if *b != 0 => Value::Int64(a % b),
            _ => Value::Null,
        },
        BinaryOp::Equal => Value::Bool(l == r),
        BinaryOp::NotEqual => Value::Bool(l != r),
        BinaryOp::LessThan => match (as_f64(l), as_f64(r)) {
            (Some(a), Some(b)) => Value::Bool(a < b),
            _ => Value::Null,
        },
        BinaryOp::LessThanOrEqual => match (as_f64(l), as_f64(r)) {
            (Some(a), Some(b)) => Value::Bool(a <= b),
            _ => Value::Null,
        },
        BinaryOp::GreaterThan => match (as_f64(l), as_f64(r)) {
            (Some(a), Some(b)) => Value::Bool(a > b),
            _ => Value::Null,
        },
        BinaryOp::GreaterThanOrEqual => match (as_f64(l), as_f64(r)) {
            (Some(a), Some(b)) => Value::Bool(a >= b),
            _ => Value::Null,
        },
        BinaryOp::And => match (l, r) {
            (Value::Bool(a), Value::Bool(b)) => Value::Bool(*a && *b),
            _ => Value::Null,
        },
        BinaryOp::Or => match (l, r) {
            (Value::Bool(a), Value::Bool(b)) => Value::Bool(*a || *b),
            _ => Value::Null,
        },
        _ => Value::Null,
    }
}

/// Evaluate a constant-only expression (literal or function call over
/// literals) into a `Value`, using the function registry. Used by the
/// CREATE DML write path to support expressions like `DATE('2024-01-15')`.
/// Returns `Value::Null` for expressions that reference variables or
/// otherwise cannot be folded without a row context.
pub fn evaluate_constant_expr(expr: &Expression, registry: &FunctionRegistry) -> Value {
    match expr {
        Expression::Constant(c) => ast_constant_to_value(c),
        Expression::List(items) => Value::List(items.iter().map(|i| evaluate_constant_expr(i, registry)).collect()),
        Expression::Map(items) => Value::Map(
            items
                .iter()
                .map(|(k, v)| (Value::String(k.clone()), evaluate_constant_expr(v, registry)))
                .collect(),
        ),
        Expression::FunctionCall(name, args) => {
            let arg_values: Vec<Value> = args.iter().map(|a| evaluate_constant_expr(a, registry)).collect();
            if arg_values.iter().any(|v| matches!(v, Value::Null)) {
                return Value::Null;
            }
            let func = match registry.get_scalar(name).cloned() {
                Some(f) => f,
                None => return Value::Null,
            };
            evaluate_scalar(&func, &arg_values).unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}
