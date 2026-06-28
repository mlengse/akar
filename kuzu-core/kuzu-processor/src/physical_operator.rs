//! Physical operator types and execution logic for query processing.
//!
//! Each physical operator implements the `Operator` trait with an `execute` method
//! that produces output `DataChunk`s from input `DataChunk`s.

use kuzu_common::types::PhysicalTypeID;
use kuzu_common::vector::{physical_type_size, DataChunk, ValueVector};
use kuzu_parser::ast::{BinaryOp, Constant, Expression, UnaryOp};

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
        // Create output vector with the requested columns
        let mut fields = Vec::new();
        for &col_id in &self.column_ids {
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1000);
            // Fill with sequential IDs as placeholder data
            for i in 0..100.min(self.estimated_cardinality as usize) {
                v.set_i64(i, (col_id as i64) * 1000 + i as i64);
            }
            v.resize(100.min(self.estimated_cardinality as usize));
            fields.push(v);
        }
        if fields.is_empty() {
            // Default: output a single int64 column
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
            Expression::FunctionCall(_, _) | Expression::List(_) | Expression::Map(_) => {
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
        // Simplified: just pass through (full sort requires multi-chunk merge)
        Ok(input)
    }
}

// ==================== Aggregate ====================

pub struct PhysicalAggregate {
    pub group_by_cols: Vec<u32>,
    pub aggregate_functions: Vec<String>,
}

impl PhysicalOperatorExec for PhysicalAggregate {
    fn operator_type(&self) -> &str { "aggregate" }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        // Simplified: return a single chunk with one row (empty aggregate)
        let mut fields = Vec::new();
        for _ in 0..self.aggregate_functions.len() {
            let mut v = ValueVector::new(PhysicalTypeID::Int64, 1);
            v.set_i64(0, 0);
            v.resize(1);
            fields.push(v);
        }
        Ok(vec![DataChunk::new(fields)])
    }
}

// ==================== HashJoin ====================

pub struct PhysicalHashJoin {
    pub build_columns: Vec<u32>,
    pub probe_columns: Vec<u32>,
}

impl PhysicalOperatorExec for PhysicalHashJoin {
    fn operator_type(&self) -> &str { "hash_join" }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        // TODO: implement actual hash join
        Ok(vec![])
    }
}