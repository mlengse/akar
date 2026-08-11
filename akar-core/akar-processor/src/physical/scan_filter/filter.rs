use crate::expression_evaluator::ExpressionEvaluator;
use crate::physical::common::value_cmp;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::processor::projection_helper::resolve_projection_column_index;
use akar_common::arrow_vector::VectorAccess;
use akar_common::error::ProcessorError;
use akar_common::selection::SelectionVector;
use akar_common::types::Value;
use akar_common::vector::DataChunk;
use akar_parser::ast::{BinaryOp, Constant, Expression, UnaryOp};
use arrow::array::Array;
use std::sync::{Arc, Mutex};

/// Build a SelectionVector from a BooleanArray, reading the packed bit buffer directly.
/// Avoids the intermediate Vec<bool> allocation used by the legacy path.
fn boolean_array_to_selection(bool_arr: &arrow::array::BooleanArray) -> SelectionVector {
    let len = bool_arr.len();
    let count = (0..len).filter(|&i| bool_arr.is_valid(i) && bool_arr.value(i)).count();
    let mut sel = SelectionVector::new(count);
    for i in 0..len {
        if bool_arr.is_valid(i) && bool_arr.value(i) {
            sel.push(i as u32);
        }
    }
    sel
}

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
    /// Uses the Arrow-native evaluator path when available (avoids Value enum boxing).
    pub fn evaluate_expression(
        expr: &Expression,
        chunk: &DataChunk,
        evaluator: Option<&ExpressionEvaluator>,
    ) -> Result<Vec<bool>, ProcessorError> {
        if let Some(eval) = evaluator {
            let arrow_result = eval.evaluate_to_arrow(expr, chunk)?;
            let size = arrow_result.size();
            if arrow_result.physical_type == akar_common::types::PhysicalTypeID::Bool {
                if let Some(bool_arr) = arrow_result.array.as_any().downcast_ref::<arrow::array::BooleanArray>() {
                    let mut mask = Vec::with_capacity(size);
                    for i in 0..size {
                        if bool_arr.is_valid(i) {
                            mask.push(bool_arr.value(i));
                        } else {
                            mask.push(false);
                        }
                    }
                    return Ok(mask);
                }
            }
            // Fallback: if the result isn't a BooleanArray, treat non-null as truthy
            let mut mask = Vec::with_capacity(size);
            for i in 0..size {
                mask.push(!arrow_result.is_null(i));
            }
            return Ok(mask);
        }

        Self::evaluate_expression_legacy(expr, chunk)
    }

    /// Build a SelectionVector from a boolean mask.
    pub fn mask_to_selection(mask: &[bool]) -> SelectionVector {
        let count = mask.iter().filter(|&v| *v).count();
        let mut sel = SelectionVector::new(count);
        for (i, &keep) in mask.iter().enumerate() {
            if keep {
                sel.push(i as u32);
            }
        }
        sel
    }

    /// Evaluate a filter expression and return a SelectionVector directly,
    /// using the Arrow-native path. This is the zero-allocation hot path.
    pub fn evaluate_to_selection(
        expr: &Expression,
        chunk: &DataChunk,
        evaluator: Option<&ExpressionEvaluator>,
    ) -> Result<SelectionVector, ProcessorError> {
        if let Some(eval) = evaluator {
            let arrow_result = eval.evaluate_to_arrow(expr, chunk)?;
            if arrow_result.physical_type == akar_common::types::PhysicalTypeID::Bool {
                if let Some(bool_arr) = arrow_result.array.as_any().downcast_ref::<arrow::array::BooleanArray>() {
                    return Ok(boolean_array_to_selection(bool_arr));
                }
            }
            // Fallback: all rows pass (non-boolean result type)
            return Ok(SelectionVector::from_range(chunk.size));
        }
        // No evaluator — fall back to legacy mask path
        let mask = Self::evaluate_expression_legacy(expr, chunk)?;
        Ok(Self::mask_to_selection(&mask))
    }

    fn evaluate_expression_legacy(expr: &Expression, chunk: &DataChunk) -> Result<Vec<bool>, ProcessorError> {
        let size = chunk.size;
        let mut mask = Vec::with_capacity(size);
        for i in 0..size {
            mask.push(Self::legacy_row_truthy(expr, chunk, i));
        }
        Ok(mask)
    }

    /// Evaluate `expr` for a single row into an optional Value (None = NULL)
    /// using only chunk data — the fallback path when no ExpressionEvaluator
    /// is available. Unlike the old mask-based recursion, NULL semantics and
    /// numeric comparisons are handled correctly (P52.43).
    fn legacy_row_value(expr: &Expression, chunk: &DataChunk, row: usize) -> Option<Value> {
        match expr {
            Expression::BinaryOp(op, left, right) => {
                let l = Self::legacy_row_value(left, chunk, row);
                let r = Self::legacy_row_value(right, chunk, row);
                match op {
                    BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => legacy_logic(op, &l, &r),
                    _ => legacy_compare(op, &l, &r),
                }
            }
            Expression::UnaryOp(op, inner) => {
                let v = Self::legacy_row_value(inner, chunk, row);
                match op {
                    UnaryOp::Not => v.map(|val| match val {
                        Value::Bool(b) => Value::Bool(!b),
                        _ => Value::Null,
                    }),
                    UnaryOp::Negate => v.map(|val| match val {
                        Value::Int64(i) => Value::Int64(i.wrapping_neg()),
                        Value::Int32(i) => Value::Int64(-(i as i64)),
                        Value::Int16(i) => Value::Int64(-(i as i64)),
                        Value::Int8(i) => Value::Int64(-(i as i64)),
                        Value::UInt64(i) => Value::Int64(-(i as i64)),
                        Value::UInt32(i) => Value::Int64(-(i as i64)),
                        Value::Double(d) => Value::Double(-d),
                        Value::Float(f) => Value::Double(-(f as f64)),
                        _ => Value::Null,
                    }),
                    UnaryOp::IsNull => Some(Value::Bool(v.is_none())),
                    UnaryOp::IsNotNull => Some(Value::Bool(v.is_some())),
                }
            }
            Expression::Variable(_) | Expression::PropertyAccess(_, _) => {
                if let Some(col) = resolve_projection_column_index(expr, chunk) {
                    chunk.get_value(col, row)
                } else if !chunk.fields.is_empty() {
                    // Unresolved variable: fall back to the first column (legacy
                    // behavior) so bare-variable predicates keep working.
                    chunk.get_value(0, row)
                } else {
                    None
                }
            }
            Expression::Constant(c) => Some(match c {
                Constant::Bool(b) => Value::Bool(*b),
                Constant::Integer(i) => Value::Int64(*i),
                Constant::Float(f) => Value::Double(*f),
                Constant::String(s) => Value::String(s.clone()),
                Constant::Null => Value::Null,
            }),
            _ => Some(Value::Null),
        }
    }

    fn legacy_row_truthy(expr: &Expression, chunk: &DataChunk, row: usize) -> bool {
        match Self::legacy_row_value(expr, chunk, row) {
            None => false,
            Some(v) => legacy_truthy(&v),
        }
    }
}

/// SQL truthiness for a non-null Value.
fn legacy_truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Int64(i) => *i != 0,
        Value::Int32(i) => *i != 0,
        Value::Int16(i) => *i != 0,
        Value::Int8(i) => *i != 0,
        Value::UInt64(i) => *i != 0,
        Value::UInt32(i) => *i != 0,
        Value::UInt16(i) => *i != 0,
        Value::UInt8(i) => *i != 0,
        Value::Double(d) => *d != 0.0,
        Value::Float(f) => *f != 0.0,
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// Three-valued AND/OR/XOR for the legacy filter path.
fn legacy_logic(op: &BinaryOp, l: &Option<Value>, r: &Option<Value>) -> Option<Value> {
    let lb = match l {
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => Some(true),
        None => None,
    };
    let rb = match r {
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => Some(true),
        None => None,
    };
    let res = match op {
        BinaryOp::And => match (lb, rb) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        },
        BinaryOp::Or => match (lb, rb) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        },
        BinaryOp::Xor => match (lb, rb) {
            (Some(a), Some(b)) => Some(a ^ b),
            _ => None,
        },
        _ => None,
    };
    res.map(Value::Bool)
}

/// NULL-aware comparison for the legacy filter path.
fn legacy_compare(op: &BinaryOp, l: &Option<Value>, r: &Option<Value>) -> Option<Value> {
    let (Some(a), Some(b)) = (l, r) else {
        return None;
    };
    let ord = value_cmp(a, b);
    let res = match op {
        BinaryOp::Equal => ord == std::cmp::Ordering::Equal,
        BinaryOp::NotEqual => ord != std::cmp::Ordering::Equal,
        BinaryOp::GreaterThan => ord == std::cmp::Ordering::Greater,
        BinaryOp::GreaterThanOrEqual => ord != std::cmp::Ordering::Less,
        BinaryOp::LessThan => ord == std::cmp::Ordering::Less,
        BinaryOp::LessThanOrEqual => ord != std::cmp::Ordering::Greater,
        _ => true,
    };
    Some(Value::Bool(res))
}

impl PhysicalOperatorExec for PhysicalFilter {
    fn operator_type(&self) -> &str {
        "filter"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let evaluator = self.evaluator.as_ref().and_then(|e| e.lock().ok());

        let mut output = Vec::new();
        for mut chunk in input {
            let mask = Self::evaluate_expression(&self.expression, &chunk, evaluator.as_deref())?;
            let sel = Self::mask_to_selection(&mask);

            if sel.is_empty() {
                // Preserve the schema on empty results: emit a zero-row chunk
                // with the same fields/types/names so downstream operators
                // (e.g. optional-match merge) still see the column layout.
                chunk.resize(0);
                chunk.sel_vector = None;
                output.push(chunk);
                continue;
            }

            if sel.size == chunk.size {
                // All rows passed — no selection needed
                output.push(chunk);
            } else {
                // Materialize the selection to shrink the chunk
                chunk.sel_vector = Some(sel);
                chunk.materialize();
                output.push(chunk);
            }
        }
        Ok(output)
    }
}
