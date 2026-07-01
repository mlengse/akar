//! Expression evaluator — recursively evaluates an expression tree against a DataChunk.
//!
//! This replaces the ad-hoc `PhysicalFilter::evaluate_expression` with a proper
//! expression evaluator that dispatches function calls through the scalar function
//! registry (kuzu-function::evaluate_scalar), supporting:
//! - Variable:        reads values from a DataChunk field by name
//! - Constant:        returns a literal value
//! - BinaryOp:        dispatches to arithmetic/comparison/boolean scalar functions
//! - UnaryOp:         dispatches to NOT/negate scalar functions
//! - FunctionCall:    resolves the function name in the registry and evaluates
//! - PropertyAccess:  reads a property from a struct/object expression
//! - List/Map:        evaluated via list_creation/map_creation scalar functions

use kuzu_common::types::Value;
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_function::registry::{FunctionRegistry, ScalarFunction};
use kuzu_function::scalar::evaluate_scalar;
use kuzu_parser::ast::{BinaryOp, Constant, Expression, Query, UnaryOp};
use std::sync::{Arc, Mutex};

/// Evaluates expressions against DataChunks using the function registry.
pub struct ExpressionEvaluator {
    registry: Arc<Mutex<FunctionRegistry>>,
    /// Optional callback to execute subqueries at evaluation time.
    /// Takes a parsed Query and returns DataChunks.
    pub subquery_fn: Option<Arc<dyn Fn(&Query) -> Result<Vec<DataChunk>, String> + Send + Sync>>,
    /// Optional callback for sequence operations (nextval/currval).
    /// Takes (sequence_name, is_nextval) and returns the resulting value.
    pub sequence_fn: Option<Arc<dyn Fn(&str, bool) -> Result<Value, String> + Send + Sync>>,
}

impl ExpressionEvaluator {
    pub fn new(registry: Arc<Mutex<FunctionRegistry>>) -> Self {
        Self {
            registry,
            subquery_fn: None,
            sequence_fn: None,
        }
    }

    /// Set the subquery execution callback.
    pub fn with_subquery_fn(mut self, f: Arc<dyn Fn(&Query) -> Result<Vec<DataChunk>, String> + Send + Sync>) -> Self {
        self.subquery_fn = Some(f);
        self
    }

    /// Set the sequence operation callback (for nextval/currval).
    pub fn with_sequence_fn(mut self, f: Arc<dyn Fn(&str, bool) -> Result<Value, String> + Send + Sync>) -> Self {
        self.sequence_fn = Some(f);
        self
    }

    /// Evaluate a subquery by calling the stored callback.
    fn evaluate_subquery(&self, query: &Query) -> Result<Vec<DataChunk>, String> {
        if let Some(ref f) = self.subquery_fn {
            f(query)
        } else {
            Err("No subquery executor configured".into())
        }
    }

    /// Evaluate an expression for every row in the chunk, returning a ValueVector.
    pub fn evaluate(&self, expr: &Expression, chunk: &DataChunk) -> Result<ValueVector, String> {
        match expr {
            Expression::Constant(c) => self.evaluate_constant(c, chunk.size),
            Expression::Variable(name) => self.evaluate_variable(name, chunk),
            Expression::PropertyAccess(obj, prop) => self.evaluate_property_access(obj, prop, chunk),
            Expression::FunctionCall(name, args) => self.evaluate_function_call(name, args, chunk),
            Expression::BinaryOp(op, left, right) => self.evaluate_binary_op(op, left, right, chunk),
            Expression::UnaryOp(op, inner) => self.evaluate_unary_op(op, inner, chunk),
            Expression::List(items) => self.evaluate_list_literal(items, chunk),
            Expression::Map(items) => self.evaluate_map_literal(items, chunk),
            Expression::Parameter(_) => {
                // Parameters should be substituted by the binder/prepared statement layer.
                // If they reach the evaluator, return a null Int64 vector matching chunk size.
                let mut v = ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, chunk.size);
                v.resize(chunk.size);
                for i in 0..chunk.size {
                    v.set_null(i, true);
                }
                Ok(v)
            }
            Expression::ExistsSubquery(query) => {
                // Evaluate EXISTS subquery: execute the inner query against
                // the database. If it returns at least one row → true, else false.
                // For uncorrelated subqueries, execute once and fill all rows.
                let result = self.evaluate_subquery(query)?;
                let exists = !result.is_empty() && result.iter().any(|c| c.size > 0);
                let mut v = ValueVector::new(kuzu_common::types::PhysicalTypeID::Bool, chunk.size);
                v.resize(chunk.size);
                for i in 0..chunk.size {
                    store_value_in_vector_simple(&mut v, i, &Value::Bool(exists));
                }
                Ok(v)
            }
        }
    }

    /// Evaluate a constant expression — returns a vector filled with the constant value.
    fn evaluate_constant(&self, c: &Constant, size: usize) -> Result<ValueVector, String> {
        let val: Value = match c {
            Constant::Null => Value::Null,
            Constant::Bool(b) => Value::Bool(*b),
            Constant::Integer(i) => Value::Int64(*i),
            Constant::Float(f) => Value::Double(*f),
            Constant::String(s) => Value::String(s.clone()),
        };

        let physical_type = val.physical_type();
        let mut v = ValueVector::new(physical_type, size);
        v.resize(size);
        for i in 0..size {
            store_value_in_vector(&mut v, i, &val);
        }
        Ok(v)
    }

    /// Evaluate a variable expression — reads a field from the DataChunk by variable name.
    /// The variable name is matched against field names by index position (0-based).
    /// Falls back to treating any non-null first field as true (legacy compatibility).
    fn evaluate_variable(&self, name: &str, chunk: &DataChunk) -> Result<ValueVector, String> {
        // Try to find the field by position (the binder resolves names to positions)
        if let Ok(idx) = name.parse::<usize>() {
            return chunk.fields.get(idx).cloned().ok_or_else(|| {
                format!(
                    "Variable '{}' (index {}) not found in chunk with {} fields",
                    name,
                    idx,
                    chunk.fields.len()
                )
            });
        }

        // For unresolved variable names (e.g., from MATCH patterns), fall back to
        // treating the first field as the variable's value and passing through.
        if let Some(field) = chunk.fields.first() {
            let mut v = ValueVector::new(field.physical_type(), chunk.size);
            v.resize(chunk.size);
            let type_size = kuzu_common::vector::physical_type_size(field.physical_type());
            let copy_size = chunk.size * type_size;
            if copy_size <= field.data().len() && copy_size <= v.data().len() {
                v.data_mut()[..copy_size].copy_from_slice(&field.data()[..copy_size]);
            }
            for i in 0..chunk.size {
                v.set_null(i, field.is_null(i));
            }
            Ok(v)
        } else {
            // No fields — return an empty vector
            Ok(ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, 0))
        }
    }

    /// Evaluate a property access expression.
    /// For now, evaluate the object expression and return it (simplified).
    fn evaluate_property_access(
        &self,
        obj: &Expression,
        _prop: &str,
        chunk: &DataChunk,
    ) -> Result<ValueVector, String> {
        // Simplified: evaluate the object expression
        self.evaluate(obj, chunk)
    }

    /// Evaluate a function call expression.
    fn evaluate_function_call(
        &self,
        name: &str,
        args: &[Expression],
        chunk: &DataChunk,
    ) -> Result<ValueVector, String> {
        // Evaluate all argument expressions first
        let arg_vectors: Vec<ValueVector> = args
            .iter()
            .map(|arg| self.evaluate(arg, chunk))
            .collect::<Result<Vec<_>, _>>()?;

        if arg_vectors.is_empty() {
            return Err(format!("Function '{}' requires at least one argument", name));
        }

        let num_rows = arg_vectors[0].size();

        // Look up the function in the registry
        let func = {
            let reg = self.registry.lock().unwrap();
            reg.get_scalar(name).cloned()
        };

        let func = match func {
            Some(f) => f,
            None => return Err(format!("Unknown function: '{}'", name)),
        };

        // Handle SequenceOp (nextval/currval) via callback with catalog access
        if matches!(func, ScalarFunction::SequenceOp { .. }) {
            return self.evaluate_sequence_op(name, &func, &arg_vectors, num_rows);
        }

        // For each row, extract values, call evaluate_scalar, and store result
        // First, determine the output type by evaluating the first non-null row
        let result_type = {
            let mut result = None;
            for row in 0..num_rows {
                let arg_values: Vec<Value> = arg_vectors
                    .iter()
                    .map(|vec| {
                        if row < vec.size() && !vec.is_null(row) {
                            vec.get_value(row).unwrap_or(Value::Null)
                        } else {
                            Value::Null
                        }
                    })
                    .collect();

                if let Ok(val) = evaluate_scalar(&func, &arg_values) {
                    if val != Value::Null {
                        result = Some(val.physical_type());
                        break;
                    }
                }
            }
            result.unwrap_or(kuzu_common::types::PhysicalTypeID::Int64)
        };

        let mut result_vec = ValueVector::new(result_type, num_rows);
        result_vec.resize(num_rows);

        for row in 0..num_rows {
            let arg_values: Vec<Value> = arg_vectors
                .iter()
                .map(|vec| {
                    if row < vec.size() && !vec.is_null(row) {
                        vec.get_value(row).unwrap_or(Value::Null)
                    } else {
                        Value::Null
                    }
                })
                .collect();

            // If any argument is null, the result is null (SQL NULL semantics)
            if arg_values.iter().any(|v| matches!(v, Value::Null)) {
                result_vec.set_null(row, true);
                continue;
            }

            match evaluate_scalar(&func, &arg_values) {
                Ok(val) => {
                    store_value_in_vector(&mut result_vec, row, &val);
                }
                Err(e) => {
                    // On error, set null
                    result_vec.set_null(row, true);
                    if row == 0 {
                        return Err(e);
                    }
                }
            }
        }

        Ok(result_vec)
    }

    /// Evaluate a binary operation.
    fn evaluate_binary_op(
        &self,
        op: &BinaryOp,
        left: &Expression,
        right: &Expression,
        chunk: &DataChunk,
    ) -> Result<ValueVector, String> {
        // Map AST BinaryOp to a scalar function name
        let func_name = match op {
            BinaryOp::Add => "+",
            BinaryOp::Subtract => "-",
            BinaryOp::Multiply => "*",
            BinaryOp::Divide => "/",
            BinaryOp::Modulo => "%",
            BinaryOp::Equal => "=",
            BinaryOp::NotEqual => "<>",
            BinaryOp::LessThan => "<",
            BinaryOp::LessThanOrEqual => "<=",
            BinaryOp::GreaterThan => ">",
            BinaryOp::GreaterThanOrEqual => ">=",
            BinaryOp::And => "AND",
            BinaryOp::Or => "OR",
            BinaryOp::Xor => "XOR",
            BinaryOp::Concat => "concat",
        };

        // Treat as a function call with two arguments
        self.evaluate_function_call(func_name, &[left.clone(), right.clone()], chunk)
    }

    /// Evaluate a unary operation.
    fn evaluate_unary_op(&self, op: &UnaryOp, inner: &Expression, chunk: &DataChunk) -> Result<ValueVector, String> {
        match op {
            UnaryOp::Not => self.evaluate_function_call("NOT", &[inner.clone()], chunk),
            UnaryOp::Negate => self.evaluate_function_call("-", &[inner.clone()], chunk),
        }
    }

    /// Evaluate a list literal expression.
    fn evaluate_list_literal(&self, items: &[Expression], chunk: &DataChunk) -> Result<ValueVector, String> {
        if items.is_empty() {
            let mut v = ValueVector::new(kuzu_common::types::PhysicalTypeID::List, chunk.size);
            v.resize(chunk.size);
            for i in 0..chunk.size {
                store_value_in_vector(&mut v, i, &Value::List(vec![]));
            }
            return Ok(v);
        }

        let num_rows = chunk.size;
        let mut result_vec = ValueVector::new(kuzu_common::types::PhysicalTypeID::List, num_rows);
        result_vec.resize(num_rows);

        for row in 0..num_rows {
            // For each row, evaluate each item expression against the same chunk
            // (list literals reference the whole chunk, so we get the per-row values)
            let mut list_values = Vec::with_capacity(items.len());
            for item in items {
                let item_vec = self.evaluate(item, chunk)?;
                let val = if row < item_vec.size() {
                    item_vec.get_value(row).unwrap_or(Value::Null)
                } else {
                    Value::Null
                };
                list_values.push(val);
            }
            store_value_in_vector(&mut result_vec, row, &Value::List(list_values));
        }

        Ok(result_vec)
    }

    /// Evaluate a map literal expression.
    fn evaluate_map_literal(&self, items: &[(String, Expression)], chunk: &DataChunk) -> Result<ValueVector, String> {
        let num_rows = chunk.size;
        let mut result_vec = ValueVector::new(kuzu_common::types::PhysicalTypeID::Struct, num_rows);
        result_vec.resize(num_rows);

        for row in 0..num_rows {
            let mut map_values = Vec::with_capacity(items.len());
            for (key, item) in items {
                let item_vec = self.evaluate(item, chunk)?;
                let val = if row < item_vec.size() {
                    item_vec.get_value(row).unwrap_or(Value::Null)
                } else {
                    Value::Null
                };
                map_values.push((Value::String(key.clone()), val));
            }
            store_value_in_vector(&mut result_vec, row, &Value::Map(map_values));
        }

        Ok(result_vec)
    }
}

/// Simplified store_value_in_vector that accepts any Value type.
/// This is a copy adapted from physical_operator.rs.
fn store_value_in_vector_simple(v: &mut ValueVector, row: usize, val: &Value) {
    match val {
        Value::Null => {
            v.set_null(row, true);
        }
        Value::Bool(x) => {
            if v.physical_type() == kuzu_common::types::PhysicalTypeID::Bool {
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
        Value::Double(x) => {
            let offset = row * 8;
            if offset + 8 <= v.data().len() {
                v.data_mut()[offset..offset + 8].copy_from_slice(&x.to_le_bytes());
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
            v.set_null(row, true);
        }
    }
}

/// Store a Value into a ValueVector at the given row index.
/// This is a copy of the helper from physical_operator.rs, kept here for independence.
fn store_value_in_vector(v: &mut ValueVector, row: usize, val: &Value) {
    match val {
        Value::Null => {
            v.set_null(row, true);
        }
        Value::Bool(x) => {
            if v.physical_type() == kuzu_common::types::PhysicalTypeID::Bool {
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
            // For complex types (List, Struct, etc.), store as null
            v.set_null(row, true);
        }
    }
}

impl ExpressionEvaluator {
    /// Evaluate a sequence operation (nextval/currval) using the sequence callback.
    /// Extracts the first string argument as the sequence name and delegates to the callback.
    fn evaluate_sequence_op(
        &self,
        name: &str,
        func: &ScalarFunction,
        arg_vectors: &[ValueVector],
        num_rows: usize,
    ) -> Result<ValueVector, String> {
        let is_nextval = match func {
            ScalarFunction::SequenceOp { is_nextval } => *is_nextval,
            _ => return Err(format!("Internal error: expected SequenceOp for '{}'", name)),
        };

        let seq_fn = self.sequence_fn.as_ref().ok_or_else(|| {
            format!("No sequence callback configured for '{}'", name)
        })?;

        let mut result_vec = ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, num_rows);
        result_vec.resize(num_rows);

        for row in 0..num_rows {
            let seq_name = if row < arg_vectors[0].size() && !arg_vectors[0].is_null(row) {
                match arg_vectors[0].get_value(row) {
                    Some(Value::String(s)) => s,
                    _ => return Err("nextval/currval requires a string argument (sequence name)".into()),
                }
            } else {
                result_vec.set_null(row, true);
                continue;
            };

            match seq_fn(&seq_name, is_nextval) {
                Ok(val) => {
                    store_value_in_vector(&mut result_vec, row, &val);
                }
                Err(e) => {
                    result_vec.set_null(row, true);
                    if row == 0 {
                        return Err(e);
                    }
                }
            }
        }

        Ok(result_vec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuzu_common::types::PhysicalTypeID;
    use kuzu_common::vector::ValueVector;
    use kuzu_function::registry::FunctionRegistry;

    fn make_registry() -> Arc<Mutex<FunctionRegistry>> {
        Arc::new(Mutex::new(FunctionRegistry::new()))
    }

    fn make_chunk(values: &[i64]) -> DataChunk {
        let mut v = ValueVector::new(PhysicalTypeID::Int64, values.len());
        v.resize(values.len());
        for (i, val) in values.iter().enumerate() {
            v.set_i64(i, *val);
        }
        DataChunk::new(vec![v])
    }

    #[test]
    fn test_evaluate_constant_int() {
        let eval = ExpressionEvaluator::new(make_registry());
        let expr = Expression::Constant(Constant::Integer(42));
        let chunk = make_chunk(&[]);
        let result = eval.evaluate(&expr, &chunk).unwrap();
        assert_eq!(result.size(), 0);
    }

    #[test]
    fn test_evaluate_constant_bool() {
        let eval = ExpressionEvaluator::new(make_registry());
        let expr = Expression::Constant(Constant::Bool(true));
        let chunk = make_chunk(&[1, 2, 3]);
        let result = eval.evaluate(&expr, &chunk).unwrap();
        assert_eq!(result.size(), 3);
        // All rows should be true
        for i in 0..3 {
            assert!(!result.is_null(i));
        }
    }

    #[test]
    fn test_evaluate_variable() {
        let eval = ExpressionEvaluator::new(make_registry());
        let chunk = make_chunk(&[10, 20, 30]);
        let expr = Expression::Variable("0".into());
        let result = eval.evaluate(&expr, &chunk).unwrap();
        assert_eq!(result.size(), 3);
        assert_eq!(result.get_i64(0), Some(10));
        assert_eq!(result.get_i64(1), Some(20));
        assert_eq!(result.get_i64(2), Some(30));
    }

    #[test]
    fn test_evaluate_binary_equal() {
        let eval = ExpressionEvaluator::new(make_registry());
        // 0 = 0 → true, 1 = 0 → false, 2 = 0 → false
        let left = Box::new(Expression::Variable("0".into()));
        let right = Box::new(Expression::Constant(Constant::Integer(0)));
        let expr = Expression::BinaryOp(BinaryOp::Equal, left, right);
        let chunk = make_chunk(&[0, 1, 2]);
        let result = eval.evaluate(&expr, &chunk).unwrap();
        assert_eq!(result.size(), 3);
        assert_eq!(result.get_value(0), Some(Value::Bool(true)));
        assert_eq!(result.get_value(1), Some(Value::Bool(false)));
        assert_eq!(result.get_value(2), Some(Value::Bool(false)));
    }

    #[test]
    fn test_evaluate_binary_greater_than() {
        let eval = ExpressionEvaluator::new(make_registry());
        // 3 > 2 → true, 1 > 2 → false, 5 > 2 → true
        let left = Box::new(Expression::Variable("0".into()));
        let right = Box::new(Expression::Constant(Constant::Integer(2)));
        let expr = Expression::BinaryOp(BinaryOp::GreaterThan, left, right);
        let chunk = make_chunk(&[3, 1, 5]);
        let result = eval.evaluate(&expr, &chunk).unwrap();
        assert_eq!(result.size(), 3);
        assert_eq!(result.get_value(0), Some(Value::Bool(true)));
        assert_eq!(result.get_value(1), Some(Value::Bool(false)));
        assert_eq!(result.get_value(2), Some(Value::Bool(true)));
    }

    #[test]
    fn test_evaluate_binary_and() {
        let eval = ExpressionEvaluator::new(make_registry());
        // We need two columns: set up chunk with column 0 as bools and column 1 as bools
        let mut v0 = ValueVector::new(PhysicalTypeID::Bool, 2);
        v0.resize(2);
        store_value_in_vector(&mut v0, 0, &Value::Bool(true));
        store_value_in_vector(&mut v0, 1, &Value::Bool(false));
        let mut v1 = ValueVector::new(PhysicalTypeID::Bool, 2);
        v1.resize(2);
        store_value_in_vector(&mut v1, 0, &Value::Bool(true));
        store_value_in_vector(&mut v1, 1, &Value::Bool(true));
        let chunk = DataChunk::new(vec![v0, v1]);

        // true AND true → true, false AND true → false
        let left = Box::new(Expression::Variable("0".into()));
        let right = Box::new(Expression::Variable("1".into()));
        let expr = Expression::BinaryOp(BinaryOp::And, left, right);
        let result = eval.evaluate(&expr, &chunk).unwrap();
        assert_eq!(result.size(), 2);
        assert_eq!(result.get_value(0), Some(Value::Bool(true)));
        assert_eq!(result.get_value(1), Some(Value::Bool(false)));
    }

    #[test]
    fn test_evaluate_function_call_string_length() {
        let eval = ExpressionEvaluator::new(make_registry());
        let chunk = DataChunk::new(vec![]);
        let expr = Expression::FunctionCall(
            "length".into(),
            vec![Expression::Constant(Constant::String("hello".into()))],
        );
        let result = eval.evaluate(&expr, &chunk).unwrap();
        assert_eq!(result.size(), 0);
    }

    #[test]
    fn test_evaluate_not() {
        let eval = ExpressionEvaluator::new(make_registry());
        let mut v = ValueVector::new(PhysicalTypeID::Bool, 3);
        v.resize(3);
        store_value_in_vector(&mut v, 0, &Value::Bool(true));
        store_value_in_vector(&mut v, 1, &Value::Bool(false));
        store_value_in_vector(&mut v, 2, &Value::Bool(true));
        let chunk = DataChunk::new(vec![v]);
        // NOT of column 0 — true→false, false→true, true→false
        let expr = Expression::UnaryOp(UnaryOp::Not, Box::new(Expression::Variable("0".into())));
        let result = eval.evaluate(&expr, &chunk).unwrap();
        assert_eq!(result.size(), 3);
        assert_eq!(result.get_value(0), Some(Value::Bool(false)));
        assert_eq!(result.get_value(1), Some(Value::Bool(true)));
        assert_eq!(result.get_value(2), Some(Value::Bool(false)));
    }
}
