//! Physical operator types and execution logic for query processing.
//!
//! Each physical operator implements the `Operator` trait with an `execute` method
//! that produces output `DataChunk`s from input `DataChunk`s.

use kuzu_common::types::{LogicalTypeID, PhysicalTypeID, Value};
use kuzu_common::vector::{DataChunk, ValueVector, physical_type_size};
use kuzu_function::AggregateFunction;
use kuzu_function::scalar::AggValueState;
use kuzu_parser::ast::{BinaryOp, Constant, Expression, UnaryOp};
use kuzu_storage::table::{ColumnDefinition, TableCatalog};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::expression_evaluator::ExpressionEvaluator;

/// Result of executing a physical operator.
pub type OperatorResult = Result<Vec<DataChunk>, String>;

pub type HashJoinBucket = Vec<(Value, Vec<(usize, usize)>)>;
pub type HashJoinTable = HashMap<u64, HashJoinBucket>;

// ==================== SemiMask (SIP) Types ====================

/// A semi-mask tracks which node offsets match a join condition.
/// Used for Sideways Information Passing (SIP) optimization.
/// Collects node IDs from the build side of a hash join and pushes
/// them down to the scan side to skip irrelevant nodes.
#[derive(Debug, Clone)]
pub struct NodeSemiMask {
    /// The set of matching node offsets.
    pub masked_offsets: Arc<Mutex<HashSet<u64>>>,
    /// Table ID that this mask applies to.
    pub table_id: u64,
    /// Whether the mask has been populated.
    pub initialized: Arc<std::sync::atomic::AtomicBool>,
}

impl NodeSemiMask {
    pub fn new(table_id: u64) -> Self {
        Self {
            masked_offsets: Arc::new(Mutex::new(HashSet::new())),
            table_id,
            initialized: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Add a node offset to the mask.
    pub fn mask(&self, offset: u64) {
        if let Ok(mut guard) = self.masked_offsets.lock() {
            guard.insert(offset);
        }
    }

    /// Check if a node offset is in the mask.
    pub fn is_masked(&self, offset: u64) -> bool {
        if !self.initialized.load(std::sync::atomic::Ordering::SeqCst) {
            return true; // No mask = pass all
        }
        self.masked_offsets
            .lock()
            .map(|guard| guard.contains(&offset))
            .unwrap_or(true)
    }

    /// Finalize the mask (called after all data is collected).
    pub fn finalize(&self) {
        self.initialized.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Trait shared by all physical operators.
pub trait PhysicalOperatorExec {
    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult;
    fn operator_type(&self) -> &str;
}

// ==================== SemiMasker (SIP) ====================

/// Physical operator that collects node IDs from its child and stores them
/// in a shared `NodeSemiMask`. This mask can then be pushed down to a
/// `PhysicalScan` to skip irrelevant nodes during scanning.
///
/// This is the Rust port of C++ `SingleTableSemiMasker`, simplified to
/// handle single-table masking (the common case for hash join SIP).
pub struct PhysicalSemiMasker {
    /// Column index containing the node ID (INTERNAL_ID) to collect.
    pub key_column: usize,
    /// The shared mask to populate.
    pub mask: NodeSemiMask,
}

impl PhysicalOperatorExec for PhysicalSemiMasker {
    fn operator_type(&self) -> &str {
        "semi_masker"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        if input.is_empty() || input[0].fields.is_empty() {
            return Ok(input);
        }
        let chunk = &input[0];
        if self.key_column >= chunk.fields.len() {
            return Err(format!(
                "SemiMasker: key_column {} out of bounds ({} fields)",
                self.key_column,
                chunk.fields.len()
            ));
        }
        let field = &chunk.fields[self.key_column];
        let num_rows = chunk.size;

        for i in 0..num_rows {
            if !field.is_null(i) {
                // Extract the offset from the INTERNAL_ID value
                let offset = u64::from_le_bytes(
                    field.data()[i * 8..i * 8 + 8].try_into().unwrap_or([0u8; 8]),
                );
                self.mask.mask(offset);
            }
        }

        self.mask.finalize();

        // Pass through the input unchanged
        Ok(input)
    }
}

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
                let offset = row * 16;
                let bytes = s.as_bytes();
                let len = bytes.len().min(15) as u8;
                if offset < v.data().len() {
                    v.data_mut()[offset] = len;
                    let copy_len = bytes.len().min(15);
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
    fn value_to_physical_type(val: &Value) -> PhysicalTypeID {
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
            Value::InternalID(_) | Value::List(_) | Value::Map(_) | Value::Struct(_) => PhysicalTypeID::Int64,
        }
    }

    /// Determine PhysicalTypeID from a LogicalTypeID.
    pub(crate) fn logical_to_physical(logical: &LogicalTypeID) -> PhysicalTypeID {
        match logical {
            LogicalTypeID::Bool => PhysicalTypeID::Bool,
            LogicalTypeID::Int64 | LogicalTypeID::UInt64 | LogicalTypeID::Int128 | LogicalTypeID::Serial => {
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
            | LogicalTypeID::Interval => PhysicalTypeID::String,
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

            // Count valid rows
            let valid_count = row_filter
                .as_ref()
                .map(|f| f.iter().filter(|&&b| b).count())
                .unwrap_or(num_rows);

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

                if let Some(ref row_filter) = row_filter {
                    // Write only valid rows
                    let mut v = ValueVector::new(phys_type, valid_count);
                    v.resize(valid_count);
                    let mut write_row = 0;
                    for (row, val) in col_data.iter().enumerate() {
                        if row < num_rows && row_filter[row] {
                            Self::write_value_to_vector(&mut v, write_row, val);
                            write_row += 1;
                        }
                    }
                    fields.push(v);
                } else {
                    // No filtering: write all rows
                    let mut v = ValueVector::new(phys_type, num_rows);
                    v.resize(num_rows);
                    for (row, val) in col_data.iter().enumerate() {
                        Self::write_value_to_vector(&mut v, row, val);
                    }
                    fields.push(v);
                }
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
                let phys_type = self.table_columns.get(col).map(|c| {
                    match c.logical_type {
                        kuzu_common::types::LogicalTypeID::Int64 => PhysicalTypeID::Int64,
                        kuzu_common::types::LogicalTypeID::Int32 => PhysicalTypeID::Int32,
                        kuzu_common::types::LogicalTypeID::Double => PhysicalTypeID::Double,
                        kuzu_common::types::LogicalTypeID::String => PhysicalTypeID::String,
                        kuzu_common::types::LogicalTypeID::Bool => PhysicalTypeID::Bool,
                        kuzu_common::types::LogicalTypeID::Float => PhysicalTypeID::Float,
                        _ => PhysicalTypeID::Int64,
                    }
                }).unwrap_or(PhysicalTypeID::Int64);
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
            Ok(vec![DataChunk { fields, size: num_rows, field_names: names }])
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
            Expression::FunctionCall(_, _) | Expression::List(_) | Expression::Map(_) | Expression::Parameter(_)
            | Expression::ExistsSubquery(_) | Expression::Case(_) | Expression::Star
            | Expression::ListPredicate { .. } => {
                Ok(vec![true; chunk.size])
            }
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
                DataChunk { fields, size, field_names: names }
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

// ==================== OrderBy ====================

pub struct PhysicalOrderBy {
    pub sort_keys: Vec<(u32, bool)>,
}

impl PhysicalOperatorExec for PhysicalOrderBy {
    fn operator_type(&self) -> &str {
        "order_by"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        if input.is_empty() {
            return Ok(Vec::new());
        }

        let total_rows: usize = input.iter().map(|c| c.size).sum();
        if total_rows == 0 {
            return Ok(input);
        }

        // Collect all values per column as Value (supports all types)
        let num_fields = input[0].num_fields();
        let mut all_values: Vec<Vec<(Value, bool)>> = (0..num_fields).map(|_| Vec::with_capacity(total_rows)).collect();

        for chunk in &input {
            for row in 0..chunk.size {
                for col in 0..num_fields {
                    if let Some(field) = chunk.fields.get(col) {
                        let val = field.get_value(row).unwrap_or(Value::Null);
                        let is_null = field.is_null(row);
                        all_values[col].push((val, is_null));
                    }
                }
            }
        }

        // Sort indices by composite key
        let mut indices: Vec<usize> = (0..total_rows).collect();
        if !self.sort_keys.is_empty() {
            indices.sort_by(|a, b| {
                for &(col, ascending) in &self.sort_keys {
                    let col = col as usize;
                    if col >= num_fields {
                        continue;
                    }
                    let va = &all_values[col][*a].0;
                    let vb = &all_values[col][*b].0;
                    let cmp = value_cmp(va, vb);
                    if cmp != std::cmp::Ordering::Equal {
                        return if ascending { cmp } else { cmp.reverse() };
                    }
                }
                std::cmp::Ordering::Equal
            });
        }

        // Build sorted output chunks (up to 100 rows per chunk)
        let chunk_size = 100usize;
        let mut output = Vec::new();
        for chunk_start in (0..total_rows).step_by(chunk_size) {
            let chunk_end = (chunk_start + chunk_size).min(total_rows);
            let size = chunk_end - chunk_start;
            let mut fields = Vec::new();
            for col in 0..num_fields {
                let first_val = &all_values[col][indices[chunk_start]].0;
                let phys_type = first_val.physical_type();
                let mut v = ValueVector::new(phys_type, size);
                v.resize(size);
                for (out_idx, &src_idx) in indices[chunk_start..chunk_end].iter().enumerate() {
                    let (ref val, is_null) = all_values[col][src_idx];
                    if is_null || matches!(val, Value::Null) {
                        v.set_null(out_idx, true);
                    } else {
                        store_value_in_vector(&mut v, out_idx, val);
                    }
                }
                fields.push(v);
            }
            output.push(DataChunk::new(fields));
        }
        Ok(output)
    }
}

/// Compare two Values for sorting. NULLs sort last.
fn value_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => std::cmp::Ordering::Equal,
        (Value::Null, _) => std::cmp::Ordering::Greater,
        (_, Value::Null) => std::cmp::Ordering::Less,
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Int64(x), Value::Int64(y)) => x.cmp(y),
        (Value::Int32(x), Value::Int32(y)) => x.cmp(y),
        (Value::Int16(x), Value::Int16(y)) => (*x as i64).cmp(&(*y as i64)),
        (Value::Int8(x), Value::Int8(y)) => (*x as i64).cmp(&(*y as i64)),
        (Value::UInt64(x), Value::UInt64(y)) => x.cmp(y),
        (Value::UInt32(x), Value::UInt32(y)) => (*x as u64).cmp(&(*y as u64)),
        (Value::UInt16(x), Value::UInt16(y)) => (*x as u64).cmp(&(*y as u64)),
        (Value::UInt8(x), Value::UInt8(y)) => (*x as u64).cmp(&(*y as u64)),
        (Value::Double(x), Value::Double(y)) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Date(x), Value::Date(y)) => x.0.cmp(&y.0),
        (Value::Timestamp(x), Value::Timestamp(y)) => x.0.cmp(&y.0),
        _ => std::cmp::Ordering::Equal,
    }
}

// ==================== Aggregate ====================

/// Helper: parse an aggregate function name string into an AggregateFunction enum.
fn parse_aggregate_function(name: &str) -> AggregateFunction {
    match name.to_uppercase().as_str() {
        "COUNT" => AggregateFunction::Count,
        "COUNT(*)" => AggregateFunction::CountStar,
        "SUM" => AggregateFunction::Sum,
        "AVG" => AggregateFunction::Avg,
        "MIN" => AggregateFunction::Min,
        "MAX" => AggregateFunction::Max,
        "COLLECT" => AggregateFunction::Collect,
        "STDDEV" => AggregateFunction::StdDev,
        "VARIANCE" => AggregateFunction::Variance,
        _ => AggregateFunction::Count,
    }
}

pub struct PhysicalAggregate {
    pub group_by_cols: Vec<u32>,
    pub aggregate_functions: Vec<String>,
}

impl PhysicalOperatorExec for PhysicalAggregate {
    fn operator_type(&self) -> &str {
        "aggregate"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        if input.is_empty() {
            let mut fields = Vec::new();
            for _ in 0..self.aggregate_functions.len() {
                let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
                v.set_i64(0, 0);
                v.resize(1);
                fields.push(v);
            }
            return Ok(vec![DataChunk::new(fields)]);
        }

        if self.group_by_cols.is_empty() {
            self.compute_scalar_aggregates(&input)
        } else {
            self.compute_grouped_aggregates(&input)
        }
    }
}

impl PhysicalAggregate {
    /// Compute scalar aggregates (no GROUP BY) across all input chunks.
    fn compute_scalar_aggregates(&self, input: &[DataChunk]) -> OperatorResult {
        let funcs: Vec<AggregateFunction> = self
            .aggregate_functions
            .iter()
            .map(|name| parse_aggregate_function(name))
            .collect();

        let mut states: Vec<AggValueState> = funcs.iter().map(AggValueState::new).collect();

        for chunk in input {
            for row in 0..chunk.size {
                for (i, state) in states.iter_mut().enumerate() {
                    let col_idx = i.min(chunk.fields.len().saturating_sub(1));
                    let val = chunk
                        .fields
                        .get(col_idx)
                        .and_then(|f| f.get_value(row))
                        .unwrap_or(Value::Null);

                    // COUNT(*) counts rows regardless
                    if matches!(funcs[i], AggregateFunction::CountStar) {
                        if let AggValueState::Count(n) = state {
                            *n += 1;
                        }
                        continue;
                    }

                    state.update(&val);
                }
            }
        }

        // Build output: determine result types from final values
        let mut fields = Vec::new();
        for state in &states {
            let result = state.finalize();
            let physical_type = result.physical_type();
            let mut v = ValueVector::new(physical_type, 1);
            v.resize(1); // Must set size first so data() is writable
            // Store the result value into the vector
            store_value_in_vector(&mut v, 0, &result);
            fields.push(v);
        }
        Ok(vec![DataChunk::new(fields)])
    }

    /// Compute hash-based GROUP BY aggregates.
    fn compute_grouped_aggregates(&self, input: &[DataChunk]) -> OperatorResult {
        let funcs: Vec<AggregateFunction> = self
            .aggregate_functions
            .iter()
            .map(|name| parse_aggregate_function(name))
            .collect();

        // Hash map: hash → Vec of (actual_value, agg_states)
        // Using separate hash + value to avoid requiring Hash+Eq on Value.
        // Collisions are resolved by comparing actual values (PartialEq).
        type GroupBucket = Vec<(Value, Vec<AggValueState>)>;
        let mut groups: HashMap<u64, GroupBucket> = HashMap::new();
        let num_group_cols = self.group_by_cols.len();

        for chunk in input {
            for row in 0..chunk.size {
                // Build composite key
                let key = if num_group_cols == 1 {
                    let col = self.group_by_cols[0] as usize;
                    chunk
                        .fields
                        .get(col)
                        .and_then(|f| f.get_value(row))
                        .unwrap_or(Value::Null)
                } else {
                    let mut key_vals = Vec::with_capacity(num_group_cols);
                    for &gc in &self.group_by_cols {
                        let val = chunk
                            .fields
                            .get(gc as usize)
                            .and_then(|f| f.get_value(row))
                            .unwrap_or(Value::Null);
                        key_vals.push(val);
                    }
                    Value::List(key_vals)
                };

                let hash = value_hash(&key);
                let bucket = groups.entry(hash).or_default();

                // Find existing entry or create new one
                let entry_idx = bucket.iter().position(|(k, _)| *k == key);
                let states = if let Some(idx) = entry_idx {
                    &mut bucket[idx].1
                } else {
                    bucket.push((key, funcs.iter().map(AggValueState::new).collect()));
                    &mut bucket.last_mut().unwrap().1
                };

                for (i, state) in states.iter_mut().enumerate() {
                    let val = chunk
                        .fields
                        .get(i.min(chunk.fields.len().saturating_sub(1)))
                        .and_then(|f| f.get_value(row))
                        .unwrap_or(Value::Null);

                    if matches!(funcs[i], AggregateFunction::CountStar) {
                        if let AggValueState::Count(n) = state {
                            *n += 1;
                        }
                        continue;
                    }

                    state.update(&val);
                }
            }
        }

        if groups.is_empty() {
            let num_cols = num_group_cols + self.aggregate_functions.len();
            return Ok(vec![DataChunk::new(Vec::with_capacity(num_cols))]);
        }

        // Collect all groups (flatten buckets)
        let mut group_keys: Vec<Value> = Vec::new();
        let mut agg_results: Vec<Vec<Value>> = (0..self.aggregate_functions.len()).map(|_| Vec::new()).collect();

        for bucket in groups.values() {
            for (key, states) in bucket {
                group_keys.push(key.clone());
                for (i, state) in states.iter().enumerate() {
                    agg_results[i].push(state.finalize());
                }
            }
        }

        let num_rows = group_keys.len();
        let num_agg = self.aggregate_functions.len();

        // Build output vectors
        let mut output_fields = Vec::with_capacity(num_group_cols + num_agg);

        // Group key columns — one per group_by_col
        if num_group_cols == 1 {
            let first_val = &group_keys[0];
            let phys_type = first_val.physical_type();
            let mut v = ValueVector::new(phys_type, num_rows);
            v.resize(num_rows);
            for (row, key) in group_keys.iter().enumerate() {
                if matches!(key, Value::Null) {
                    v.set_null(row, true);
                } else {
                    store_value_in_vector(&mut v, row, key);
                }
            }
            output_fields.push(v);
        } else {
            // Multi-key: expand Value::List back into individual columns
            for gc_idx in 0..num_group_cols {
                let first_key = &group_keys[0];
                let inner_val = match first_key {
                    Value::List(vals) => vals.get(gc_idx).cloned().unwrap_or(Value::Null),
                    _ => Value::Null,
                };
                let phys_type = inner_val.physical_type();
                let mut v = ValueVector::new(phys_type, num_rows);
                v.resize(num_rows);
                for (row, key) in group_keys.iter().enumerate() {
                    let val = match key {
                        Value::List(vals) => vals.get(gc_idx).cloned().unwrap_or(Value::Null),
                        _ => Value::Null,
                    };
                    if matches!(val, Value::Null) {
                        v.set_null(row, true);
                    } else {
                        store_value_in_vector(&mut v, row, &val);
                    }
                }
                output_fields.push(v);
            }
        }

        // Aggregate result columns
        for i in 0..num_agg {
            let first_val = &agg_results[i][0];
            let physical_type = first_val.physical_type();
            let mut v = ValueVector::new(physical_type, num_rows);
            v.resize(num_rows);
            for (row, val) in agg_results[i].iter().enumerate() {
                store_value_in_vector(&mut v, row, val);
            }
            output_fields.push(v);
        }

        Ok(vec![DataChunk::new(output_fields)])
    }
}

/// Store a Value into a ValueVector at the given row index.
fn store_value_in_vector(v: &mut ValueVector, row: usize, val: &Value) {
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
            let offset = row * 16;
            let bytes = s.as_bytes();
            let len = bytes.len().min(15) as u8;
            v.data_mut()[offset] = len;
            let copy_len = bytes.len().min(15);
            v.data_mut()[offset + 1..offset + 1 + copy_len].copy_from_slice(&bytes[..copy_len]);
            v.set_null(row, false);
        }
        _ => {
            // For complex types (List, Struct, etc.), store as Int64 0 placeholder
            v.set_null(row, true);
        }
    }
}

// ==================== CrossProduct ====================

/// Physical cross product (Cartesian product) operator.
///
/// Combines every row from the left side with every row from the right side.
/// The left side is the first half of input chunks, the right side is the
/// second half.
pub struct PhysicalCrossProduct;

impl PhysicalCrossProduct {
    pub fn execute_binary(&self, left_chunks: &[DataChunk], right_chunks: &[DataChunk]) -> OperatorResult {
        if left_chunks.is_empty() || right_chunks.is_empty() {
            return Ok(vec![]);
        }

        // Count total rows on each side
        let left_rows: usize = left_chunks.iter().map(|c| c.size).sum();
        let right_rows: usize = right_chunks.iter().map(|c| c.size).sum();

        if left_rows == 0 || right_rows == 0 {
            return Ok(vec![]);
        }

        // Collect left and right values into column-major Vec<Vec<Value>>
        let num_left_cols = left_chunks.first().map(|c| c.num_fields()).unwrap_or(0);
        let num_right_cols = right_chunks.first().map(|c| c.num_fields()).unwrap_or(0);
        let total_cols = num_left_cols + num_right_cols;
        let total_rows = left_rows * right_rows;

        let mut left_values: Vec<Vec<Value>> = (0..num_left_cols).map(|_| Vec::with_capacity(left_rows)).collect();
        for chunk in left_chunks {
            for col in 0..num_left_cols {
                if let Some(field) = chunk.fields.get(col) {
                    for row in 0..chunk.size {
                        left_values[col].push(field.get_value(row).unwrap_or(Value::Null));
                    }
                }
            }
        }

        let mut right_values: Vec<Vec<Value>> = (0..num_right_cols).map(|_| Vec::with_capacity(right_rows)).collect();
        for chunk in right_chunks {
            for col in 0..num_right_cols {
                if let Some(field) = chunk.fields.get(col) {
                    for row in 0..chunk.size {
                        right_values[col].push(field.get_value(row).unwrap_or(Value::Null));
                    }
                }
            }
        }

        // Build physical types and names for output
        let mut output_types: Vec<PhysicalTypeID> = Vec::with_capacity(total_cols);
        let mut field_names = Vec::with_capacity(total_cols);
        for col in 0..num_left_cols {
            if let Some(field) = left_chunks[0].fields.get(col) {
                output_types.push(field.physical_type());
            }
        }
        for col in 0..num_right_cols {
            if let Some(field) = right_chunks[0].fields.get(col) {
                output_types.push(field.physical_type());
            }
        }
        
        if let Some(c) = left_chunks.first() {
            field_names.extend(c.field_names.iter().cloned());
        }
        if let Some(c) = right_chunks.first() {
            field_names.extend(c.field_names.iter().cloned());
        }

        // Build output vectors
        let mut output_fields: Vec<ValueVector> = output_types
            .iter()
            .map(|t| ValueVector::new(*t, total_rows.max(1)))
            .collect();

        let mut out_row = 0usize;
        for lr in 0..left_rows {
            for rr in 0..right_rows {
                for (col, field) in output_fields.iter_mut().enumerate().take(num_left_cols) {
                    let val = &left_values[col][lr];
                    let _ = field.set_value(out_row, val);
                }
                for col in 0..num_right_cols {
                    let val = &right_values[col][rr];
                    let _ = output_fields[num_left_cols + col].set_value(out_row, val);
                }
                out_row += 1;
            }
        }

        for field in &mut output_fields {
            field.resize(total_rows);
        }

        // Propagate field names from left ++ right sides
        let mut output_names: Vec<String> = left_chunks.first()
            .map(|c| c.field_names.clone())
            .unwrap_or_default();
        output_names.extend(right_chunks.first()
            .map(|c| c.field_names.clone())
            .unwrap_or_default());
        Ok(vec![DataChunk {
            fields: output_fields,
            size: total_rows,
            field_names: output_names,
        }])
    }
}

// ==================== SemiJoin ====================

/// Physical semi-join: Returns left rows that have a matching join key in the right side.
/// Only left-side columns are emitted (no right columns in output).
pub struct PhysicalSemiJoin {
    pub build_columns: Vec<u32>,
    pub probe_columns: Vec<u32>,
}

impl PhysicalSemiJoin {
    pub fn execute_binary(&self, build_chunks: &[DataChunk], probe_chunks: &[DataChunk]) -> OperatorResult {
        if build_chunks.is_empty() || probe_chunks.is_empty() {
            return Ok(vec![]);
        }

        let build_col = self.build_columns.first().copied().unwrap_or(0) as usize;
        let probe_col = self.probe_columns.first().copied().unwrap_or(0) as usize;

        // Build hash set of right-side keys
        let mut hash_set: std::collections::HashSet<u64> = std::collections::HashSet::new();
        for chunk in build_chunks {
            for row in 0..chunk.size {
                if let Some(field) = chunk.fields.get(build_col) {
                    let key = field.get_value(row).unwrap_or(Value::Null);
                    if matches!(key, Value::Null) { continue; }
                    hash_set.insert(value_hash(&key));
                }
            }
        }

        // Probe: emit left rows whose key is in hash_set
        let mut probe_types: Vec<PhysicalTypeID> = Vec::new();
        if let Some(first) = probe_chunks.first() {
            for col in 0..first.num_fields() {
                probe_types.push(first.field(col).physical_type());
            }
        }

        // Count matching rows first
        let mut match_rows: Vec<(usize, usize)> = Vec::new();
        for (ci, chunk) in probe_chunks.iter().enumerate() {
            for row in 0..chunk.size {
                if let Some(field) = chunk.fields.get(probe_col) {
                    let key = field.get_value(row).unwrap_or(Value::Null);
                    if matches!(key, Value::Null) { continue; }
                    if hash_set.contains(&value_hash(&key)) {
                        match_rows.push((ci, row));
                    }
                }
            }
        }

        if match_rows.is_empty() {
            return Ok(vec![]);
        }

        // Build output with only left-side columns
        let num_left_cols = probe_types.len();
        let mut output_fields: Vec<ValueVector> = probe_types
            .iter()
            .map(|t| ValueVector::new(*t, match_rows.len().max(1)))
            .collect();

        for (out_idx, (ci, row)) in match_rows.iter().enumerate() {
            if let Some(chunk) = probe_chunks.get(*ci) {
                for (col, out_field) in output_fields.iter_mut().enumerate().take(num_left_cols) {
                    if let Some(field) = chunk.fields.get(col) {
                        let val = field.get_value(*row).unwrap_or(Value::Null);
                        let _ = out_field.set_value(out_idx, &val);
                    }
                }
            }
        }
        for field in &mut output_fields {
            field.resize(match_rows.len());
        }
        Ok(vec![DataChunk { fields: output_fields, size: match_rows.len(), field_names: vec![] }])
    }
}

// ==================== AntiJoin ====================

/// Physical anti-join: Returns left rows that have NO matching join key in the right side.
/// Only left-side columns are emitted.
pub struct PhysicalAntiJoin {
    pub build_columns: Vec<u32>,
    pub probe_columns: Vec<u32>,
}

impl PhysicalAntiJoin {
    pub fn execute_binary(&self, build_chunks: &[DataChunk], probe_chunks: &[DataChunk]) -> OperatorResult {
        if probe_chunks.is_empty() {
            return Ok(vec![]);
        }
        if build_chunks.is_empty() {
            // If build is empty, AntiJoin returns all of probe
            return Ok(probe_chunks.to_vec());
        }

        let build_col = self.build_columns.first().copied().unwrap_or(0) as usize;
        let probe_col = self.probe_columns.first().copied().unwrap_or(0) as usize;

        // Build hash set of right-side keys
        let mut hash_set: std::collections::HashSet<u64> = std::collections::HashSet::new();
        for chunk in build_chunks {
            for row in 0..chunk.size {
                if let Some(field) = chunk.fields.get(build_col) {
                    let key = field.get_value(row).unwrap_or(Value::Null);
                    if matches!(key, Value::Null) { continue; }
                    hash_set.insert(value_hash(&key));
                }
            }
        }

        let mut probe_types: Vec<PhysicalTypeID> = Vec::new();
        if let Some(first) = probe_chunks.first() {
            for col in 0..first.num_fields() {
                probe_types.push(first.field(col).physical_type());
            }
        }

        // Probe: emit left rows whose key is NOT in hash_set
        let mut non_match_rows: Vec<(usize, usize)> = Vec::new();
        for (ci, chunk) in probe_chunks.iter().enumerate() {
            for row in 0..chunk.size {
                if let Some(field) = chunk.fields.get(probe_col) {
                    let key = field.get_value(row).unwrap_or(Value::Null);
                    if matches!(key, Value::Null) { continue; }
                    if !hash_set.contains(&value_hash(&key)) {
                        non_match_rows.push((ci, row));
                    }
                }
            }
        }

        if non_match_rows.is_empty() {
            return Ok(vec![]);
        }

        let num_left_cols = probe_types.len();
        let mut output_fields: Vec<ValueVector> = probe_types
            .iter()
            .map(|t| ValueVector::new(*t, non_match_rows.len().max(1)))
            .collect();

        for (out_idx, (ci, row)) in non_match_rows.iter().enumerate() {
            if let Some(chunk) = probe_chunks.get(*ci) {
                for (col, out_field) in output_fields.iter_mut().enumerate().take(num_left_cols) {
                    if let Some(field) = chunk.fields.get(col) {
                        let val = field.get_value(*row).unwrap_or(Value::Null);
                        let _ = out_field.set_value(out_idx, &val);
                    }
                }
            }
        }
        for field in &mut output_fields {
            field.resize(non_match_rows.len());
        }
        Ok(vec![DataChunk { fields: output_fields, size: non_match_rows.len(), field_names: vec![] }])
    }
}

// ==================== Intersect ====================

/// Physical intersect operator.
///
/// For multi-pattern matching like `MATCH (a)-[:r1]->(b), (a)-[:r2]->(c)`:
/// - Multiple build sides each produce a hash table keyed by the shared variable `a`
/// - The probe side produces candidate values for `a`
/// - For each probe key, all build hash tables are probed
/// - The matching node ID lists are pairwise intersected (two-way sorted merge)
/// - Only keys that appear in ALL build sides produce output
///
/// Implementation: a simplified version of the C++ `Intersect` (intersect.h).
/// Builds hash tables from build chunks, probes with probe chunks, and does
/// pairwise intersection using sorted node ID comparison.
pub struct PhysicalIntersect {
    /// Number of build hash tables (one per pattern).
    pub num_build_sides: u32,
    /// Column index of the key in the probe side.
    pub probe_key_col: u32,
    /// Column index of the key in each build side.
    pub build_key_col: u32,
}

impl PhysicalIntersect {
    pub fn execute_binary(&self, build_chunks: &[DataChunk], probe_chunks: &[DataChunk]) -> OperatorResult {
        let num_builds = self.num_build_sides.max(1) as usize;
        if build_chunks.is_empty() || probe_chunks.is_empty() {
            return Ok(vec![]);
        }

        // For each build side, build a hash table: key_hash → (key_value, Vec<(ci, row)>)
        let build_col = self.build_key_col as usize;
        let probe_col = self.probe_key_col as usize;
        let chunk_group_size = (build_chunks.len() / num_builds).max(1);

        let mut build_tables: Vec<HashJoinTable> = Vec::new();

        for side in 0..num_builds {
            let start = side * chunk_group_size;
            let end = (start + chunk_group_size).min(build_chunks.len());
            let chunks = &build_chunks[start..end];

            let mut ht: HashJoinTable = HashMap::new();

            for (ci, chunk) in chunks.iter().enumerate() {
                for row in 0..chunk.size {
                    if let Some(field) = chunk.fields.get(build_col) {
                        let key = field.get_value(row).unwrap_or(Value::Null);
                        if matches!(key, Value::Null) {
                            continue;
                        }
                        let hash = value_hash(&key);
                        ht.entry(hash).or_default().push((key, vec![(ci, row)]));
                    }
                }
            }
            build_tables.push(ht);
        }

        if build_tables.is_empty() || build_tables.iter().any(|t| t.is_empty()) {
            // No build data — empty result
            return Ok(vec![]);
        }

        // For each probe row, probe all build tables, find intersecting keys
        let mut output_rows: Vec<Vec<Value>> = Vec::new();
        let mut probe_field_count = 0usize;

        for (ci, chunk) in probe_chunks.iter().enumerate() {
            if ci == 0 {
                probe_field_count = chunk.fields.len();
            }
            for row in 0..chunk.size {
                let probe_key = chunk.fields.get(probe_col)
                    .and_then(|f| f.get_value(row))
                    .unwrap_or(Value::Null);
                if matches!(probe_key, Value::Null) {
                    continue;
                }
                let probe_hash = value_hash(&probe_key);

                // Check if the probe key appears in ALL build tables
                let mut all_match = true;
                let mut matched_build_rows: Vec<Vec<(usize, usize)>> = Vec::new();

                for ht in &build_tables {
                    if let Some(bucket) = ht.get(&probe_hash) {
                        let mut side_matches = Vec::new();
                        for (stored_key, locations) in bucket {
                            if stored_key == &probe_key {
                                side_matches.extend(locations.iter().cloned());
                            }
                        }
                        if side_matches.is_empty() {
                            all_match = false;
                            break;
                        }
                        matched_build_rows.push(side_matches);
                    } else {
                        all_match = false;
                        break;
                    }
                }

                if !all_match || matched_build_rows.is_empty() {
                    continue;
                }

                // The probe key matches ALL build sides — emit combined payload
                // First, count total fields in output: probe fields + all build side fields
                let mut row_values: Vec<Value> = Vec::new();

                // Collect probe side values (all columns from probe chunk)
                for col_in_probe in 0..probe_field_count {
                    let val = chunk.fields.get(col_in_probe)
                        .and_then(|f| f.get_value(row))
                        .unwrap_or(Value::Null);
                    row_values.push(val);
                }

                // For each build side, emit the first matching row's payload values
                for matches in matched_build_rows.iter() {
                    if let Some(&(b_ci, b_row)) = matches.first()
                        && let Some(chunk) = build_chunks.get(b_ci) {
                            for col in 0..chunk.fields.len() {
                                let val = chunk.fields.get(col)
                                    .and_then(|f| f.get_value(b_row))
                                    .unwrap_or(Value::Null);
                                row_values.push(val);
                            }
                        }
                }
                output_rows.push(row_values);
            }
        }

        if output_rows.is_empty() {
            return Ok(vec![]);
        }

        // Build output DataChunk (one row per field group)
        // Output format: [probe_field_1, ..., probe_field_N, build_1_field_1, ..., build_N_field_M]
        let output_size = output_rows.len();
        let mut output_fields: Vec<ValueVector> = Vec::new();

        // Determine physical types from first row
        if let Some(first_row) = output_rows.first() {
            for val in first_row {
                let ptype = val.physical_type();
                let mut vv = ValueVector::new(ptype, output_size);
                vv.resize(output_size);
                output_fields.push(vv);
            }
        }

        // Fill output
        for (out_idx, row_values) in output_rows.iter().enumerate() {
            for (col, val) in row_values.iter().enumerate() {
                if let Some(field) = output_fields.get_mut(col) {
                    let _ = field.set_value(out_idx, val);
                }
            }
        }

        Ok(vec![DataChunk {
            fields: output_fields,
            size: output_size,
            field_names: vec![],
        }])
    }
}

// ==================== HashJoin ====================

pub struct PhysicalHashJoin {
    pub build_columns: Vec<u32>,
    pub probe_columns: Vec<u32>,
    /// Optional semi-mask for SIP optimization.
    /// When populated, the build-side keys are collected into this mask
    /// and can be used by downstream scan operators to filter nodes.
    pub semi_mask: Option<NodeSemiMask>,
}

impl PhysicalHashJoin {
    pub fn new(build_columns: Vec<u32>, probe_columns: Vec<u32>) -> Self {
        Self {
            build_columns,
            probe_columns,
            semi_mask: None,
        }
    }

    /// Attach a semi-mask for SIP optimization.
    pub fn with_semi_mask(mut self, mask: NodeSemiMask) -> Self {
        self.semi_mask = Some(mask);
        self
    }
}

impl PhysicalHashJoin {
    pub fn execute_binary(&self, build_chunks: &[DataChunk], probe_chunks: &[DataChunk]) -> OperatorResult {
        // Simplified Hash Join: build hash table from build chunks, probe with probe chunks
        if build_chunks.is_empty() || probe_chunks.is_empty() {
            // Inner join semantics: if either side is empty, result is empty.
            // Note: If we support outer joins later, this logic will need adjusting.
            return Ok(vec![]);
        }

        let build_col = self.build_columns.first().copied().unwrap_or(0) as usize;
        let probe_col = self.probe_columns.first().copied().unwrap_or(0) as usize;

        // Build hash table from build side (Value-keyed, hash + equality)
        // We use a two-level structure: hash → vec of (actual_value, locations)
        // The actual Value is stored alongside to disambiguate hash collisions.
        type HashBucket = Vec<(Value, Vec<(usize, usize)>)>;
        let mut hash_table: HashMap<u64, HashBucket> = HashMap::new();

        // If semi_mask is active, also collect build-side node offsets
        let mask = &self.semi_mask;

        for (ci, chunk) in build_chunks.iter().enumerate() {
            for row in 0..chunk.size {
                if let Some(field) = chunk.fields.get(build_col) {
                    // If semi_mask is active, extract node offset from INTERNAL_ID
                    if let Some(m) = mask
                        && let Some(val) = field.get_value(row) {
                            if let Value::InternalID(id) = val {
                                m.mask(id.offset);
                            } else if let Value::Int64(offset) = val {
                                m.mask(offset as u64);
                            }
                        }
                    let key = field.get_value(row).unwrap_or(Value::Null);
                    // SQL semantics: NULL keys never match in a join
                    if matches!(key, Value::Null) {
                        continue;
                    }
                    let hash = value_hash(&key);
                    hash_table.entry(hash).or_default().push((key, vec![(ci, row)]));
                }
            }
        }

        // Probe and output matching rows — collect typed Value rows
        let mut output_rows: Vec<Vec<(Value, bool)>> = Vec::new();
        let mut output_types: Vec<PhysicalTypeID> = Vec::new();
        let mut built_cols = false;
        let mut num_build_fields = 0usize;

        for chunk in probe_chunks {
            for row in 0..chunk.size {
                let probe_key = chunk
                    .fields
                    .get(probe_col)
                    .and_then(|f| f.get_value(row))
                    .unwrap_or(Value::Null);
                // SQL semantics: NULL keys never match in a join
                if matches!(probe_key, Value::Null) {
                    continue;
                }
                let probe_hash = value_hash(&probe_key);

                if let Some(bucket) = hash_table.get(&probe_hash) {
                    for (build_key, locations) in bucket {
                        // Value equality (PartialEq) disambiguates hash collisions
                        if build_key != &probe_key {
                            continue;
                        }

                        if !built_cols {
                            num_build_fields = build_chunks[0].num_fields();
                            let num_probe_fields = chunk.num_fields();
                            let total_cols = num_build_fields + num_probe_fields;

                            // Record physical types for each output column
                            for col in 0..num_build_fields {
                                if let Some(field) = build_chunks[0].fields.get(col) {
                                    output_types.push(field.physical_type());
                                }
                            }
                            for col in 0..num_probe_fields {
                                if let Some(field) = chunk.fields.get(col) {
                                    output_types.push(field.physical_type());
                                }
                            }

                            output_rows = (0..total_cols).map(|_| Vec::new()).collect();
                            built_cols = true;
                        }

                        for &(bci, brow) in locations {
                            // Build side columns
                            for (col, out_col) in output_rows.iter_mut().enumerate().take(build_chunks[bci].num_fields()) {
                                if let Some(field) = build_chunks[bci].fields.get(col) {
                                    let val = field.get_value(brow).unwrap_or(Value::Null);
                                    out_col.push((val, field.is_null(brow)));
                                }
                            }
                            // Probe side columns
                            let offset = num_build_fields;
                            for col in 0..chunk.num_fields() {
                                if let Some(field) = chunk.fields.get(col) {
                                    let val = field.get_value(row).unwrap_or(Value::Null);
                                    output_rows[offset + col].push((val, field.is_null(row)));
                                }
                            }
                        }
                    }
                }
            }
        }

        if !built_cols {
            return Ok(Vec::new());
        }

        let num_rows = output_rows[0].len();
        let mut result_fields = Vec::with_capacity(output_rows.len());
        for (col, row_data) in output_rows.iter().enumerate() {
            let phys_type = output_types.get(col).copied().unwrap_or(PhysicalTypeID::Int64);
            let mut v = ValueVector::new(phys_type, num_rows);
            v.resize(num_rows);
            for (row, (val, _is_null)) in row_data.iter().enumerate() {
                if matches!(val, Value::Null) {
                    v.set_null(row, true);
                } else {
                    store_value_in_vector(&mut v, row, val);
                }
            }
            result_fields.push(v);
        }
        // Propagate field names from both sides: build_names ++ probe_names
        let mut output_names: Vec<String> = build_chunks.first()
            .map(|c| c.field_names.clone())
            .unwrap_or_default();
        output_names.extend(probe_chunks.first()
            .map(|c| c.field_names.clone())
            .unwrap_or_default());
        let result = DataChunk::new(result_fields).with_names(output_names);
        Ok(vec![result])
    }
}

// ==================== Unwind ====================

/// Physical operator for UNWIND — expands a list expression into rows.
pub struct PhysicalUnwind {
    pub expression: kuzu_parser::ast::Expression,
    pub variable: String,
}

impl PhysicalOperatorExec for PhysicalUnwind {
    fn operator_type(&self) -> &str {
        "unwind"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        // Evaluate the expression to get a list value
        let list_val = evaluate_unwind_expr(&self.expression);
        let items = match &list_val {
            kuzu_common::types::Value::List(items) => items.clone(),
            _ => return Err("UNWIND expression must evaluate to a list".into()),
        };

        if items.is_empty() {
            return Ok(Vec::new());
        }

        // Create a new ValueVector for the unwound variable
        let first_type = items
            .first()
            .map(|v| v.physical_type())
            .unwrap_or(PhysicalTypeID::Int64);

        let mut result_chunks = Vec::new();
        // If we have input data, repeat for each input row
        if let Some(chunk) = input.first() {
            for row in 0..chunk.size {
                let mut chunk_fields = Vec::new();
                for field in chunk.fields.iter() {
                    let val = field.get_value(row).unwrap_or(Value::Null);
                    let mut v = ValueVector::new(field.physical_type(), items.len());
                    v.resize(items.len());
                    for i in 0..items.len() {
                        store_value_in_vector(&mut v, i, &val);
                    }
                    chunk_fields.push(v);
                }
                // Add unwound vector
                let mut uw_v = ValueVector::new(first_type, items.len());
                uw_v.resize(items.len());
                for (i, item) in items.iter().enumerate() {
                    store_value_in_vector(&mut uw_v, i, item);
                }
                chunk_fields.push(uw_v);
                result_chunks.push(DataChunk::new(chunk_fields));
            }
        } else {
            // No input — just the unwound vector
            let mut uw_v = ValueVector::new(first_type, items.len());
            uw_v.resize(items.len());
            for (i, item) in items.iter().enumerate() {
                store_value_in_vector(&mut uw_v, i, item);
            }
            result_chunks.push(DataChunk::new(vec![uw_v]));
        }

        Ok(result_chunks)
    }
}

/// Evaluate an UNWIND expression to get the list value.
fn evaluate_unwind_expr(expr: &kuzu_parser::ast::Expression) -> Value {
    match expr {
        kuzu_parser::ast::Expression::List(items) => {
            let values: Vec<Value> = items.iter().map(expr_to_value).collect();
            Value::List(values)
        }
        _ => Value::List(Vec::new()),
    }
}

/// Convert an AST expression to a runtime Value (for simple constants).
fn expr_to_value(expr: &kuzu_parser::ast::Expression) -> Value {
    match expr {
        kuzu_parser::ast::Expression::Constant(c) => match c {
            kuzu_parser::ast::Constant::Null => Value::Null,
            kuzu_parser::ast::Constant::Bool(b) => Value::Bool(*b),
            kuzu_parser::ast::Constant::Integer(i) => Value::Int64(*i),
            kuzu_parser::ast::Constant::Float(f) => Value::Double(*f),
            kuzu_parser::ast::Constant::String(s) => Value::String(s.clone()),
        },
        _ => Value::Null,
    }
}

// ==================== Set ====================

/// Physical operator for SET — updates a property on matched rows.
pub struct PhysicalSet {
    pub table_name: String,
    pub table_id: u64,
    pub column_name: String,
    pub column_idx: usize,
    pub value: kuzu_parser::ast::Expression,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalSet {
    fn operator_type(&self) -> &str {
        "set"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        // Collect row indices from input chunks (first column has row index)
        let mut rows_to_update: Vec<(u64, kuzu_common::types::Value)> = Vec::new();

        for chunk in &input {
            for row in 0..chunk.size {
                if let Some(field) = chunk.fields.first()
                    && let Some(val) = field.get_i64(row) {
                        // Evaluate the SET value expression against the current row
                        let set_val = evaluate_expression_for_row(&self.value, chunk, row);
                        rows_to_update.push((val as u64, set_val));
                    }
            }
        }

        if rows_to_update.is_empty() {
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
            v.resize(1);
            v.set_i64(0, 0);
            return Ok(vec![DataChunk::new(vec![v])]);
        }

        // Apply updates to the table
        let updated =
            if let Some(mut table) = self.table_catalog.get_node_table_by_name_mut(&self.table_name)
            {
                let mut count = 0u64;
                for (row_idx, val) in &rows_to_update {
                    if table.update_cell(*row_idx, self.column_idx, val.clone()).is_ok() {
                        count += 1;
                    }
                }
                count
        } else {
            return Err(format!("Table '{}' not found for SET", self.table_name));
        };

        tracing::info!("SET: updated {updated} rows in '{}'", self.table_name);

        let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
        v.resize(1);
        v.set_i64(0, updated as i64);
        Ok(vec![DataChunk::new(vec![v])])
    }
}

/// Simple expression evaluator for SET value expressions against a DataChunk row.
fn evaluate_expression_for_row(
    expr: &kuzu_parser::ast::Expression,
    chunk: &DataChunk,
    row: usize,
) -> kuzu_common::types::Value {
    match expr {
        kuzu_parser::ast::Expression::Constant(c) => match c {
            kuzu_parser::ast::Constant::Null => kuzu_common::types::Value::Null,
            kuzu_parser::ast::Constant::Bool(b) => kuzu_common::types::Value::Bool(*b),
            kuzu_parser::ast::Constant::Integer(i) => kuzu_common::types::Value::Int64(*i),
            kuzu_parser::ast::Constant::Float(f) => kuzu_common::types::Value::Double(*f),
            kuzu_parser::ast::Constant::String(s) => kuzu_common::types::Value::String(s.clone()),
        },
        _ => {
            // Fallback: try to get value from chunk fields
            if let Some(field) = chunk.fields.get(1) {
                field.get_value(row).unwrap_or(kuzu_common::types::Value::Null)
            } else {
                kuzu_common::types::Value::Null
            }
        }
    }
}

// ==================== Delete ====================

/// Physical operator for DELETE — removes rows from a node table.
pub struct PhysicalDelete {
    pub table_name: String,
    pub table_id: u64,
    pub primary_key_column: String,
    /// Row indices to delete (found by the scan/filter pipeline).
    pub row_indices: Vec<u64>,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalDelete {
    fn operator_type(&self) -> &str {
        "delete"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        // Collect row indices from input chunks
        let mut rows_to_delete: Vec<u64> = self.row_indices.clone();

        // If input has data, extract row indices from it
        for chunk in &input {
            for row in 0..chunk.size {
                if let Some(field) = chunk.fields.first()
                    && let Some(val) = field.get_i64(row) {
                        rows_to_delete.push(val as u64);
                    }
            }
        }

        if rows_to_delete.is_empty() {
            // No rows to delete — still return success
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
            v.resize(1);
            v.set_i64(0, 0);
            return Ok(vec![DataChunk::new(vec![v])]);
        }

        // Delete rows from the table
        let deleted = if let Some(mut table) = self.table_catalog.get_node_table_by_name_mut(&self.table_name) {
            let mut count = 0u64;
            for &row_idx in &rows_to_delete {
                if table.delete_row(row_idx).is_ok() {
                    count += 1;
                }
            }
            count
        } else {
            return Err(format!("Table '{}' not found for DELETE", self.table_name));
        };

        tracing::info!("DELETE: removed {deleted} rows from '{}'", self.table_name);

        let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
        v.resize(1);
        v.set_i64(0, deleted as i64);
        Ok(vec![DataChunk::new(vec![v])])
    }
}

/// Convert an AST Constant to a Value.
fn ast_constant_to_value(c: &Constant) -> Value {
    match c {
        Constant::Null => Value::Null,
        Constant::Bool(b) => Value::Bool(*b),
        Constant::Integer(i) => Value::Int64(*i),
        Constant::Float(f) => Value::Double(*f),
        Constant::String(s) => Value::String(s.clone()),
    }
}

/// Compute a hash of a Value for use in hash-based joins.
/// Hashes the discriminant (variant type) and the payload data.
fn value_hash(v: &Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::mem::discriminant(v).hash(&mut hasher);
    match v {
        Value::Null => {}
        Value::Bool(b) => b.hash(&mut hasher),
        Value::Int64(x) => x.hash(&mut hasher),
        Value::Int32(x) => x.hash(&mut hasher),
        Value::Int16(x) => x.hash(&mut hasher),
        Value::Int8(x) => x.hash(&mut hasher),
        Value::UInt64(x) => x.hash(&mut hasher),
        Value::UInt32(x) => x.hash(&mut hasher),
        Value::UInt16(x) => x.hash(&mut hasher),
        Value::UInt8(x) => x.hash(&mut hasher),
        Value::Int128(x) => x.hash(&mut hasher),
        Value::Double(x) => x.to_bits().hash(&mut hasher),
        Value::Float(x) => x.to_bits().hash(&mut hasher),
        Value::String(s) => s.hash(&mut hasher),
        Value::Blob(b) => b.hash(&mut hasher),
        Value::Date(d) => d.0.hash(&mut hasher),
        Value::Timestamp(ts) => ts.0.hash(&mut hasher),
        Value::TimestampTz(ts) => ts.0.hash(&mut hasher),
        Value::TimestampNs(ts) => ts.0.hash(&mut hasher),
        Value::TimestampMs(ts) => ts.0.hash(&mut hasher),
        Value::TimestampSec(ts) => ts.0.hash(&mut hasher),
        Value::Interval(iv) => {
            iv.months.hash(&mut hasher);
            iv.days.hash(&mut hasher);
            iv.micros.hash(&mut hasher);
        }
        Value::InternalID(id) => {
            id.table_id.hash(&mut hasher);
            id.offset.hash(&mut hasher);
        }
        Value::List(items) => {
            for item in items {
                hasher.write_u64(value_hash(item));
            }
        }
        Value::Map(pairs) => {
            for (k, v) in pairs {
                hasher.write_u64(value_hash(k));
                hasher.write_u64(value_hash(v));
            }
        }
        Value::Struct(fields) => {
            for (name, val) in fields {
                name.hash(&mut hasher);
                hasher.write_u64(value_hash(val));
            }
        }
    }
    hasher.finish()
}

// ==================== Foreach ====================

/// Physical FOREACH operator — iterates over list elements and executes sub-plans.
pub struct PhysicalForeach {
    pub variable: String,
    pub expression: Expression,
    pub sub_plans: Vec<Vec<kuzu_planner::logical_operator::LogicalOperator>>,
    pub function_registry: Option<Arc<Mutex<kuzu_function::registry::FunctionRegistry>>>,
    pub table_catalog: Option<Arc<TableCatalog>>,
}

impl PhysicalOperatorExec for PhysicalForeach {
    fn operator_type(&self) -> &str {
        "foreach"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        // Evaluate the list expression
        let list_val = match &self.expression {
            Expression::List(items) => {
                let mut vals = Vec::with_capacity(items.len());
                for item in items {
                    if let Expression::Constant(c) = item {
                        vals.push(ast_constant_to_value(c));
                    } else {
                        vals.push(Value::Null);
                    }
                }
                Value::List(vals)
            }
            _ => {
                return Err(format!(
                    "FOREACH requires a list expression, got: {:?}",
                    self.expression
                ));
            }
        };

        let list_items = match &list_val {
            Value::List(items) => items.clone(),
            _ => return Ok(vec![]),
        };

        if list_items.is_empty() || self.sub_plans.is_empty() {
            return Ok(vec![]);
        }

        // For each list item, execute sub-plans with the item value in scope.
        // We use a simplified approach: create a DataChunk with the item value
        // and pass it to each sub-plan.
        for item in &list_items {
            for sub_plan in &self.sub_plans {
                // Create a single-row DataChunk containing the current item
                let phys_type = PhysicalScan::value_to_physical_type(item);
                let mut v = ValueVector::new(phys_type, 1);
                v.resize(1);
                store_value_in_vector(&mut v, 0, item);
                let _chunk = DataChunk::new(vec![v]);

                // Execute the sub-plan using the QueryProcessor-like pipeline
                // Use the processor module directly from the same crate
                let processor = crate::processor::QueryProcessor::with_catalog(
                    self.function_registry.clone().unwrap(),
                    self.table_catalog.clone().unwrap(),
                );
                let _result = processor.execute(sub_plan)?;
            }
        }

        // FOREACH produces no output rows (it's a write-only operation)
        Ok(vec![])
    }
}

// ==================== VectorSimilarityScan ====================

/// Physical operator for vector similarity search using an HNSW index.
///
/// Searches the `VectorIndexTable` for the top-K nearest neighbours and
/// looks up the corresponding rows from the `NodeTable` to produce
/// output columns including a `distance` column.
pub struct PhysicalVectorSimilarityScan {
    pub index_name: String,
    pub index_id: u64,
    pub query_vector: Vec<f64>,
    pub top_k: u64,
    pub table_name: String,
    pub table_catalog: Option<Arc<TableCatalog>>,
}

impl PhysicalOperatorExec for PhysicalVectorSimilarityScan {
    fn operator_type(&self) -> &str {
        "vector_similarity_scan"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        let tc = self
            .table_catalog
            .clone()
            .ok_or_else(|| "No table catalog available for VectorSimilarityScan".to_string())?;

        // Resolve the vector index — by name if given, or find first index on the table
        let vi = if self.index_name.is_empty() {
            // Find the first vector index on this table
            // Scan all vector indexes to find one matching this table
            let index_name = {
                let mut found_name = String::new();
                for entry in tc.all_vector_indexes() {
                    if entry.table_name == self.table_name {
                        found_name = entry.name.clone();
                        break;
                    }
                }
                if found_name.is_empty() {
                    return Err(format!("No vector index found on table '{}'", self.table_name));
                }
                found_name
            };
            tc.get_vector_index_by_name(&index_name)
                .ok_or_else(|| format!("Vector index '{}' not found", index_name))?
        } else {
            tc.get_vector_index_by_name(&self.index_name)
                .ok_or_else(|| format!("Vector index '{}' not found", self.index_name))?
        };

        // Search the HNSW index for top-K nearest neighbours
        let results = vi.hnsw().search(&self.query_vector, self.top_k as usize);
        drop(vi); // Release the DashMap reference

        if results.is_empty() {
            return Ok(vec![DataChunk::new(vec![])]);
        }

        // Look up rows from the node table
        let node_table = tc
            .get_node_table_by_name(&self.table_name)
            .ok_or_else(|| format!("Node table '{}' not found", self.table_name))?;

        let num_cols = node_table.columns.len();
        let num_results = results.len();

        // Build output columns: all table columns + distance column
        let mut output_columns: Vec<Vec<Value>> = vec![Vec::with_capacity(num_results); num_cols + 1];

        for (dist, row_id) in &results {
            // Add distance as the last column
            output_columns[num_cols].push(Value::Double(*dist));

            // Look up each column value from the node table
            for (col_idx, out_col) in output_columns.iter_mut().enumerate().take(num_cols) {
                match node_table.get_value(*row_id, col_idx) {
                    Some(val) => out_col.push(val.clone()),
                    None => out_col.push(Value::Null),
                }
            }
        }

        drop(node_table);

        // Convert column-major Vec<Vec<Value>> to DataChunks
        use kuzu_common::types::PhysicalTypeID;
        use kuzu_common::vector::{DataChunk, ValueVector};

        let mut fields = Vec::with_capacity(num_cols + 1);

        // Add table columns
        for (_col_idx, col_data) in output_columns.iter().enumerate().take(num_cols) {
            let mut v = ValueVector::new(PhysicalTypeID::Double, num_results);
            v.resize(num_results);
            for (i, val) in col_data.iter().enumerate() {
                match val {
                    Value::Double(d) => v.set_double(i, *d),
                    Value::Int64(x) => {
                        let buf = &mut v.data_mut()[i * 8..(i + 1) * 8];
                        buf.copy_from_slice(&x.to_le_bytes());
                        v.set_null(i, false);
                    }
                    Value::String(s) => {
                        let bytes = s.as_bytes();
                        let len = bytes.len().min(15) as u8;
                        v.data_mut()[i * 16] = len;
                        let copy_len = bytes.len().min(15);
                        v.data_mut()[i * 16 + 1..i * 16 + 1 + copy_len]
                            .copy_from_slice(&bytes[..copy_len]);
                        v.set_null(i, false);
                    }
                    Value::Null => {
                        v.set_null(i, true);
                    }
                    _ => {
                        v.set_null(i, true);
                    }
                }
            }
            fields.push(v);
        }

        // Add distance column
        let dist_data = &output_columns[num_cols];
        let mut dist_v = ValueVector::new(PhysicalTypeID::Double, num_results);
        dist_v.resize(num_results);
        for (i, val) in dist_data.iter().enumerate() {
            if let Value::Double(d) = val {
                dist_v.set_double(i, *d);
            } else {
                dist_v.set_null(i, true);
            }
        }
        fields.push(dist_v);

        Ok(vec![DataChunk { fields, size: num_results, field_names: vec![] }])
    }
}

// ==================== CopyFrom ====================

/// Physical operator for COPY FROM — loads data from CSV/Parquet files into a table.
///
/// Detects file type from extension, calls the appropriate reader,
/// and inserts rows into the target table via the `TableCatalog`.
pub struct PhysicalCopyFrom {
    pub table_name: String,
    pub table_id: u64,
    pub file_path: String,
    pub columns: Vec<ColumnDefinition>,
    pub options: std::collections::HashMap<String, String>,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalOperatorExec for PhysicalCopyFrom {
    fn operator_type(&self) -> &str {
        "copy_from"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        let path = Path::new(&self.file_path);

        // 1. Detect file type from extension
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        // 2. Build config and convert column schema
        let catalog_cols: Vec<kuzu_catalog::CatalogColumn> = self
            .columns
            .iter()
            .map(|c| kuzu_catalog::CatalogColumn {
                name: c.name.clone(),
                logical_type: c.logical_type,
                is_primary_key: c.is_primary_key,
                default_value: None,
            })
            .collect();

        // 3. Read the file
        let rows = match ext.as_str() {
            "csv" | "tsv" => {
                let mut config = kuzu_storage::csv_reader::CsvReaderConfig::from_options(&self.options);
                if ext == "tsv" && !self.options.contains_key("DELIM") && !self.options.contains_key("delim") {
                    config.delimiter = b'\t';
                }

                kuzu_storage::csv_reader::read_csv(path, &catalog_cols, &config)
                    .map_err(|e| format!("CSV read error: {e}"))?
            }
            #[cfg(feature = "parquet")]
            "parquet" => kuzu_storage::parquet_reader::read_parquet(path, &catalog_cols)
                .map_err(|e| format!("Parquet read error: {e}"))?,
            #[cfg(not(feature = "parquet"))]
            "parquet" => return Err("Parquet support not enabled (feature 'parquet' in kuzu-storage)".into()),
            _ => {
                return Err(format!(
                    "Unsupported file type: .{ext} (supported: .csv, .tsv, .parquet)"
                ));
            }
        };

        // 4. Insert rows into the table
        let num_rows = rows.len();

        if let Some(mut table) = self.table_catalog.get_node_table_by_name_mut(&self.table_name) {
            for row in &rows {
                table
                    .insert_row(row.clone())
                    .map_err(|e| format!("Insert error: {e}"))?;
            }
            tracing::info!(
                "COPY FROM: inserted {num_rows} rows into node table '{}'",
                self.table_name
            );
        } else if let Some(mut table) = self.table_catalog.get_rel_table_by_name_mut(&self.table_name) {
            for row in &rows {
                if row.len() < 2 {
                    return Err("RelTable COPY FROM needs at least FROM and TO columns".into());
                }
                let from = match &row[0] {
                    Value::Int64(v) => *v as u64,
                    _ => return Err("First column of rel table must be FROM node offset (Int64)".into()),
                };
                let to = match &row[1] {
                    Value::Int64(v) => *v as u64,
                    _ => return Err("Second column of rel table must be TO node offset (Int64)".into()),
                };
                let props: Vec<Value> = row[2..].to_vec();
                table
                    .insert_rel(from, to, props)
                    .map_err(|e| format!("Insert rel error: {e}"))?;
            }
            tracing::info!(
                "COPY FROM: inserted {num_rows} rows into rel table '{}'",
                self.table_name
            );
        } else {
            return Err(format!("Table '{}' not found in storage catalog", self.table_name));
        }

        // Return success chunk with row count
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
        v.resize(1);
        v.set_i64(0, num_rows as i64);
        Ok(vec![DataChunk::new(vec![v])])
    }
}

/// Physical operator for ART index range scans.
///
/// Uses the ART index on a node table's PK column to efficiently find rows
/// within a key range, then fetches the full column data for those rows.
///
/// Pattern follows `PhysicalVectorSimilarityScan`.
#[derive(Debug, Clone)]
pub struct PhysicalArtIndexRangeScan {
    pub table_name: String,
    pub table_id: u64,
    pub lower_bound: Option<Value>,
    pub upper_bound: Option<Value>,
    pub lower_inclusive: bool,
    pub upper_inclusive: bool,
    pub table_catalog: Option<Arc<TableCatalog>>,
}

impl PhysicalOperatorExec for PhysicalArtIndexRangeScan {
    fn operator_type(&self) -> &str {
        "art_index_range_scan"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        let tc = self
            .table_catalog
            .clone()
            .ok_or_else(|| "No table catalog available for ArtIndexRangeScan".to_string())?;

        let node_table = tc
            .get_node_table_by_name(&self.table_name)
            .ok_or_else(|| format!("Node table '{}' not found", self.table_name))?;

        // Verify ART index exists
        if node_table.art_index.is_none() {
            return Err(format!(
                "Table '{}' does not have an ART index",
                self.table_name
            ));
        }

        // Execute range scan on the ART index
        let row_ids = node_table.lookup_by_pk_range(
            self.lower_bound.as_ref(),
            self.lower_inclusive,
            self.upper_bound.as_ref(),
            self.upper_inclusive,
            u64::MAX,
        );
        drop(node_table); // Release table ref before cloning data

        if row_ids.is_empty() {
            return Ok(vec![DataChunk::new(vec![])]);
        }

        // Fetch column values for matched row IDs
        let node_table = tc
            .get_node_table_by_name(&self.table_name)
            .ok_or_else(|| format!("Node table '{}' not found", self.table_name))?;

        let num_cols = node_table.columns.len();
        let num_results = row_ids.len();

        let mut output_columns: Vec<Vec<Value>> = vec![Vec::with_capacity(num_results); num_cols];

        for &row_id in &row_ids {
            for (col_idx, out_col) in output_columns.iter_mut().enumerate().take(num_cols) {
                match node_table.get_value(row_id as usize, col_idx) {
                    Some(val) => out_col.push(val.clone()),
                    None => out_col.push(Value::Null),
                }
            }
        }

        let mut col_types = Vec::with_capacity(num_cols);
        let mut col_names: Vec<String> = Vec::with_capacity(num_cols);
        for col in &node_table.columns {
            col_types.push(col.logical_type);
            col_names.push(col.name.clone());
        }

        drop(node_table);

        // Convert column-major Vec<Vec<Value>> to DataChunks
        use kuzu_common::types::PhysicalTypeID;
        use kuzu_common::vector::{DataChunk, ValueVector};

        let num_rows = output_columns.first().map(|c| c.len()).unwrap_or(0);
        if num_rows == 0 {
            return Ok(vec![DataChunk::new(vec![])]);
        }

        let mut chunks = Vec::new();
        let chunk_size = 1024usize;
        for start in (0..num_rows).step_by(chunk_size) {
            let end = (start + chunk_size).min(num_rows);
            let count = end - start;
            let mut fields = Vec::with_capacity(num_cols);

            for col_idx in 0..num_cols {
                let col_data = &output_columns[col_idx];
                let phys_type = match col_types[col_idx] {
                    kuzu_common::types::LogicalTypeID::Bool => PhysicalTypeID::Bool,
                    kuzu_common::types::LogicalTypeID::Int64 | kuzu_common::types::LogicalTypeID::Serial => PhysicalTypeID::Int64,
                    kuzu_common::types::LogicalTypeID::Int32 => PhysicalTypeID::Int32,
                    kuzu_common::types::LogicalTypeID::Int16 => PhysicalTypeID::Int16,
                    kuzu_common::types::LogicalTypeID::Int8 => PhysicalTypeID::Int8,
                    kuzu_common::types::LogicalTypeID::UInt64 => PhysicalTypeID::UInt64,
                    kuzu_common::types::LogicalTypeID::UInt32 => PhysicalTypeID::UInt32,
                    kuzu_common::types::LogicalTypeID::UInt16 => PhysicalTypeID::UInt16,
                    kuzu_common::types::LogicalTypeID::UInt8 => PhysicalTypeID::UInt8,
                    kuzu_common::types::LogicalTypeID::Double => PhysicalTypeID::Double,
                    kuzu_common::types::LogicalTypeID::Float => PhysicalTypeID::Float,
                    kuzu_common::types::LogicalTypeID::String => PhysicalTypeID::String,
                    kuzu_common::types::LogicalTypeID::Blob => PhysicalTypeID::Blob,
                    kuzu_common::types::LogicalTypeID::Date => PhysicalTypeID::Int32,
                    kuzu_common::types::LogicalTypeID::Timestamp => PhysicalTypeID::Int64,
                    kuzu_common::types::LogicalTypeID::Interval => PhysicalTypeID::Interval,
                    kuzu_common::types::LogicalTypeID::List => PhysicalTypeID::List,
                    kuzu_common::types::LogicalTypeID::Array => PhysicalTypeID::Array,
                    kuzu_common::types::LogicalTypeID::Struct => PhysicalTypeID::Struct,
                    kuzu_common::types::LogicalTypeID::Node => PhysicalTypeID::Struct,
                    kuzu_common::types::LogicalTypeID::Rel => PhysicalTypeID::Struct,
                    kuzu_common::types::LogicalTypeID::InternalID => PhysicalTypeID::Struct, // Internal IDs are Structs
                    _ => PhysicalTypeID::Any,
                };
                let mut vv = ValueVector::new(phys_type, count);
                vv.resize(count);
                for row_offset in 0..count {
                    let val = &col_data[start + row_offset];
                    match val {
                        Value::Null => vv.set_null(row_offset, true),
                        Value::Int64(x) => {
                            let buf = &mut vv.data_mut()[row_offset * 8..(row_offset + 1) * 8];
                            buf.copy_from_slice(&x.to_le_bytes());
                        }
                        Value::Int32(x) => {
                            let buf = &mut vv.data_mut()[row_offset * 4..(row_offset + 1) * 4];
                            buf.copy_from_slice(&x.to_le_bytes());
                        }
                        Value::Double(x) => {
                            let buf = &mut vv.data_mut()[row_offset * 8..(row_offset + 1) * 8];
                            buf.copy_from_slice(&x.to_le_bytes());
                        }
                        Value::String(s) => {
                            let bytes = s.as_bytes();
                            let copy_len = bytes.len().min(15);
                            vv.data_mut()[row_offset * 16] = copy_len as u8;
                            vv.data_mut()[row_offset * 16 + 1..row_offset * 16 + 1 + copy_len]
                                .copy_from_slice(&bytes[..copy_len]);
                        }
                        _ => {}
                    }
                }
                fields.push(vv);
            }

            chunks.push(DataChunk { fields, size: count, field_names: col_names.clone() });
        }

        Ok(chunks)
    }
}

// ==================== PhysicalExplain ====================

/// Physical EXPLAIN operator — serializes a logical plan tree to a human-readable
/// string and returns it as a single-row result.
///
/// Corresponds to C++ `PlanPrinter::printPlanToOstream` and `mapExplain`.
pub struct PhysicalExplain {
    /// The inner logical operator tree to serialize.
    pub inner_plan: String,
}

impl PhysicalOperatorExec for PhysicalExplain {
    fn operator_type(&self) -> &str {
        "explain"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        use kuzu_common::types::PhysicalTypeID;
        use kuzu_common::vector::{DataChunk, ValueVector};

        let plan_str = self.inner_plan.clone();
        let mut vv = ValueVector::new(PhysicalTypeID::String, 1);
        vv.resize(1);
        let bytes = plan_str.as_bytes();
        let copy_len = bytes.len().min(15);
        vv.data_mut()[0] = copy_len as u8;
        if copy_len > 0 {
            vv.data_mut()[1..1 + copy_len].copy_from_slice(&bytes[..copy_len]);
        }
        // For long strings, store the full string in the ValueVector's overflow
        // We store the original Value for the query result
        let chunk = DataChunk {
            fields: vec![vv],
            size: 1,
            field_names: vec![],
        };
        Ok(vec![chunk])
    }
}

// ==================== RecursiveExtend ====================

/// Physical operator for variable-length path matching (BFS traversal).
///
/// For each source node, performs BFS up to `upper_bound` depth and emits
/// result rows for all nodes reachable at depths between `lower_bound` and
/// `upper_bound`.
///
/// Uses GDS-style path tracking to record actual paths (node IDs + edge IDs)
/// and enforces path semantics (WALK/TRAIL/ACYCLIC).
///
/// Produces a DataChunk with columns:
///   (src_offset, dst_offset, length, path_node_ids, path_edge_ids[, cost])
///
/// When `weight_property` is `Some`, uses Dijkstra's algorithm for weighted
/// shortest path traversal (port of C++ `WeightedSPPathsFunction`).
/// The `cost` column is appended to the output.
pub struct PhysicalRecursiveExtend {
    pub source_table_id: u64,
    pub rel_table_ids: Vec<u64>,
    pub lower_bound: u64,
    pub upper_bound: u64,
    pub direction: kuzu_common::enums::ExtendDirection,
    pub semantic: kuzu_common::enums::PathSemantic,
    pub table_catalog: Option<Arc<TableCatalog>>,
    /// Optional edge weight property name for weighted shortest path.
    /// When set, Dijkstra traversal is used instead of BFS.
    pub weight_property: Option<String>,
    /// Optional name for the cost output column.
    pub cost_output_name: Option<String>,
}

impl PhysicalOperatorExec for PhysicalRecursiveExtend {
    fn operator_type(&self) -> &str {
        "recursive_extend"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        use kuzu_common::enums::ExtendDirection;
        use kuzu_common::enums::PathSemantic;
        use kuzu_common::types::Value;
        use kuzu_common::vector::ValueVector;
        use std::collections::{HashMap, VecDeque};

        let catalog = self
            .table_catalog
            .as_ref()
            .ok_or_else(|| "No table catalog available for RecursiveExtend".to_string())?;

        // Build adjacency with edge IDs: neighbor_offset -> (neighbor_offset, edge_id)
        let mut fwd_adj: HashMap<u64, Vec<(u64, u64)>> = HashMap::new();
        let mut rev_adj: HashMap<u64, Vec<(u64, u64)>> = HashMap::new();
        // Edge weight lookup: edge_id -> weight (for weighted shortest path)
        let mut edge_weights: HashMap<u64, f64> = HashMap::new();
        // Whether we're doing weighted shortest path
        let is_weighted = self.weight_property.is_some();
        // Resolve weight column index for each rel table
        let mut weight_col_idx: HashMap<u64, Option<usize>> = HashMap::new();

        for &rel_table_id in &self.rel_table_ids {
            if let Some(rel_table) = catalog.get_rel_table(rel_table_id) {
                // Resolve weight column index
                if let Some(ref wp) = self.weight_property {
                    let idx = rel_table.columns.iter().position(|c| c.name == *wp);
                    weight_col_idx.insert(rel_table_id, idx);
                }

                for (&src, neighbors) in rel_table.fwd_adj.iter() {
                    fwd_adj.entry(src).or_default().extend(
                        neighbors.iter().map(|(dst, edge_idx)| (*dst, *edge_idx as u64))
                    );
                    // Pre-compute edge weights
                    if is_weighted
                        && let Some(col_idx) = weight_col_idx.get(&rel_table_id).and_then(|&c| c) {
                            for &(_dst, edge_idx) in neighbors {
                                if let Some(weight_val) = rel_table.properties.get(col_idx).and_then(|col| col.get(edge_idx)) {
                                    let w = match weight_val {
                                        Value::Int64(i) => *i as f64,
                                        Value::Double(d) => *d,
                                        Value::Float(f) => *f as f64,
                                        Value::Int32(i) => *i as f64,
                                        _ => 1.0, // default weight for unrecognized types
                                    };
                                    edge_weights.insert(edge_idx as u64, w);
                                }
                            }
                        }
                }
                for (&dst, neighbors) in rel_table.rev_adj.iter() {
                    rev_adj.entry(dst).or_default().extend(
                        neighbors.iter().map(|(src, edge_idx)| (*src, *edge_idx as u64))
                    );
                }
            }
        }

        // Collect source node offsets from input
        let source_offsets: Vec<i64> = if input.is_empty() || input[0].fields.is_empty() {
            let mut all: Vec<i64> = fwd_adj.keys().chain(rev_adj.keys()).copied().map(|k| k as i64).collect();
            all.sort();
            all.dedup();
            all
        } else {
            let field = &input[0].fields[0];
            let num_rows = input[0].size;
            let mut offsets = Vec::with_capacity(num_rows);
            for i in 0..num_rows {
                if !field.is_null(i) {
                    let offset = i64::from_le_bytes(
                        field.data()[i * 8..i * 8 + 8].try_into().unwrap(),
                    );
                    offsets.push(offset);
                }
            }
            offsets
        };

        if source_offsets.is_empty() {
            return Ok(vec![DataChunk::new(vec![])]);
        }

        // Result columns
        let mut result_src: Vec<i64> = Vec::new();
        let mut result_dst: Vec<i64> = Vec::new();
        let mut result_len: Vec<i64> = Vec::new();
        let mut result_cost: Vec<f64> = Vec::new(); // only used for weighted
        // Path tracking: for each result, store the sequence of (node_id, edge_id) pairs
        let mut result_path_nodes: Vec<Vec<i64>> = Vec::new();
        let mut result_path_edges: Vec<Vec<i64>> = Vec::new();

        for &src in &source_offsets {
            let src_u = src as u64;

            if is_weighted {
                // === Weighted Shortest Path: Dijkstra ===
                use std::cmp::Reverse;
                use std::collections::BinaryHeap;

                // Use i64 for priority queue (cost * PRECISION) since f64 doesn't implement Ord.
                // PRECISION = 1000 captures 3 decimal places.
                const COST_PRECISION: i64 = 1000;

                // Helper to convert f64 cost to i64 for the pq
                let cost_to_i64 = |c: f64| -> i64 { (c * COST_PRECISION as f64).round() as i64 };

                // Parent map: child -> (parent, edge_id, depth, cumulative_cost)
                let mut parents: HashMap<u64, (u64, u64, u64, f64)> = HashMap::new();
                let mut pq: BinaryHeap<Reverse<(i64, u64)>> = BinaryHeap::new();

                pq.push(Reverse((cost_to_i64(0.0), src_u)));
                parents.insert(src_u, (u64::MAX, u64::MAX, 0, 0.0));

                while let Some(Reverse((cur_cost_i64, node))) = pq.pop() {
                    let cur_cost = cur_cost_i64 as f64 / COST_PRECISION as f64;
                    let cur_depth = parents.get(&node).map(|&(_, _, d, _)| d).unwrap_or(0);

                    // If we already found a better path to this node, skip
                    if let Some(&(_, _, _, best_cost)) = parents.get(&node)
                        && cur_cost > best_cost + 1e-9 {
                            continue;
                        }

                    if cur_depth >= self.upper_bound {
                        continue;
                    }

                    // Get neighbors
                    let neighbors: Vec<(u64, u64)> = match self.direction {
                        ExtendDirection::Fwd => fwd_adj.get(&node).cloned().unwrap_or_default(),
                        ExtendDirection::Bwd => rev_adj.get(&node).cloned().unwrap_or_default(),
                        ExtendDirection::Both => {
                            let mut nbrs = fwd_adj.get(&node).cloned().unwrap_or_default();
                            if let Some(bwd) = rev_adj.get(&node) {
                                nbrs.extend(bwd.iter().copied());
                            }
                            nbrs
                        }
                    };

                    for (nbr, edge_id) in neighbors {
                        let edge_w = edge_weights.get(&edge_id).copied().unwrap_or(1.0);
                        let new_cost = cur_cost + edge_w;
                        let new_depth = cur_depth + 1;

                        let should_visit = match parents.get(&nbr) {
                            Some(&(_, _, _, existing_cost)) => new_cost < existing_cost - 1e-9,
                            None => true,
                        };

                        if should_visit {
                            parents.insert(nbr, (node, edge_id, new_depth, new_cost));
                            pq.push(Reverse((cost_to_i64(new_cost), nbr)));
                        }
                    }
                }

                // Emit results
                for (&node, &(_parent, _eid, depth, cost)) in &parents {
                    if depth < self.lower_bound || depth > self.upper_bound {
                        continue;
                    }
                    if depth == 0 && self.lower_bound > 0 {
                        continue;
                    }

                    result_src.push(src);
                    result_dst.push(node as i64);
                    result_len.push(depth as i64);
                    result_cost.push(cost);

                    // Reconstruct path
                    let mut cur = node;
                    let mut temp_nodes = vec![node as i64];
                    let mut temp_edges = Vec::new();

                    while cur != src_u {
                        if let Some(&(parent, eid, _, _)) = parents.get(&cur) {
                            if parent == u64::MAX { break; }
                            temp_edges.push(eid as i64);
                            temp_nodes.push(parent as i64);
                            cur = parent;
                        } else { break; }
                    }

                    temp_nodes.reverse();
                    temp_edges.reverse();
                    let mut path_nodes = vec![src];
                    path_nodes.extend(temp_nodes);
                    result_path_nodes.push(path_nodes);
                    result_path_edges.push(temp_edges);
                }
            } else {
                // === Unweighted: BFS ===
                let mut queue = VecDeque::new();
                // Parent map: child -> (parent, edge_id, depth)
                let mut parents: HashMap<u64, (u64, u64, u64)> = HashMap::new();
                queue.push_back((src_u, 0u64));
                parents.insert(src_u, (u64::MAX, u64::MAX, 0));

                let semantic = self.semantic;

                while let Some((node, depth)) = queue.pop_front() {
                    if depth >= self.upper_bound { continue; }

                    let neighbors: Vec<(u64, u64)> = match self.direction {
                        ExtendDirection::Fwd => fwd_adj.get(&node).cloned().unwrap_or_default(),
                        ExtendDirection::Bwd => rev_adj.get(&node).cloned().unwrap_or_default(),
                        ExtendDirection::Both => {
                            let mut nbrs = fwd_adj.get(&node).cloned().unwrap_or_default();
                            if let Some(bwd) = rev_adj.get(&node) {
                                nbrs.extend(bwd.iter().copied());
                            }
                            nbrs
                        }
                    };

                    'neighbors: for (nbr, edge_id) in neighbors {
                        if parents.contains_key(&nbr) {
                            match semantic {
                                PathSemantic::Walk | PathSemantic::Acyclic => continue 'neighbors,
                                PathSemantic::Trail => {
                                    let mut cur = node;
                                    while let Some(&(p, eid, _)) = parents.get(&cur) {
                                        if eid == edge_id { continue 'neighbors; }
                                        if p == u64::MAX { break; }
                                        cur = p;
                                    }
                                }
                            }
                        }

                        let new_depth = depth + 1;
                        parents.insert(nbr, (node, edge_id, new_depth));
                        queue.push_back((nbr, new_depth));
                    }
                }

                // Emit results for nodes at valid depths
                for (&node, &(_parent_node, _edge_id, depth)) in &parents {
                    if depth < self.lower_bound || depth > self.upper_bound { continue; }
                    if depth == 0 && self.lower_bound > 0 { continue; }

                    result_src.push(src);
                    result_dst.push(node as i64);
                    result_len.push(depth as i64);

                    // Reconstruct path
                    let mut cur = node;
                    let mut temp_nodes = vec![node as i64];
                    let mut temp_edges = Vec::new();

                    while cur != src_u {
                        if let Some(&(parent, eid, _)) = parents.get(&cur) {
                            if parent == u64::MAX { break; }
                            temp_edges.push(eid as i64);
                            temp_nodes.push(parent as i64);
                            cur = parent;
                        } else { break; }
                    }

                    temp_nodes.reverse();
                    temp_edges.reverse();
                    let mut path_nodes = vec![src];
                    path_nodes.extend(temp_nodes);
                    result_path_nodes.push(path_nodes);
                    result_path_edges.push(temp_edges);
                }
            }
        }

        // Build output DataChunk
        let num_results = result_src.len();
        if num_results == 0 {
            return Ok(vec![DataChunk::new(vec![])]);
        }

        // Column 0-2: primitive Int64 vectors
        let mut src_v = ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, num_results);
        let mut dst_v = ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, num_results);
        let mut len_v = ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, num_results);

        for i in 0..num_results {
            let offset = i * 8;
            src_v.data_mut()[offset..offset + 8].copy_from_slice(&result_src[i].to_le_bytes());
            src_v.set_null(i, false);
            dst_v.data_mut()[offset..offset + 8].copy_from_slice(&result_dst[i].to_le_bytes());
            dst_v.set_null(i, false);
            len_v.data_mut()[offset..offset + 8].copy_from_slice(&result_len[i].to_le_bytes());
            len_v.set_null(i, false);
        }
        src_v.resize(num_results);
        dst_v.resize(num_results);
        len_v.resize(num_results);

        // Column 3-4: create Value vectors for path lists, then convert to columns
        // Use Vec<Option<Value>> to store per-row path data
        let mut path_nodes_col: Vec<Value> = Vec::with_capacity(num_results);
        let mut path_edges_col: Vec<Value> = Vec::with_capacity(num_results);

        for i in 0..num_results {
            // Path nodes as List(Int64)
            let node_vals: Vec<Value> = result_path_nodes[i].iter().map(|&n| Value::Int64(n)).collect();
            path_nodes_col.push(Value::List(node_vals));
            // Path edges as List(Int64)
            let edge_vals: Vec<Value> = result_path_edges[i].iter().map(|&e| Value::Int64(e)).collect();
            path_edges_col.push(Value::List(edge_vals));
        }

        // Store List values in ValueVector via set_value
        let mut path_nodes_v = ValueVector::new(kuzu_common::types::PhysicalTypeID::List, num_results);
        let mut path_edges_v = ValueVector::new(kuzu_common::types::PhysicalTypeID::List, num_results);

        for (i, val) in path_nodes_col.iter().enumerate() {
            path_nodes_v.set_value(i, val).ok();
        }
        for (i, val) in path_edges_col.iter().enumerate() {
            path_edges_v.set_value(i, val).ok();
        }

        // When weighted, include cost column
        let has_cost = is_weighted;

        if has_cost {
            let mut cost_v = ValueVector::new(kuzu_common::types::PhysicalTypeID::Double, num_results);
            for (i, cost) in result_cost.iter().enumerate().take(num_results) {
                let offset = i * 8;
                cost_v.data_mut()[offset..offset + 8].copy_from_slice(&cost.to_le_bytes());
                cost_v.set_null(i, false);
            }
            cost_v.resize(num_results);

            Ok(vec![DataChunk {
                fields: vec![src_v, dst_v, len_v, path_nodes_v, path_edges_v, cost_v],
                size: num_results,
                field_names: vec![],
            }])
        } else {
            Ok(vec![DataChunk {
                fields: vec![src_v, dst_v, len_v, path_nodes_v, path_edges_v],
                size: num_results,
                field_names: vec![],
            }])
        }
    }
}

pub struct PhysicalCreateNode {
    pub table_name: String,
    pub table_id: u64,
    pub out_var_name: String,
    pub properties: Vec<(String, kuzu_parser::ast::Expression)>,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalCreateNode {
    pub fn execute(&self, input: Vec<DataChunk>) -> Result<Vec<DataChunk>, String> {
        if input.is_empty() {
            return Ok(input);
        }

        let mut table = self.table_catalog.get_node_table_by_name_mut(&self.table_name)
            .ok_or_else(|| format!("Node table {} not found", self.table_name))?;

        // For each input chunk, we create nodes and attach the new node IDs
        let mut output = Vec::with_capacity(input.len());
        
        for mut chunk in input {
            let mut node_ids = ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, chunk.size);
            
            for i in 0..chunk.size {
                let mut values = vec![kuzu_common::types::Value::Null; table.columns.len()];
                for (prop_name, prop_expr) in &self.properties {
                    if let Some(col_idx) = table.columns.iter().position(|c| c.name == *prop_name) {
                        values[col_idx] = evaluate_expression_for_row(prop_expr, &chunk, i);
                    }
                }
                
                let row_offset = table.insert_row(values)?;
                node_ids.data_mut()[i * 8..(i + 1) * 8].copy_from_slice(&(row_offset as i64).to_le_bytes());
                node_ids.set_null(i, false);
            }
            node_ids.resize(chunk.size);
            
            chunk.fields.push(node_ids);
            chunk.field_names.push(self.out_var_name.clone());
            output.push(chunk);
        }

        Ok(output)
    }
}

pub struct PhysicalCreateRel {
    pub table_name: String,
    pub table_id: u64,
    pub src_node_name: String,
    pub dst_node_name: String,
    pub properties: Vec<(String, kuzu_parser::ast::Expression)>,
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalCreateRel {
    pub fn execute(&self, input: Vec<DataChunk>) -> Result<Vec<DataChunk>, String> {
        if input.is_empty() {
            return Ok(input);
        }

        let mut table = self.table_catalog.get_rel_table_by_name_mut(&self.table_name)
            .ok_or_else(|| format!("Rel table {} not found", self.table_name))?;

        let mut output = Vec::with_capacity(input.len());
        
        for chunk in input {
            let src_name_id = format!("{}.{}", self.src_node_name, "_id");
            let src_name_pk = format!("{}.{}", self.src_node_name, "id");
            let src_idx = chunk.field_names.iter().position(|name| name == &src_name_id)
                .or_else(|| chunk.field_names.iter().position(|name| name == &self.src_node_name))
                .or_else(|| chunk.field_names.iter().position(|name| name == &src_name_pk))
                .ok_or_else(|| format!("Source node variable {} not found", self.src_node_name))?;
                
            let dst_name_id = format!("{}.{}", self.dst_node_name, "_id");
            let dst_name_pk = format!("{}.{}", self.dst_node_name, "id");
            let dst_idx = chunk.field_names.iter().position(|name| name == &dst_name_id)
                .or_else(|| chunk.field_names.iter().position(|name| name == &self.dst_node_name))
                .or_else(|| chunk.field_names.iter().position(|name| name == &dst_name_pk))
                .ok_or_else(|| format!("Destination node variable {} not found", self.dst_node_name))?;
                
            let src_vec = &chunk.fields[src_idx];
            let dst_vec = &chunk.fields[dst_idx];

            let mut inserted = 0;
            for i in 0..chunk.size {
                if src_vec.is_null(i) || dst_vec.is_null(i) {
                    continue; // Skip creating relationships involving NULL nodes
                }
                
                let mut src_bytes = [0u8; 8];
                src_bytes.copy_from_slice(&src_vec.data()[i * 8..(i + 1) * 8]);
                let src_id = i64::from_le_bytes(src_bytes) as u64;

                let mut dst_bytes = [0u8; 8];
                dst_bytes.copy_from_slice(&dst_vec.data()[i * 8..(i + 1) * 8]);
                let dst_id = i64::from_le_bytes(dst_bytes) as u64;

                let mut values = vec![kuzu_common::types::Value::Null; table.columns.len()];
                for (prop_name, prop_expr) in &self.properties {
                    if let Some(col_idx) = table.columns.iter().position(|c| c.name == *prop_name) {
                        values[col_idx] = evaluate_expression_for_row(prop_expr, &chunk, i);
                    }
                }

                table.insert_rel(src_id, dst_id, values)?;
                inserted += 1;
            }
            println!("PhysicalCreateRel inserted {} relationships from chunk of size {}", inserted, chunk.size);
            
            output.push(chunk);
        }

        Ok(output)
    }
}

/// Physical operator for extending from a source node through a relationship.
///
/// Takes input chunks from the source node scan, and for each source row,
/// looks up adjacency list entries in the relationship table, producing
/// output rows that include the source fields, relationship properties,
/// and destination node properties.
///
/// Ported from C++ `ScanRelTable` (the physical extend operator).
pub struct PhysicalExtend {
    /// Name of the relationship table.
    pub rel_table_name: String,
    /// ID of the relationship table.
    pub rel_table_id: u64,
    /// Variable name of the bound (source) node.
    pub bound_node_var: String,
    /// Direction of the extend.
    pub direction: kuzu_parser::ast::EdgeDirection,
    /// Variable name of the destination node.
    pub dst_node_var: String,
    /// Table name of the destination node.
    pub dst_table_name: String,
    /// Table ID of the destination node.
    pub dst_table_id: u64,
    /// Table catalog for data access.
    pub table_catalog: Arc<TableCatalog>,
}

impl PhysicalExtend {
    pub fn execute(&self, input: Vec<DataChunk>) -> Result<Vec<DataChunk>, String> {
        if input.is_empty() || input.iter().all(|c| c.size == 0) {
            return Ok(input);
        }

        // Collect rel table data upfront (owned)
        let (fwd_adj, rev_adj, rel_props, rel_cols) = {
            let rel_table = self.table_catalog.get_rel_table_by_name(&self.rel_table_name)
                .ok_or_else(|| format!("Rel table {} not found", self.rel_table_name))?;
            let fwd = rel_table.fwd_adj.clone();
            let rev = rel_table.rev_adj.clone();
            let props = rel_table.properties.clone();
            let cols = rel_table.columns.clone();
            (fwd, rev, props, cols)
        };

        // Collect dest node table data upfront (owned)
        let (dest_data, dest_cols, dest_pk_col) = {
            let dest_table = self.table_catalog.get_node_table_by_name(&self.dst_table_name)
                .ok_or_else(|| format!("Node table {} not found", self.dst_table_name))?;
            let data = dest_table.to_column_major_data();
            let cols = dest_table.columns.clone();
            let pk = dest_table.primary_key_column;
            (data, cols, pk)
        };

        // Build PK → row offset map for destination lookups
        let pk_to_row: std::collections::HashMap<u64, usize> = if dest_pk_col < dest_data.len() {
            dest_data[dest_pk_col].iter().enumerate()
                .filter_map(|(row, val)| {
                    if let Value::Int64(id) = val { Some((*id as u64, row)) }
                    else { None }
                })
                .collect()
        } else {
            std::collections::HashMap::new()
        };

        let mut output = Vec::with_capacity(input.len());

        for chunk in input {
            // Find the bound node column in the chunk
            let bound_name_id = format!("{}.{}", self.bound_node_var, "_id");
            let bound_name_pk = format!("{}.{}", self.bound_node_var, "id");
            let bound_idx = chunk.field_names.iter().position(|name| name == &bound_name_id)
                .or_else(|| chunk.field_names.iter().position(|name| name == &self.bound_node_var))
                .or_else(|| chunk.field_names.iter().position(|name| name == &bound_name_pk))
                .ok_or_else(|| {
                    format!("Bound node variable {} not found in Extend input. Available fields: {:?}",
                        self.bound_node_var, chunk.field_names)
                })?;

            // Calculate total output rows and build row mapping
            let mut total_rows = 0;
            let mut row_mappings: Vec<(usize, u64, usize)> = Vec::new(); // (input_row, dst_offset, edge_idx)

            for i in 0..chunk.size {
                if chunk.fields[bound_idx].is_null(i) {
                    continue;
                }
                let offset = i * 8;
                let src_vec_data = chunk.fields[bound_idx].data();
                if offset + 8 > src_vec_data.len() {
                    continue;
                }
                let mut src_bytes = [0u8; 8];
                src_bytes.copy_from_slice(&src_vec_data[offset..offset + 8]);
                let src_id = i64::from_le_bytes(src_bytes) as u64;

                let edges: Vec<(u64, usize)> = match self.direction {
                    kuzu_parser::ast::EdgeDirection::LeftToRight => {
                        fwd_adj.get(&src_id).cloned().unwrap_or_default()
                    }
                    kuzu_parser::ast::EdgeDirection::RightToLeft => {
                        rev_adj.get(&src_id).cloned().unwrap_or_default()
                    }
                    kuzu_parser::ast::EdgeDirection::Both => {
                        let mut all = fwd_adj.get(&src_id).cloned().unwrap_or_default();
                        if let Some(rev) = rev_adj.get(&src_id) {
                            all.extend(rev.clone());
                        }
                        all
                    }
                };

                for &(dst_offset, edge_idx) in &edges {
                    if !pk_to_row.contains_key(&dst_offset) {
                        if dst_offset as usize >= dest_data.first().map(|c| c.len()).unwrap_or(0) {
                            continue;
                        }
                    }
                    total_rows += 1;
                    row_mappings.push((i, dst_offset, edge_idx));
                }
            }

            if total_rows == 0 {
                output.push(DataChunk::new(vec![]));
                continue;
            }

            // Build output:
            // Column layout: [input_fields | rel_properties | dest_node_fields]
            let num_input_fields = chunk.fields.len();
            let num_rel_cols = rel_cols.len();
            let num_dest_cols = dest_cols.len();
            let num_out_cols = num_input_fields + num_rel_cols + num_dest_cols;

            // Build column-major data
            let mut out_data: Vec<Vec<Value>> = vec![Vec::with_capacity(total_rows); num_out_cols];

            for &(input_row, dst_offset, edge_idx) in &row_mappings {
                // Copy input fields
                for col in 0..num_input_fields {
                    let val = chunk.fields[col].get_value(input_row).unwrap_or(Value::Null);
                    out_data[col].push(val);
                }
                // Copy rel properties
                for col in 0..num_rel_cols {
                    let val = rel_props.get(col)
                        .and_then(|c| c.get(edge_idx))
                        .cloned()
                        .unwrap_or(Value::Null);
                    out_data[num_input_fields + col].push(val);
                }
                // Copy dest node properties
                let dest_row = pk_to_row.get(&dst_offset).copied();
                for col in 0..num_dest_cols {
                    let val = dest_row
                        .and_then(|r| dest_data.get(col).and_then(|c| c.get(r)))
                        .cloned()
                        .unwrap_or_else(|| {
                            dest_data.get(col)
                                .and_then(|c| c.get(dst_offset as usize))
                                .cloned()
                                .unwrap_or(Value::Null)
                        });
                    out_data[num_input_fields + num_rel_cols + col].push(val);
                }
            }

            // Convert column-major data to ValueVectors
            let mut fields = Vec::with_capacity(num_out_cols);
            let mut field_names = Vec::with_capacity(num_out_cols);

            // Input field names (already prefixed)
            for col in 0..num_input_fields {
                let phys_type = chunk.fields[col].physical_type();
                let mut v = ValueVector::new(phys_type, total_rows);
                v.resize(total_rows);
                for row in 0..total_rows {
                    store_value_in_vector(&mut v, row, &out_data[col][row]);
                }
                fields.push(v);
                if col < chunk.field_names.len() {
                    field_names.push(chunk.field_names[col].clone());
                } else {
                    field_names.push(format!("field_{}", col));
                }
            }

            // Rel field names (prefixed with rel table name)
            for col in 0..num_rel_cols {
                let phys_type = if col < rel_cols.len() {
                    PhysicalScan::logical_to_physical(&rel_cols[col].logical_type)
                } else {
                    PhysicalTypeID::Int64
                };
                let mut v = ValueVector::new(phys_type, total_rows);
                v.resize(total_rows);
                for row in 0..total_rows {
                    store_value_in_vector(&mut v, row, &out_data[num_input_fields + col][row]);
                }
                fields.push(v);
                let rel_prefix = &self.rel_table_name;
                let col_name = rel_cols.get(col).map(|c| c.name.as_str()).unwrap_or("");
                field_names.push(format!("{}.{}", rel_prefix, col_name));
            }

            // Dest field names (prefixed with dest variable)
            for col in 0..num_dest_cols {
                let phys_type = if col < dest_cols.len() {
                    PhysicalScan::logical_to_physical(&dest_cols[col].logical_type)
                } else {
                    PhysicalTypeID::Int64
                };
                let mut v = ValueVector::new(phys_type, total_rows);
                v.resize(total_rows);
                for row in 0..total_rows {
                    store_value_in_vector(&mut v, row, &out_data[num_input_fields + num_rel_cols + col][row]);
                }
                fields.push(v);
                let prefix = &self.dst_node_var;
                let col_name = dest_cols.get(col).map(|c| c.name.as_str()).unwrap_or("");
                field_names.push(format!("{}.{}", prefix, col_name));
            }

            output.push(DataChunk {
                fields,
                size: total_rows,
                field_names,
            });
        }

        Ok(output)
    }
}
