//! Physical operator types and execution logic for query processing.
//!
//! Each physical operator implements the `Operator` trait with an `execute` method
//! that produces output `DataChunk`s from input `DataChunk`s.

use kuzu_common::types::{LogicalTypeID, PhysicalTypeID, Value};
use kuzu_common::vector::{physical_type_size, DataChunk, ValueVector};
use kuzu_function::scalar::AggValueState;
use kuzu_function::AggregateFunction;
use kuzu_parser::ast::{BinaryOp, Constant, Expression, UnaryOp};
use kuzu_storage::table::{ColumnDefinition, TableCatalog};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::expression_evaluator::ExpressionEvaluator;

/// Result of executing a physical operator.
pub type OperatorResult = Result<Vec<DataChunk>, String>;

/// Trait shared by all physical operators.
pub trait PhysicalOperatorExec {
    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult;
    fn operator_type(&self) -> &str;
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
}

impl PhysicalScan {
    pub fn new(
        table_name: String,
        table_id: u64,
        estimated_cardinality: u64,
    ) -> Self {
        Self {
            table_name,
            table_id,
            column_ids: Vec::new(),
            estimated_cardinality,
            table_data: None,
            table_columns: Vec::new(),
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
                        v.data_mut()[offset + 1..offset + 1 + copy_len]
                            .copy_from_slice(&bytes[..copy_len]);
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
            Value::String(_) | Value::Date(_) | Value::Timestamp(_)
                | Value::TimestampTz(_) | Value::TimestampNs(_)
                | Value::TimestampMs(_) | Value::TimestampSec(_)
                | Value::Interval(_) => PhysicalTypeID::String,
            Value::Blob(_) => PhysicalTypeID::Blob,
            Value::InternalID(_) | Value::List(_) | Value::Map(_) | Value::Struct(_) => {
                PhysicalTypeID::Int64
            }
        }
    }

    /// Determine PhysicalTypeID from a LogicalTypeID.
    fn logical_to_physical(logical: &LogicalTypeID) -> PhysicalTypeID {
        match logical {
            LogicalTypeID::Bool => PhysicalTypeID::Bool,
            LogicalTypeID::Int64 | LogicalTypeID::UInt64 | LogicalTypeID::Int128 | LogicalTypeID::Serial => PhysicalTypeID::Int64,
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
    fn operator_type(&self) -> &str { "scan" }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        // If we have real table data, read from it
        if let Some(ref data) = self.table_data {
            if data.is_empty() || data[0].is_empty() {
                return Ok(vec![DataChunk::new(vec![])]);
            }

            let num_rows = data[0].len();
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
                    col_data.iter().find_map(|v| {
                        if !matches!(v, Value::Null) {
                            Some(Self::value_to_physical_type(v))
                        } else {
                            None
                        }
                    }).unwrap_or(PhysicalTypeID::Int64)
                };

                let mut v = ValueVector::new(phys_type, num_rows);
                v.resize(num_rows);
                for (row, val) in col_data.iter().enumerate() {
                    Self::write_value_to_vector(&mut v, row, val);
                }
                fields.push(v);
            }

            let chunk = DataChunk::new(fields);
            return Ok(vec![chunk]);
        }

        // Fallback: no data available — return empty result
        Ok(vec![DataChunk::new(vec![])])
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
                    None => mask.push(false),    // null = false
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
                        // Negation as filter doesn't change boolean mask
                        Ok(vals)
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
            Expression::FunctionCall(_, _) | Expression::List(_) | Expression::Map(_) | Expression::Parameter(_) => {
                Ok(vec![true; chunk.size])
            }
        }
    }
}

impl PhysicalOperatorExec for PhysicalFilter {
    fn operator_type(&self) -> &str { "filter" }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let evaluator = self.evaluator.as_ref()
            .and_then(|e| e.lock().ok());

        let mut output = Vec::new();
        for chunk in input {
            let mask = Self::evaluate_expression(
                &self.expression,
                &chunk,
                evaluator.as_deref(),
            )?;
            // Filter rows based on mask
            let selected: Vec<usize> = mask.iter().enumerate()
                .filter(|&(_, v)| *v).map(|(i, _)| i).collect();

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
            output.push(DataChunk::new(new_fields));
        }
        Ok(output)
    }
}

fn evaluate_binary_op_legacy(
    op: &BinaryOp,
    left: &[bool],
    right: &[bool],
    size: usize,
) -> Result<Vec<bool>, String> {
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
    fn operator_type(&self) -> &str { "projection" }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let output: Vec<DataChunk> = input
            .into_iter()
            .map(|chunk| {
                let fields: Vec<ValueVector> = self
                    .column_indices
                    .iter()
                    .filter_map(|&i| chunk.fields.get(i).cloned())
                    .collect();
                let size = fields.first().map(|f| f.size()).unwrap_or(0);
                DataChunk { fields, size }
            })
            .collect();

        if output.is_empty() {
            Ok(vec![DataChunk { fields: vec![], size: 0 }])
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
    fn operator_type(&self) -> &str { "limit" }

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

            // Apply offset
            if skipped + chunk_size <= skip {
                skipped += chunk_size;
                continue;
            }
            let start_in_chunk = if skipped < skip {
                (skip - skipped) as usize
            } else {
                0
            };
            skipped = skip.max(skipped + chunk_size);

            // Apply limit
            let available = chunk_size.saturating_sub(start_in_chunk as u64) as usize;
            let take = available.min(remaining as usize);
            remaining -= take as u64;

            if take == chunk.size {
                output.push(chunk);
            } else {
                // Create truncated chunk (simplified: just resize)
                let mut new_fields = Vec::new();
                for field in &chunk.fields {
                    let mut new_v = ValueVector::new(field.physical_type(), take);
                    new_v.resize(take); // Pre-allocate
                    let type_size = physical_type_size(field.physical_type());
                    let src_start = start_in_chunk * type_size;
                    let copy_size = take * type_size;
                    if src_start + copy_size <= field.data().len() && copy_size <= new_v.data().len() {
                        new_v.data_mut()[..copy_size].copy_from_slice(
                            &field.data()[src_start..src_start + copy_size],
                        );
                    }
                    for i in 0..take {
                        new_v.set_null(i, field.is_null(start_in_chunk + i));
                    }
                    new_fields.push(new_v);
                }
                output.push(DataChunk::new(new_fields));
            }
        }
        Ok(output)
    }
}

// ==================== OrderBy ====================

pub struct PhysicalOrderBy {
    pub sort_column: u32,
    pub ascending: bool,
}

impl PhysicalOperatorExec for PhysicalOrderBy {
    fn operator_type(&self) -> &str { "order_by" }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        // Collect all rows into a single buffer, sort, then output
        if input.is_empty() {
            return Ok(Vec::new());
        }

        // Count total rows
        let total_rows: usize = input.iter().map(|c| c.size).sum();
        if total_rows == 0 {
            return Ok(input);
        }

        // Collect all field values per column
        let num_fields = input[0].num_fields();
        let mut all_values: Vec<Vec<(i64, bool)>> = (0..num_fields)
            .map(|_| Vec::with_capacity(total_rows))
            .collect();

        for chunk in &input {
            for row in 0..chunk.size {
                for col in 0..num_fields {
                    if let Some(field) = chunk.fields.get(col) {
                        let val = field.get_i64(row).unwrap_or(0);
                        let is_null = field.is_null(row);
                        all_values[col].push((val, is_null));
                    }
                }
            }
        }

        // Sort indices based on sort_column
        let sort_col = self.sort_column as usize;
        let mut indices: Vec<usize> = (0..total_rows).collect();
        if sort_col < num_fields {
            let vals = &all_values[sort_col];
            if self.ascending {
                indices.sort_by(|a, b| vals[*a].0.cmp(&vals[*b].0));
            } else {
                indices.sort_by(|a, b| vals[*b].0.cmp(&vals[*a].0));
            }
        }

        // Build sorted output chunks (up to 100 rows per chunk)
        let chunk_size = 100usize;
        let mut output = Vec::new();
        for chunk_start in (0..total_rows).step_by(chunk_size) {
            let chunk_end = (chunk_start + chunk_size).min(total_rows);
            let size = chunk_end - chunk_start;
            let mut fields = Vec::new();
            for col in 0..num_fields {
                let mut v = ValueVector::new(PhysicalTypeID::Int64, size);
                for (out_idx, &src_idx) in indices[chunk_start..chunk_end].iter().enumerate() {
                    let (val, is_null) = all_values[col][src_idx];
                    v.set_i64(out_idx, val);
                    if is_null {
                        v.set_null(out_idx, true);
                    }
                }
                v.resize(size);
                fields.push(v);
            }
            output.push(DataChunk::new(fields));
        }
        Ok(output)
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
    fn operator_type(&self) -> &str { "aggregate" }

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
        let funcs: Vec<AggregateFunction> = self.aggregate_functions.iter()
            .map(|name| parse_aggregate_function(name))
            .collect();

        let mut states: Vec<AggValueState> = funcs.iter()
            .map(|f| AggValueState::new(f))
            .collect();

        for chunk in input {
            for row in 0..chunk.size {
                for (i, state) in states.iter_mut().enumerate() {
                    let col_idx = i.min(chunk.fields.len().saturating_sub(1));
                    let val = chunk.fields.get(col_idx)
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
        let group_col = self.group_by_cols.first().copied().unwrap_or(0) as usize;

        let funcs: Vec<AggregateFunction> = self.aggregate_functions.iter()
            .map(|name| parse_aggregate_function(name))
            .collect();

        // Hash map: group key → Vec of AggValueState (one per aggregate function)
        let mut groups: HashMap<i64, Vec<AggValueState>> = HashMap::new();

        for chunk in input {
            for row in 0..chunk.size {
                let key = chunk.fields.get(group_col)
                    .and_then(|f| f.get_i64(row))
                    .unwrap_or(0);

                let entry = groups.entry(key).or_insert_with(|| {
                    funcs.iter().map(|f| AggValueState::new(f)).collect()
                });

                for (i, state) in entry.iter_mut().enumerate() {
                    let val = chunk.fields.get(i.min(chunk.fields.len().saturating_sub(1)))
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
            let mut fields = Vec::new();
            for _ in 0..=self.aggregate_functions.len() {
                let mut v = ValueVector::new(PhysicalTypeID::Int64, 0);
                v.resize(0);
                fields.push(v);
            }
            return Ok(vec![DataChunk::new(fields)]);
        }

        let num_rows = groups.len();
        let num_agg = self.aggregate_functions.len();

        // For each column, collect final values
        let mut group_key_values: Vec<i64> = Vec::with_capacity(num_rows);
        let mut agg_results: Vec<Vec<Value>> = (0..num_agg).map(|_| Vec::with_capacity(num_rows)).collect();

        for (key, states) in &groups {
            group_key_values.push(*key);
            for (i, state) in states.iter().enumerate() {
                agg_results[i].push(state.finalize());
            }
        }

        // Build output vectors
        let mut output_fields = Vec::with_capacity(1 + num_agg);

        // Group key column (Int64)
        {
            let mut v = ValueVector::new(PhysicalTypeID::Int64, num_rows);
            v.resize(num_rows); // Must set size first so data() is writable
            for (row, &key) in group_key_values.iter().enumerate() {
                v.set_i64(row, key);
            }
            output_fields.push(v);
        }

        // Aggregate result columns
        for i in 0..num_agg {
            let first_val = &agg_results[i][0];
            let physical_type = first_val.physical_type();
            let mut v = ValueVector::new(physical_type, num_rows);
            v.resize(num_rows); // Must set size first so data() is writable
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

// ==================== HashJoin ====================

pub struct PhysicalHashJoin {
    pub build_columns: Vec<u32>,
    pub probe_columns: Vec<u32>,
}

impl PhysicalOperatorExec for PhysicalHashJoin {
    fn operator_type(&self) -> &str { "hash_join" }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        // Expect input chunks from both build and probe sides
        // Simplified: treat first N chunks as build side, rest as probe
        if input.len() < 2 {
            return Ok(input); // Not enough data — pass through
        }

        let mid = input.len() / 2;
        let build_chunks = &input[..mid];
        let probe_chunks = &input[mid..];

        let build_col = self.build_columns.first().copied().unwrap_or(0) as usize;
        let probe_col = self.probe_columns.first().copied().unwrap_or(0) as usize;

        // Build hash table from build side (Value-keyed, hash + equality)
        // We use a two-level structure: hash → vec of (actual_value, locations)
        // The actual Value is stored alongside to disambiguate hash collisions.
        type HashBucket = Vec<(Value, Vec<(usize, usize)>)>;
        let mut hash_table: HashMap<u64, HashBucket> = HashMap::new();

        for (ci, chunk) in build_chunks.iter().enumerate() {
            for row in 0..chunk.size {
                if let Some(field) = chunk.fields.get(build_col) {
                    let key = field.get_value(row).unwrap_or(Value::Null);
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
                let probe_key = chunk.fields.get(probe_col)
                    .and_then(|f| f.get_value(row))
                    .unwrap_or(Value::Null);
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
                            for col in 0..build_chunks[bci].num_fields() {
                                if let Some(field) = build_chunks[bci].fields.get(col) {
                                    let val = field.get_value(brow).unwrap_or(Value::Null);
                                    output_rows[col].push((val, field.is_null(brow)));
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
        for col in 0..output_rows.len() {
            let phys_type = output_types.get(col).copied().unwrap_or(PhysicalTypeID::Int64);
            let mut v = ValueVector::new(phys_type, num_rows);
            v.resize(num_rows);
            for (row, (val, _is_null)) in output_rows[col].iter().enumerate() {
                if matches!(val, Value::Null) {
                    v.set_null(row, true);
                } else {
                    store_value_in_vector(&mut v, row, val);
                }
            }
            result_fields.push(v);
        }
        Ok(vec![DataChunk::new(result_fields)])
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
        let first_type = items.first().map(|v| v.physical_type()).unwrap_or(PhysicalTypeID::Int64);

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
            let values: Vec<Value> = items.iter().map(|e| expr_to_value(e)).collect();
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
    pub table_catalog: Arc<Mutex<TableCatalog>>,
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
                if let Some(field) = chunk.fields.first() {
                    if let Some(val) = field.get_i64(row) {
                        // Evaluate the SET value expression against the current row
                        let set_val = evaluate_expression_for_row(&self.value, chunk, row);
                        rows_to_update.push((val as u64, set_val));
                    }
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
        let mut catalog = self.table_catalog.lock().unwrap();
        let updated = if let Some(table) = catalog.get_node_table_by_name_mut(&self.table_name) {
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
fn evaluate_expression_for_row(expr: &kuzu_parser::ast::Expression, chunk: &DataChunk, row: usize) -> kuzu_common::types::Value {
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
    pub table_catalog: Arc<Mutex<TableCatalog>>,
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
                if let Some(field) = chunk.fields.first() {
                    if let Some(val) = field.get_i64(row) {
                        rows_to_delete.push(val as u64);
                    }
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
        let mut catalog = self.table_catalog.lock().unwrap();
        let deleted = if let Some(table) = catalog.get_node_table_by_name_mut(&self.table_name) {
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
    pub table_catalog: Arc<Mutex<TableCatalog>>,
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
            "parquet" => {
                kuzu_storage::parquet_reader::read_parquet(path, &catalog_cols)
                    .map_err(|e| format!("Parquet read error: {e}"))?
            }
            _ => {
                return Err(format!(
                    "Unsupported file type: .{ext} (supported: .csv, .tsv, .parquet)"
                ));
            }
        };

        // 4. Insert rows into the table
        let mut catalog = self.table_catalog.lock().unwrap();
        let num_rows = rows.len();

        if let Some(table) = catalog.get_node_table_by_name_mut(&self.table_name) {
            for row in &rows {
                table
                    .insert_row(row.clone())
                    .map_err(|e| format!("Insert error: {e}"))?;
            }
            tracing::info!(
                "COPY FROM: inserted {num_rows} rows into node table '{}'",
                self.table_name
            );
        } else if let Some(table) = catalog.get_rel_table_by_name_mut(&self.table_name) {
            for row in &rows {
                if row.len() < 2 {
                    return Err(
                        "RelTable COPY FROM needs at least FROM and TO columns".into(),
                    );
                }
                let from = match &row[0] {
                    Value::Int64(v) => *v as u64,
                    _ => {
                        return Err(
                            "First column of rel table must be FROM node offset (Int64)".into(),
                        )
                    }
                };
                let to = match &row[1] {
                    Value::Int64(v) => *v as u64,
                    _ => {
                        return Err(
                            "Second column of rel table must be TO node offset (Int64)".into(),
                        )
                    }
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
            return Err(format!(
                "Table '{}' not found in storage catalog",
                self.table_name
            ));
        }

        // Return success chunk with row count
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
        v.resize(1);
        v.set_i64(0, num_rows as i64);
        Ok(vec![DataChunk::new(vec![v])])
    }
}