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

        // Build the unwound variable column once. `build_arrow_from_values`
        // preserves complex values (map/struct/list) that
        // `store_value_in_vector` would drop to NULL (P53.26).
        let first_type = items
            .first()
            .map(|v| v.physical_type())
            .unwrap_or(PhysicalTypeID::Int64);
        let uw_arrow = crate::expression_evaluator::build_arrow_from_values(&items, first_type, items.len())
            .map_err(|e| e.to_string())?;

        let mut result_chunks = Vec::new();
        // If we have input data, repeat for each input row
        if let Some(chunk) = input.first() {
            for row in 0..chunk.size {
                let mut arrow_fields: Vec<arrow::array::ArrayRef> = Vec::with_capacity(chunk.fields.len() + 1);
                let mut arrow_field_types: Vec<PhysicalTypeID> = Vec::with_capacity(chunk.fields.len() + 1);
                for (col_idx, _field) in chunk.fields.iter().enumerate() {
                    let val = chunk.get_value(col_idx, row).unwrap_or(Value::Null);
                    let mut v = ValueVector::new(chunk.field_types[col_idx], items.len());
                    v.resize(items.len());
                    for i in 0..items.len() {
                        store_value_in_vector(&mut v, i, &val)?;
                    }
                    arrow_fields.push(akar_common::arrow_vector::ArrowVector::from_legacy(&v).array);
                    arrow_field_types.push(v.physical_type());
                }
                // Add unwound vector
                arrow_fields.push(uw_arrow.array.clone());
                arrow_field_types.push(uw_arrow.physical_type);
                let mut named_chunk = DataChunk::new(arrow_fields, arrow_field_types);
                named_chunk.field_names = chunk
                    .field_names
                    .iter()
                    .cloned()
                    .chain([self.variable.clone()])
                    .collect();
                result_chunks.push(named_chunk);
            }
        } else {
            // No input — just the unwound vector
            let mut named_chunk = DataChunk::new(vec![uw_arrow.array.clone()], vec![uw_arrow.physical_type]);
            named_chunk.field_names = vec![self.variable.clone()];
            result_chunks.push(named_chunk);
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

/// Convert an AST expression to a runtime Value (for literal constants and
/// nested list/map literals).
fn expr_to_value(expr: &akar_parser::ast::Expression) -> Value {
    match expr {
        akar_parser::ast::Expression::Constant(c) => match c {
            akar_parser::ast::Constant::Null => Value::Null,
            akar_parser::ast::Constant::Bool(b) => Value::Bool(*b),
            akar_parser::ast::Constant::Integer(i) => Value::Int64(*i),
            akar_parser::ast::Constant::Float(f) => Value::Double(*f),
            akar_parser::ast::Constant::String(s) => Value::String(s.clone()),
        },
        akar_parser::ast::Expression::List(items) => Value::List(items.iter().map(expr_to_value).collect()),
        akar_parser::ast::Expression::Map(items) => Value::Map(
            items
                .iter()
                .map(|(k, v)| (Value::String(k.clone()), expr_to_value(v)))
                .collect(),
        ),
        _ => Value::Null,
    }
}
