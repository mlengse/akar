//! Auto-extracted from physical_operator.rs
use crate::physical::common::store_value_in_vector;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use akar_common::types::{PhysicalTypeID, Value};
use akar_common::vector::{DataChunk, ValueVector};

// ==================== Unwind ====================

/// Physical operator for UNWIND — expands a list expression into rows.
pub struct PhysicalUnwind {
    pub expression: akar_parser::ast::Expression,
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
            akar_common::types::Value::List(items) => items.clone(),
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
                for (col_idx, _field) in chunk.fields.iter().enumerate() {
                    let val = chunk.get_value(col_idx, row).unwrap_or(Value::Null);
                    let mut v = ValueVector::new(chunk.field_types[col_idx], items.len());
                    v.resize(items.len());
                    for i in 0..items.len() {
                        store_value_in_vector(&mut v, i, &val)?;
                    }
                    chunk_fields.push(v);
                }
                // Add unwound vector
                let mut uw_v = ValueVector::new(first_type, items.len());
                uw_v.resize(items.len());
                for (i, item) in items.iter().enumerate() {
                    store_value_in_vector(&mut uw_v, i, item)?;
                }
                chunk_fields.push(uw_v);
                let arrow_fields = chunk_fields
                    .iter()
                    .map(|v| akar_common::arrow_vector::ArrowVector::from_legacy(v).array)
                    .collect::<Vec<_>>();
                let arrow_field_types = chunk_fields.iter().map(|v| v.physical_type()).collect::<Vec<_>>();
                result_chunks.push(DataChunk::new(arrow_fields, arrow_field_types));
            }
        } else {
            // No input — just the unwound vector
            let mut uw_v = ValueVector::new(first_type, items.len());
            uw_v.resize(items.len());
            for (i, item) in items.iter().enumerate() {
                store_value_in_vector(&mut uw_v, i, item)?;
            }
            let arr = akar_common::arrow_vector::ArrowVector::from_legacy(&uw_v).array;
            result_chunks.push(DataChunk::new(vec![arr], vec![uw_v.physical_type()]));
        }

        Ok(result_chunks)
    }
}

/// Evaluate an UNWIND expression to get the list value.
fn evaluate_unwind_expr(expr: &akar_parser::ast::Expression) -> Value {
    match expr {
        akar_parser::ast::Expression::List(items) => {
            let values: Vec<Value> = items.iter().map(expr_to_value).collect();
            Value::List(values)
        }
        _ => Value::List(Vec::new()),
    }
}

/// Convert an AST expression to a runtime Value (for simple constants).
fn expr_to_value(expr: &akar_parser::ast::Expression) -> Value {
    match expr {
        akar_parser::ast::Expression::Constant(c) => match c {
            akar_parser::ast::Constant::Null => Value::Null,
            akar_parser::ast::Constant::Bool(b) => Value::Bool(*b),
            akar_parser::ast::Constant::Integer(i) => Value::Int64(*i),
            akar_parser::ast::Constant::Float(f) => Value::Double(*f),
            akar_parser::ast::Constant::String(s) => Value::String(s.clone()),
        },
        _ => Value::Null,
    }
}
