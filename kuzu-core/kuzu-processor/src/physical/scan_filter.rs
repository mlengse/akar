//! Auto-extracted from physical_operator.rs
use kuzu_common::types::{LogicalTypeID, PhysicalTypeID, Value};
use kuzu_common::vector::{DataChunk, ValueVector, physical_type_size};
use kuzu_parser::ast::{BinaryOp, Constant, Expression, UnaryOp};
use kuzu_storage::table::ColumnDefinition;
use std::sync::{Arc, Mutex};
use crate::expression_evaluator::ExpressionEvaluator;
use crate::physical::types::{OperatorResult, NodeSemiMask, PhysicalOperatorExec};
use super::write_ops::PhysicalFtsScan;
use crate::physical::common::store_value_in_vector;
// ==================== Scan ====================

#[derive(Debug, Clone)]
pub struct PhysicalScan {
    pub table_name: String,
    pub table_id: u64,
    pub column_ids: Vec<u32>,
    pub estimated_cardinality: u64,
    /// Column-major table data: data[col_idx][row_idx].
    /// When present, PhysicalScan reads actual data instead of generating synthetic values.
    pub table_data: Option<Vec<Vec<Value>>>,
    /// Column definitions to map column names to physical types.
    pub table_columns: Vec<ColumnDefinition>,
    /// Optional semi-mask for SIP optimization. If present, only rows whose
    /// internal node ID offset is in the mask will be emitted.
    pub semi_mask: Option<NodeSemiMask>,
    /// Column index of the internal ID field to test against the mask.
    /// Only used when `semi_mask` is `Some`.
    pub mask_id_column: usize,
    pub fts_query: Option<PhysicalFtsScan>,
}

impl PhysicalScan {
    pub fn new(table_name: String, table_id: u64, estimated_cardinality: u64) -> Self {
        Self {
            table_name,
            table_id,
            column_ids: Vec::new(),
            estimated_cardinality,
            table_data: None,
            table_columns: Vec::new(),
            semi_mask: None,
            mask_id_column: 0,
            fts_query: None,
        }
    }

    /// Set column IDs to scan. These map to column indices in the table data.
    pub fn with_columns(mut self, column_ids: Vec<u32>) -> Self {
        self.column_ids = column_ids;
        self
    }

    /// Attach table data for the scan to read from.
    pub fn with_data(mut self, data: Vec<Vec<Value>>, columns: Vec<ColumnDefinition>) -> Self {
        self.table_data = Some(data);
        self.table_columns = columns;
        self
    }

    /// Attach a semi-mask for SIP optimization.
    /// When set, only rows whose internal node ID at `mask_id_column` is in the mask will be emitted.
    pub fn with_semi_mask(mut self, mask: NodeSemiMask, mask_id_column: usize) -> Self {
        self.semi_mask = Some(mask);
        self.mask_id_column = mask_id_column;
        self
    }

    pub fn with_fts_query(mut self, fts_query: PhysicalFtsScan) -> Self {
        self.fts_query = Some(fts_query);
        self
    }

    /// Convert a Value to bytes in a ValueVector at the given row index.
    fn write_value_to_vector(v: &mut ValueVector, row: usize, val: &Value) {
        match val {
            Value::Null => {
                v.set_null(row, true);
            }
            Value::Bool(x) => {
                if v.physical_type() == PhysicalTypeID::Bool {
                    v.data_mut()[row] = if *x { 1 } else { 0 };
                    v.set_null(row, false);
                }
            }
            Value::Int64(x) => {
                let offset = row * 8;
                if offset + 8 <= v.data().len() {
                    v.data_mut()[offset..offset + 8].copy_from_slice(&x.to_le_bytes());
                    v.set_null(row, false);
                }
            }
            Value::Int32(x) => {
                let offset = row * 4;
                if offset + 4 <= v.data().len() {
                    v.data_mut()[offset..offset + 4].copy_from_slice(&x.to_le_bytes());
                    v.set_null(row, false);
                }
            }
            Value::Double(x) => {
                let offset = row * 8;
                if offset + 8 <= v.data().len() {
                    v.data_mut()[offset..offset + 8].copy_from_slice(&x.to_le_bytes());
                    v.set_null(row, false);
                }
            }
            Value::Float(x) => {
                let offset = row * 4;
                if offset + 4 <= v.data().len() {
                    v.data_mut()[offset..offset + 4].copy_from_slice(&x.to_le_bytes());
                    v.set_null(row, false);
                }
            }
            Value::String(s) => {
                let offset = row * 256;
                let bytes = s.as_bytes();
                let len = bytes.len().min(255) as u8;
                if offset < v.data().len() {
                    v.data_mut()[offset] = len;
                    let copy_len = bytes.len().min(255);
                    if offset + 1 + copy_len <= v.data().len() {
                        v.data_mut()[offset + 1..offset + 1 + copy_len].copy_from_slice(&bytes[..copy_len]);
                    }
                    v.set_null(row, false);
                }
            }
            _ => {
                // For complex types, set null
                v.set_null(row, true);
            }
        }
    }

    /// Determine the PhysicalTypeID for a Value, with fallback.
    pub(crate) fn value_to_physical_type(val: &Value) -> PhysicalTypeID {
        match val {
            Value::Null => PhysicalTypeID::Int64,
            Value::Bool(_) => PhysicalTypeID::Bool,
            Value::Int64(_) | Value::UInt64(_) | Value::Int128(_) => PhysicalTypeID::Int64,
            Value::Int32(_) | Value::UInt32(_) => PhysicalTypeID::Int32,
            Value::Int16(_) | Value::UInt16(_) => PhysicalTypeID::Int16,
            Value::Int8(_) | Value::UInt8(_) => PhysicalTypeID::Int8,
            Value::Double(_) => PhysicalTypeID::Double,
            Value::Float(_) => PhysicalTypeID::Float,
            Value::String(_)
            | Value::Date(_)
            | Value::Timestamp(_)
            | Value::TimestampTz(_)
            | Value::TimestampNs(_)
            | Value::TimestampMs(_)
            | Value::TimestampSec(_)
            | Value::Interval(_) => PhysicalTypeID::String,
            Value::Blob(_) => PhysicalTypeID::Blob,
            Value::UInt128(_) => PhysicalTypeID::Int128,
            Value::Json(_) => PhysicalTypeID::String,
            Value::DTime(_) => PhysicalTypeID::Int64,
            Value::Union(_, _) => PhysicalTypeID::Struct,
            Value::InternalID(_) | Value::List(_) | Value::Map(_) | Value::Struct(_) => PhysicalTypeID::Int64,
        }
    }

    /// Determine PhysicalTypeID from a LogicalTypeID.
    pub(crate) fn logical_to_physical(logical: &LogicalTypeID) -> PhysicalTypeID {
        match logical {
            LogicalTypeID::Bool => PhysicalTypeID::Bool,
            LogicalTypeID::Int64 | LogicalTypeID::UInt64 | LogicalTypeID::Int128 | LogicalTypeID::Serial | LogicalTypeID::UInt128 => {
                PhysicalTypeID::Int64
            }
            LogicalTypeID::Int32 | LogicalTypeID::UInt32 => PhysicalTypeID::Int32,
            LogicalTypeID::Int16 | LogicalTypeID::UInt16 => PhysicalTypeID::Int16,
            LogicalTypeID::Int8 | LogicalTypeID::UInt8 => PhysicalTypeID::Int8,
            LogicalTypeID::Double | LogicalTypeID::Decimal => PhysicalTypeID::Double,
            LogicalTypeID::Float => PhysicalTypeID::Float,
            LogicalTypeID::String
            | LogicalTypeID::Date
            | LogicalTypeID::Timestamp
            | LogicalTypeID::TimestampTz
            | LogicalTypeID::TimestampMs
            | LogicalTypeID::TimestampNs
            | LogicalTypeID::TimestampSec
            | LogicalTypeID::Interval
            | LogicalTypeID::Time
            | LogicalTypeID::Json => PhysicalTypeID::String,
            LogicalTypeID::Blob => PhysicalTypeID::Blob,
            LogicalTypeID::Any
            | LogicalTypeID::Node
            | LogicalTypeID::Rel
            | LogicalTypeID::RecursiveRel
            | LogicalTypeID::List
            | LogicalTypeID::Array
            | LogicalTypeID::Map
            | LogicalTypeID::Struct
            | LogicalTypeID::Union
            | LogicalTypeID::Uuid
            | LogicalTypeID::InternalID => PhysicalTypeID::Int64,
        }
    }
}

impl PhysicalOperatorExec for PhysicalScan {
    fn operator_type(&self) -> &str {
        "scan"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        // If we have real table data, read from it
        if let Some(ref data) = self.table_data {
            if data.is_empty() || data[0].is_empty() {
                return Ok(vec![DataChunk::new(vec![])]);
            }

            let num_rows = data[0].len();

            // If FTS query is active, run it to get matching row offsets (doc_ids) in order of relevance
            let mut fts_doc_ids = None;
            if let Some(ref fts) = self.fts_query {
                let fts_chunks = fts.execute(vec![])?;
                let mut doc_ids = Vec::new();
                if let Some(chunk) = fts_chunks.first() {
                    if let Some(id_vec) = chunk.fields.first() {
                        for row in 0..chunk.size {
                            if let Some(doc_id) = id_vec.get_i64(row) {
                                doc_ids.push(doc_id);
                            }
                        }
                    }
                }
                fts_doc_ids = Some(doc_ids);
            }

            // Build a row inclusion mask if semi_mask is active
            let row_filter: Option<Vec<bool>> = if let Some(ref mask) = self.semi_mask {
                if self.mask_id_column < data.len() {
                    Some(
                        (0..num_rows)
                            .map(|row| {
                                if let Value::InternalID(id) = &data[self.mask_id_column][row] {
                                    mask.is_masked(id.offset)
                                } else {
                                    true
                                }
                            })
                            .collect(),
                    )
                } else {
                    None
                }
            } else {
                None
            };

            // Determine final list of row indices to emit
            let rows_to_emit: Vec<usize> = if let Some(doc_ids) = fts_doc_ids {
                doc_ids
                    .into_iter()
                    .filter_map(|doc_id| {
                        let row_idx = doc_id as usize;
                        if row_idx < num_rows {
                            if let Some(ref filter) = row_filter {
                                if filter[row_idx] { Some(row_idx) } else { None }
                            } else {
                                Some(row_idx)
                            }
                        } else {
                            None
                        }
                    })
                    .collect()
            } else if let Some(ref filter) = row_filter {
                (0..num_rows).filter(|&r| filter[r]).collect()
            } else {
                (0..num_rows).collect()
            };

            let valid_count = rows_to_emit.len();

            // Use column_ids if specified, otherwise scan all columns
            let cols_to_scan: Vec<usize> = if self.column_ids.is_empty() {
                (0..data.len()).collect()
            } else {
                self.column_ids.iter().map(|&id| id as usize).collect()
            };

            let mut fields = Vec::with_capacity(cols_to_scan.len());
            for &col_idx in &cols_to_scan {
                if col_idx >= data.len() {
                    continue;
                }
                let col_data = &data[col_idx];

                // Determine physical type from the first non-null value or column definition
                let phys_type = if let Some(col_def) = self.table_columns.get(col_idx) {
                    Self::logical_to_physical(&col_def.logical_type)
                } else {
                    col_data
                        .iter()
                        .find_map(|v| {
                            if !matches!(v, Value::Null) {
                                Some(Self::value_to_physical_type(v))
                            } else {
                                None
                            }
                        })
                        .unwrap_or(PhysicalTypeID::Int64)
                };

                let mut v = ValueVector::new(phys_type, valid_count);
                v.resize(valid_count);
                for (write_row, &row_idx) in rows_to_emit.iter().enumerate() {
                    if let Some(val) = col_data.get(row_idx) {
                        Self::write_value_to_vector(&mut v, write_row, val);
                    }
                }
                fields.push(v);
            }

            let names: Vec<String> = cols_to_scan
                .iter()
                .filter_map(|&ci| self.table_columns.get(ci).map(|c| c.name.clone()))
                .collect();
            let chunk = DataChunk::new(fields).with_names(names);
            return Ok(vec![chunk]);
        }

        // Fallback: no data available — return empty result
        Ok(vec![DataChunk::new(vec![])])
    }
}



// ==================== ScanRel ====================

/// Physical scan operator for relationship tables.
///
/// Reads data from a relationship table, with direction metadata.
/// Currently delegates to the same data resolution as PhysicalScan
/// but carries direction info for future optimization (e.g.,
/// adjacency-aware scanning, direction-based filtering).
#[derive(Debug, Clone)]
pub struct PhysicalScanRel {
    pub table_name: String,
    pub table_id: u64,
    pub direction: kuzu_parser::ast::EdgeDirection,
    pub table_data: Option<Vec<Vec<Value>>>,
    pub table_columns: Vec<ColumnDefinition>,
}

impl PhysicalOperatorExec for PhysicalScanRel {
    fn operator_type(&self) -> &str {
        "scan_rel"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        if let Some(ref data) = self.table_data {
            let num_cols = data.len();
            let num_rows = data.first().map(|c| c.len()).unwrap_or(0);
            if num_rows == 0 || num_cols == 0 {
                return Ok(vec![DataChunk::new(vec![])]);
            }
            let mut fields: Vec<ValueVector> = Vec::with_capacity(num_cols);
            for col in 0..num_cols {
                // Default to Int64; column definitions provide accurate types when available
                let phys_type = self
                    .table_columns
                    .get(col)
                    .map(|c| match c.logical_type {
                        kuzu_common::types::LogicalTypeID::Int64 => PhysicalTypeID::Int64,
                        kuzu_common::types::LogicalTypeID::Int32 => PhysicalTypeID::Int32,
                        kuzu_common::types::LogicalTypeID::Double => PhysicalTypeID::Double,
                        kuzu_common::types::LogicalTypeID::String => PhysicalTypeID::String,
                        kuzu_common::types::LogicalTypeID::Bool => PhysicalTypeID::Bool,
                        kuzu_common::types::LogicalTypeID::Float => PhysicalTypeID::Float,
                        _ => PhysicalTypeID::Int64,
                    })
                    .unwrap_or(PhysicalTypeID::Int64);
                let mut v = ValueVector::new(phys_type, num_rows.max(1));
                for row in 0..num_rows {
                    if let Some(val) = data[col].get(row) {
                        let _ = v.set_value(row, val);
                    }
                }
                v.resize(num_rows);
                fields.push(v);
            }
            let names: Vec<String> = (0..num_cols)
                .filter_map(|ci| self.table_columns.get(ci).map(|c| c.name.clone()))
                .collect();
            Ok(vec![DataChunk {
                fields,
                size: num_rows,
                field_names: names,
            }])
        } else {
            Ok(vec![DataChunk::new(vec![])])
        }
    }
}


// ==================== Filter ====================

pub struct PhysicalFilter {
    pub expression: Expression,
    pub evaluator: Option<Arc<Mutex<ExpressionEvaluator>>>,
}

impl PhysicalFilter {
    pub fn new(expression: Expression) -> Self {
        Self {
            expression,
            evaluator: None,
        }
    }

    pub fn with_evaluator(expression: Expression, evaluator: Arc<Mutex<ExpressionEvaluator>>) -> Self {
        Self {
            expression,
            evaluator: Some(evaluator),
        }
    }

    /// Evaluate a filter expression against values and return a boolean mask.
    /// Uses the new ExpressionEvaluator when available, falls back to legacy logic.
    pub fn evaluate_expression(
        expr: &Expression,
        chunk: &DataChunk,
        evaluator: Option<&ExpressionEvaluator>,
    ) -> Result<Vec<bool>, String> {
        // If we have a proper ExpressionEvaluator, use it
        if let Some(eval) = evaluator {
            let result_vec = eval.evaluate(expr, chunk)?;
            let size = result_vec.size();
            let mut mask = Vec::with_capacity(size);
            for i in 0..size {
                let val = result_vec.get_value(i);
                match val {
                    Some(Value::Bool(b)) => mask.push(b),
                    Some(Value::Null) => mask.push(false),
                    Some(_) => mask.push(true), // non-null, non-bool = truthy
                    None => mask.push(false),   // null = false
                }
            }
            return Ok(mask);
        }

        // Legacy fallback
        Self::evaluate_expression_legacy(expr, chunk)
    }

    /// Legacy evaluate_expression — preserved for backward compatibility.
    fn evaluate_expression_legacy(expr: &Expression, chunk: &DataChunk) -> Result<Vec<bool>, String> {
        match expr {
            Expression::BinaryOp(op, left, right) => {
                let left_vals = Self::evaluate_expression_legacy(left, chunk)?;
                let right_vals = Self::evaluate_expression_legacy(right, chunk)?;
                evaluate_binary_op_legacy(op, &left_vals, &right_vals, chunk.size)
            }
            Expression::UnaryOp(op, inner) => {
                let vals = Self::evaluate_expression_legacy(inner, chunk)?;
                match op {
                    UnaryOp::Not => Ok(vals.iter().map(|v| !v).collect()),
                    UnaryOp::Negate => {
                        // Negation on a boolean mask inverts it
                        Ok(vals.iter().map(|v| !v).collect())
                    }
                    UnaryOp::IsNull => {
                        // IS NULL: if the inner expression returned all-false, mark as null
                        Ok(vec![false; chunk.size]) // conservative: non-null by default
                    }
                    UnaryOp::IsNotNull => {
                        Ok(vals) // pass through as-is
                    }
                }
            }
            Expression::Variable(_name) => {
                // Treat any non-null first field as true
                if let Some(field) = chunk.fields.first() {
                    Ok((0..chunk.size).map(|i| !field.is_null(i)).collect())
                } else {
                    Ok(vec![true; chunk.size])
                }
            }
            Expression::Constant(c) => {
                let val = match c {
                    Constant::Bool(true) | Constant::Integer(1) => true,
                    _ => false,
                };
                Ok(vec![val; chunk.size])
            }
            Expression::PropertyAccess(obj, _prop) => {
                // Simplified: evaluate the object expression, return mask
                Self::evaluate_expression_legacy(obj, chunk)
            }
            Expression::FunctionCall(_, _)
            | Expression::List(_)
            | Expression::Map(_)
            | Expression::Parameter(_)
            | Expression::ExistsSubquery(_)
            | Expression::Case(_)
            | Expression::Star
            | Expression::ListPredicate { .. }
            | Expression::Lambda { .. } => Ok(vec![true; chunk.size]),
        }
    }
}

impl PhysicalOperatorExec for PhysicalFilter {
    fn operator_type(&self) -> &str {
        "filter"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let evaluator = self.evaluator.as_ref().and_then(|e| e.lock().ok());

        let mut output = Vec::new();
        for chunk in input {
            let mask = Self::evaluate_expression(&self.expression, &chunk, evaluator.as_deref())?;
            // Filter rows based on mask
            let selected: Vec<usize> = mask.iter().enumerate().filter(|&(_, v)| *v).map(|(i, _)| i).collect();

            if selected.is_empty() {
                continue;
            }

            let mut new_fields = Vec::new();
            for field in &chunk.fields {
                let mut new_v = ValueVector::new(field.physical_type(), selected.len());
                new_v.resize(selected.len()); // Pre-allocate so data_mut() is writable
                for (new_idx, &old_idx) in selected.iter().enumerate() {
                    let type_size = physical_type_size(field.physical_type());
                    let src_offset = old_idx * type_size;
                    let dst_offset = new_idx * type_size;
                    let src_data = field.data();
                    if src_offset + type_size <= src_data.len() && dst_offset + type_size <= new_v.data().len() {
                        new_v.data_mut()[dst_offset..dst_offset + type_size]
                            .copy_from_slice(&src_data[src_offset..src_offset + type_size]);
                    }
                    new_v.set_null(new_idx, field.is_null(old_idx));
                }
                new_fields.push(new_v);
            }
            output.push(DataChunk::new(new_fields).with_names(chunk.field_names.clone()));
        }
        Ok(output)
    }
}

fn evaluate_binary_op_legacy(op: &BinaryOp, left: &[bool], right: &[bool], size: usize) -> Result<Vec<bool>, String> {
    let len = left.len().min(right.len()).min(size);
    let result: Vec<bool> = (0..len)
        .map(|i| match op {
            BinaryOp::And => left[i] && right[i],
            BinaryOp::Or => left[i] || right[i],
            BinaryOp::Xor => left[i] ^ right[i],
            BinaryOp::Equal => left[i] == right[i],
            BinaryOp::NotEqual => left[i] != right[i],
            _ => true, // default pass-through for other comparisons
        })
        .collect();
    Ok(result)
}


// ==================== Projection ====================

pub struct PhysicalProjection {
    /// Column indices to include (in order).
    pub column_indices: Vec<usize>,
}

impl PhysicalOperatorExec for PhysicalProjection {
    fn operator_type(&self) -> &str {
        "projection"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let output: Vec<DataChunk> = input
            .into_iter()
            .map(|chunk| {
                let fields: Vec<ValueVector> = self
                    .column_indices
                    .iter()
                    .filter_map(|&i| chunk.fields.get(i).cloned())
                    .collect();
                let size = if fields.is_empty() {
                    chunk.size
                } else {
                    fields.first().map(|f| f.size()).unwrap_or(0)
                };
                let names = self
                    .column_indices
                    .iter()
                    .filter_map(|&i| chunk.field_names.get(i).cloned())
                    .collect();
                DataChunk {
                    fields,
                    size,
                    field_names: names,
                }
            })
            .collect();

        if output.is_empty() {
            Ok(vec![DataChunk {
                fields: vec![],
                size: 0,
                field_names: vec![],
            }])
        } else {
            Ok(output)
        }
    }
}


// ==================== Limit ====================

#[derive(Debug, Clone)]
pub struct PhysicalLimit {
    pub limit: u64,
    pub offset: u64,
}

impl PhysicalOperatorExec for PhysicalLimit {
    fn operator_type(&self) -> &str {
        "limit"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let mut remaining = self.limit;
        let skip = self.offset;
        let mut output = Vec::new();
        let mut skipped: u64 = 0;

        for chunk in input {
            if remaining == 0 {
                break;
            }
            let chunk_size = chunk.size as u64;

            // Apply offset: skip entire chunks before the offset
            if skipped + chunk_size <= skip {
                skipped += chunk_size;
                continue;
            }

            // Calculate start position within this chunk
            let start_in_chunk = if skipped < skip { (skip - skipped) as usize } else { 0 };

            // Mark this chunk as processed
            skipped += chunk_size;

            // Calculate how many rows to take from this chunk
            let available = (chunk_size as usize).saturating_sub(start_in_chunk);
            let take = available.min(remaining as usize);

            if take == 0 {
                continue;
            }

            remaining -= take as u64;

            if start_in_chunk == 0 && take == chunk.size {
                // Full chunk, no truncation needed
                output.push(chunk);
            } else {
                // Partial chunk: copy row-by-row using get_value/store_value_in_vector
                // This correctly handles all Value types (including variable-length ones)
                let mut new_fields = Vec::with_capacity(chunk.fields.len());
                for field in &chunk.fields {
                    let phys_type = field.physical_type();
                    let mut new_v = ValueVector::new(phys_type, take);
                    new_v.resize(take);
                    for i in 0..take {
                        let src_row = start_in_chunk + i;
                        if field.is_null(src_row) {
                            new_v.set_null(i, true);
                        } else if let Some(val) = field.get_value(src_row) {
                            store_value_in_vector(&mut new_v, i, &val);
                        }
                    }
                    new_fields.push(new_v);
                }
                output.push(DataChunk::new(new_fields).with_names(chunk.field_names.clone()));
            }
        }
        Ok(output)
    }
}

// ==================== PrimaryKeyScan ====================

/// Physical operator for scanning a table by primary key lookup from an input key column.
///
/// Reads keys from `key_column_idx` in the input `DataChunk`s, performs
/// point lookups using the ART index on a node table, and produces an
/// output chunk containing the retrieved rows.
pub struct PhysicalPrimaryKeyScan {
    pub table_name: String,
    pub table_id: u64,
    pub key_column_idx: usize,
    pub table_catalog: Arc<kuzu_storage::table::TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalPrimaryKeyScan {
    fn operator_type(&self) -> &str {
        "primary_key_scan"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let node_table = self
            .table_catalog
            .get_node_table_by_name(&self.table_name)
            .ok_or_else(|| format!("Node table '{}' not found for PrimaryKeyScan", self.table_name))?;

        let num_cols = node_table.columns.len();
        let mut output_chunks = Vec::new();

        for chunk in input {
            if chunk.size == 0 {
                continue;
            }
            if self.key_column_idx >= chunk.fields.len() {
                return Err(format!("PrimaryKeyScan key column index out of bounds"));
            }

            let key_field = &chunk.fields[self.key_column_idx];
            let mut row_ids = Vec::with_capacity(chunk.size);
            
            for i in 0..chunk.size {
                if key_field.is_null(i) { continue; }
                let val = key_field.get_value(i).unwrap();
                let matched = node_table.lookup_by_pk_range(
                    Some(&val), true,
                    Some(&val), true,
                    1
                );
                if !matched.is_empty() {
                    row_ids.push(matched[0] as usize);
                }
            }
            
            if row_ids.is_empty() {
                continue;
            }
            
            let mut new_fields = Vec::with_capacity(num_cols);
            let mut field_names = Vec::with_capacity(num_cols);
            
            for col_idx in 0..num_cols {
                let phys_type = kuzu_common::types::physical_type_from_logical(node_table.columns[col_idx].logical_type);
                let mut v = ValueVector::new(phys_type, row_ids.len());
                v.resize(row_ids.len());
                
                for (out_idx, &row_id) in row_ids.iter().enumerate() {
                    let val = node_table.get_value(row_id, col_idx).cloned().unwrap_or(Value::Null);
                    if matches!(val, Value::Null) {
                        v.set_null(out_idx, true);
                    } else {
                        crate::physical::common::store_value_in_vector(&mut v, out_idx, &val);
                    }
                }
                new_fields.push(v);
                field_names.push(node_table.columns[col_idx].name.clone());
            }
            
            output_chunks.push(DataChunk {
                fields: new_fields,
                size: row_ids.len(),
                field_names,
            });
        }

        if output_chunks.is_empty() {
            Ok(vec![DataChunk {
                fields: vec![],
                size: 0,
                field_names: vec![],
            }])
        } else {
            Ok(output_chunks)
        }
    }
}

