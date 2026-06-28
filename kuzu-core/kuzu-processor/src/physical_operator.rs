//! Physical operator types and execution logic for query processing.
//!
//! Each physical operator implements the `Operator` trait with an `execute` method
//! that produces output `DataChunk`s from input `DataChunk`s.

use kuzu_common::types::{PhysicalTypeID, Value};
use kuzu_common::vector::{physical_type_size, DataChunk, ValueVector};
use kuzu_function::scalar::AggValueState;
use kuzu_function::AggregateFunction;
use kuzu_parser::ast::{BinaryOp, Constant, Expression, UnaryOp};
use std::collections::HashMap;
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
}

impl PhysicalOperatorExec for PhysicalScan {
    fn operator_type(&self) -> &str { "scan" }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        let mut fields = Vec::new();
        for &col_id in &self.column_ids {
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1000);
            for i in 0..100.min(self.estimated_cardinality as usize) {
                v.set_i64(i, (col_id as i64) * 1000 + i as i64);
            }
            v.resize(100.min(self.estimated_cardinality as usize));
            fields.push(v);
        }
        if fields.is_empty() {
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1000);
            for i in 0..100.min(self.estimated_cardinality as usize) {
                v.set_i64(i, i as i64);
            }
            v.resize(100.min(self.estimated_cardinality as usize));
            fields.push(v);
        }
        let chunk = DataChunk::new(fields);
        Ok(vec![chunk])
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

        // Build hash table from build side
        let mut hash_table: HashMap<i64, Vec<(usize, usize)>> = HashMap::new(); // key → (chunk_idx, row)
        for (ci, chunk) in build_chunks.iter().enumerate() {
            for row in 0..chunk.size {
                if let Some(key) = chunk.fields.get(build_col)
                    .and_then(|f| f.get_i64(row))
                {
                    hash_table.entry(key).or_default().push((ci, row));
                }
            }
        }

        // Probe and output matching rows
        let mut output_fields: Vec<Vec<(i64, bool)>> = Vec::new();
        let mut built_cols = false;

        for chunk in probe_chunks {
            for row in 0..chunk.size {
                let key = chunk.fields.get(probe_col)
                    .and_then(|f| f.get_i64(row))
                    .unwrap_or(0);

                if let Some(matches) = hash_table.get(&key) {
                    if !built_cols {
                        let num_cols = build_chunks[0].num_fields() + chunk.num_fields();
                        output_fields = (0..num_cols).map(|_| Vec::new()).collect();
                        built_cols = true;
                    }

                    for &(bci, brow) in matches {
                        // Build side columns
                        for col in 0..build_chunks[bci].num_fields() {
                            if let Some(field) = build_chunks[bci].fields.get(col) {
                                let val = field.get_i64(brow).unwrap_or(0);
                                let is_null = field.is_null(brow);
                                output_fields[col].push((val, is_null));
                            }
                        }
                        // Probe side columns
                        let offset = build_chunks[0].num_fields();
                        for col in 0..chunk.num_fields() {
                            if let Some(field) = chunk.fields.get(col) {
                                let val = field.get_i64(row).unwrap_or(0);
                                let is_null = field.is_null(row);
                                output_fields[offset + col].push((val, is_null));
                            }
                        }
                    }
                }
            }
        }

        if !built_cols {
            return Ok(Vec::new());
        }

        let num_rows = output_fields[0].len();
        let mut result_fields = Vec::new();
        for col in 0..output_fields.len() {
            let mut v = ValueVector::new(PhysicalTypeID::Int64, num_rows);
            for (row, (val, is_null)) in output_fields[col].iter().enumerate() {
                v.set_i64(row, *val);
                if *is_null { v.set_null(row, true); }
            }
            v.resize(num_rows);
            result_fields.push(v);
        }
        Ok(vec![DataChunk::new(result_fields)])
    }
}