use kuzu_common::selection::SelectionVector;
use kuzu_common::types::Value;
use kuzu_common::vector::DataChunk;
use kuzu_parser::ast::{BinaryOp, Constant, Expression, UnaryOp};
use std::sync::{Arc, Mutex};
use crate::expression_evaluator::ExpressionEvaluator;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};

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
    pub fn evaluate_expression(
        expr: &Expression,
        chunk: &DataChunk,
        evaluator: Option<&ExpressionEvaluator>,
    ) -> Result<Vec<bool>, String> {
        if let Some(eval) = evaluator {
            let result_vec = eval.evaluate(expr, chunk)?;
            let size = result_vec.size();
            let mut mask = Vec::with_capacity(size);
            for i in 0..size {
                let val = result_vec.get_value(i);
                match val {
                    Some(Value::Bool(b)) => mask.push(b),
                    Some(Value::Null) => mask.push(false),
                    Some(_) => mask.push(true),
                    None => mask.push(false),
                }
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
                    UnaryOp::Negate => Ok(vals.iter().map(|v| !v).collect()),
                    UnaryOp::IsNull => Ok(vec![false; chunk.size]),
                    UnaryOp::IsNotNull => Ok(vals),
                }
            }
            Expression::Variable(_name) => {
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
        for mut chunk in input {
            let mask = Self::evaluate_expression(&self.expression, &chunk, evaluator.as_deref())?;
            let sel = Self::mask_to_selection(&mask);

            if sel.is_empty() {
                continue;
            }

            if sel.size == chunk.size {
                // All rows passed — no selection needed
                output.push(chunk);
            } else {
                // Zero-copy: attach selection vector
                chunk.sel_vector = Some(sel);
                output.push(chunk);
            }
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
            _ => true,
        })
        .collect();
    Ok(result)
}
