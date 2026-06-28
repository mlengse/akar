//! Physical operator types and execution logic for query processing.
//!
//! Each physical operator implements the `Operator` trait with an `execute` method
//! that produces output `DataChunk`s from input `DataChunk`s.

use kuzu_common::types::PhysicalTypeID;
use kuzu_common::vector::{physical_type_size, DataChunk, ValueVector};
use kuzu_parser::ast::{BinaryOp, Constant, Expression, UnaryOp};
use std::collections::HashMap;

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
}

impl PhysicalFilter {
    /// Evaluate a filter expression against values and return a boolean mask.
    pub fn evaluate_expression(expr: &Expression, chunk: &DataChunk) -> Result<Vec<bool>, String> {
        match expr {
            Expression::BinaryOp(op, left, right) => {
                let left_vals = Self::evaluate_expression(left, chunk)?;
                let right_vals = Self::evaluate_expression(right, chunk)?;
                evaluate_binary_op(op, &left_vals, &right_vals, chunk.size)
            }
            Expression::UnaryOp(op, inner) => {
                let vals = Self::evaluate_expression(inner, chunk)?;
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
                Self::evaluate_expression(obj, chunk)
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
        let mut output = Vec::new();
        for chunk in input {
            let mask = Self::evaluate_expression(&self.expression, &chunk)?;
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

fn evaluate_binary_op(
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

#[derive(Debug, Clone)]
pub struct Aggregator {
    pub function_name: String,
    pub column_index: usize,
}

/// State for a single aggregate computation.
#[derive(Debug, Clone)]
enum AggState {
    Count(u64),
    Sum(i64),
    Min(i64),
    Max(i64),
    Avg { sum: i64, count: u64 },
}

pub struct PhysicalAggregate {
    pub group_by_cols: Vec<u32>,
    pub aggregate_functions: Vec<String>,
}

impl PhysicalOperatorExec for PhysicalAggregate {
    fn operator_type(&self) -> &str { "aggregate" }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        // Simplified: if no group by, compute scalar aggregates
        // If group by exists, hash-based grouping

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
            // Scalar aggregation (no GROUP BY)
            self.compute_scalar_aggregates(&input)
        } else {
            // Hash-based GROUP BY
            self.compute_grouped_aggregates(&input)
        }
    }
}

impl PhysicalAggregate {
    /// Compute scalar aggregates (no GROUP BY) across all input chunks.
    fn compute_scalar_aggregates(&self, input: &[DataChunk]) -> OperatorResult {
        let mut states: Vec<AggState> = self.aggregate_functions.iter()
            .map(|name| match name.to_uppercase().as_str() {
                "COUNT" | "COUNT(*)" => AggState::Count(0),
                "SUM" => AggState::Sum(0),
                "MIN" => AggState::Min(i64::MAX),
                "MAX" => AggState::Max(i64::MIN),
                "AVG" => AggState::Avg { sum: 0, count: 0 },
                _ => AggState::Count(0),
            })
            .collect();

        for chunk in input {
            for row in 0..chunk.size {
                for (i, state) in states.iter_mut().enumerate() {
                    let col_idx = i.min(chunk.fields.len().saturating_sub(1));
                    let val = chunk.fields.get(col_idx)
                        .and_then(|f| f.get_i64(row))
                        .unwrap_or(0);
                    match state {
                        AggState::Count(n) => *n += 1,
                        AggState::Sum(s) => *s += val,
                        AggState::Min(m) => *m = (*m).min(val),
                        AggState::Max(m) => *m = (*m).max(val),
                        AggState::Avg { sum, count } => { *sum += val; *count += 1; }
                    }
                }
            }
        }

        let mut fields = Vec::new();
        for state in &states {
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
            let result = match state {
                AggState::Count(n) => *n as i64,
                AggState::Sum(s) => *s,
                AggState::Min(m) => *m,
                AggState::Max(m) => *m,
                AggState::Avg { sum, count } => {
                    if *count > 0 { *sum / *count as i64 } else { 0 }
                }
            };
            v.set_i64(0, result);
            v.resize(1);
            fields.push(v);
        }
        Ok(vec![DataChunk::new(fields)])
    }

    /// Compute hash-based GROUP BY aggregates.
    fn compute_grouped_aggregates(&self, input: &[DataChunk]) -> OperatorResult {
        // Simplified: group by first column, aggregate the rest
        let group_col = self.group_by_cols.first().copied().unwrap_or(0) as usize;

        let mut groups: HashMap<i64, Vec<AggState>> = HashMap::new();

        for chunk in input {
            for row in 0..chunk.size {
                let key = chunk.fields.get(group_col)
                    .and_then(|f| f.get_i64(row))
                    .unwrap_or(0);

                let entry = groups.entry(key).or_insert_with(|| {
                    self.aggregate_functions.iter()
                        .map(|name| match name.to_uppercase().as_str() {
                            "COUNT" | "COUNT(*)" => AggState::Count(0),
                            "SUM" => AggState::Sum(0),
                            "MIN" => AggState::Min(i64::MAX),
                            "MAX" => AggState::Max(i64::MIN),
                            "AVG" => AggState::Avg { sum: 0, count: 0 },
                            _ => AggState::Count(0),
                        })
                        .collect()
                });

                for (i, state) in entry.iter_mut().enumerate() {
                    let val = chunk.fields.get(i)
                        .and_then(|f| f.get_i64(row))
                        .unwrap_or(0);
                    match state {
                        AggState::Count(n) => *n += 1,
                        AggState::Sum(s) => *s += val,
                        AggState::Min(m) => *m = (*m).min(val),
                        AggState::Max(m) => *m = (*m).max(val),
                        AggState::Avg { sum, count } => { *sum += val; *count += 1; }
                    }
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

        // Build output chunks
        let mut fields: Vec<Vec<(i64, bool)>> = (0..=self.aggregate_functions.len())
            .map(|_| Vec::new())
            .collect();

        // Group key column
        for (key, states) in &groups {
            fields[0].push((*key, false));
            for (i, state) in states.iter().enumerate() {
                let val = match state {
                    AggState::Count(n) => *n as i64,
                    AggState::Sum(s) => *s,
                    AggState::Min(m) => *m,
                    AggState::Max(m) => *m,
                    AggState::Avg { sum, count } => {
                        if *count > 0 { *sum / *count as i64 } else { 0 }
                    }
                };
                fields[i + 1].push((val, false));
            }
        }

        let num_rows = groups.len();
        let mut output_fields = Vec::new();
        for col in 0..=self.aggregate_functions.len() {
            let mut v = ValueVector::new(PhysicalTypeID::Int64, num_rows);
            for (row, (val, _)) in fields[col].iter().enumerate() {
                v.set_i64(row, *val);
            }
            v.resize(num_rows);
            output_fields.push(v);
        }
        Ok(vec![DataChunk::new(output_fields)])
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