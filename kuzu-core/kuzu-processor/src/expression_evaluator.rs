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

use arrow::array::ArrayRef;
use kuzu_common::arrow_vector::{ArrowVector, VectorAccess};
use kuzu_common::types::{PhysicalTypeID, Value};
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_function::registry::{FunctionRegistry, ScalarFunction};
use kuzu_function::scalar::evaluate_scalar;
use kuzu_parser::ast::{BinaryOp, Constant, Expression, Query, UnaryOp};
use std::sync::{Arc, Mutex};

pub type SubqueryFn = Arc<dyn Fn(&Query) -> Result<Vec<DataChunk>, String> + Send + Sync>;
pub type SequenceFn = Arc<dyn Fn(&str, bool) -> Result<Value, String> + Send + Sync>;

/// Evaluates expressions against DataChunks using the function registry.
impl std::fmt::Debug for ExpressionEvaluator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExpressionEvaluator").finish()
    }
}

pub struct ExpressionEvaluator {
    registry: Arc<Mutex<FunctionRegistry>>,
    /// Optional callback to execute subqueries at evaluation time.
    /// Takes a parsed Query and returns DataChunks.
    pub subquery_fn: Option<SubqueryFn>,
    /// Optional callback for sequence operations (nextval/currval).
    /// Takes (sequence_name, is_nextval) and returns the resulting value.
    pub sequence_fn: Option<SequenceFn>,
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
    pub fn with_subquery_fn(mut self, f: SubqueryFn) -> Self {
        self.subquery_fn = Some(f);
        self
    }

    /// Set the sequence operation callback (for nextval/currval).
    pub fn with_sequence_fn(mut self, f: SequenceFn) -> Self {
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

    /// Evaluate an expression for every row in the chunk, returning an ArrowVector
    /// directly. Delegates to evaluate_arrow which uses Arrow compute kernels for
    /// the hot path (comparisons, arithmetic, boolean ops).
    pub fn evaluate_to_arrow(&self, expr: &Expression, chunk: &DataChunk) -> Result<ArrowVector, String> {
        self.evaluate_arrow(expr, chunk)
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
            Expression::Case(case_expr) => self.evaluate_case(case_expr, chunk),
            Expression::Star => {
                Err("STAR expression should be expanded by the binder before reaching the evaluator".into())
            }
            Expression::ListPredicate {
                quantifier,
                list,
                var_name,
                predicate,
            } => self.evaluate_list_predicate(quantifier, list, var_name, predicate, chunk),
            Expression::Lambda { .. } => {
                Err("Lambda expression should only appear as argument to list_transform/filter/reduce".into())
            }
        }
    }

    // ==================== Arrow-native evaluation ====================

    /// Evaluate an expression returning an ArrowVector directly.
    /// For operations supported by Arrow compute kernels (comparisons,
    /// arithmetic, boolean), this is vectorized and avoids Value enum boxing.
    pub fn evaluate_arrow(&self, expr: &Expression, chunk: &DataChunk) -> Result<ArrowVector, String> {
        match expr {
            Expression::Constant(c) => self.evaluate_arrow_constant(c, chunk.size),
            Expression::Variable(name) => self.evaluate_arrow_variable(name, chunk),
            Expression::PropertyAccess(obj, prop) => {
                self.evaluate_arrow_property_access(obj, prop, chunk)
            }
            Expression::FunctionCall(name, args) => {
                self.evaluate_arrow_function_call(name, args, chunk)
            }
            Expression::BinaryOp(op, left, right) => {
                self.evaluate_arrow_binary_op(op, left, right, chunk)
            }
            Expression::UnaryOp(op, inner) => self.evaluate_arrow_unary_op(op, inner, chunk),
            // Fallback for complex types: use evaluate + from_legacy
            _ => {
                let legacy = self.evaluate(expr, chunk)?;
                Ok(ArrowVector::from_legacy(&legacy))
            }
        }
    }

    /// Evaluate a constant directly as ArrowVector using typed Arrow builders.
    fn evaluate_arrow_constant(&self, c: &Constant, size: usize) -> Result<ArrowVector, String> {
        match c {
            Constant::Null => {
                let mut builder = arrow::array::Int64Builder::with_capacity(size);
                builder.append_nulls(size);
                Ok(ArrowVector::new(Arc::new(builder.finish()), PhysicalTypeID::Int64))
            }
            Constant::Bool(b) => {
                let mut builder = arrow::array::BooleanBuilder::with_capacity(size);
                for _ in 0..size {
                    builder.append_value(*b);
                }
                Ok(ArrowVector::new(Arc::new(builder.finish()), PhysicalTypeID::Bool))
            }
            Constant::Integer(i) => {
                let mut builder = arrow::array::Int64Builder::with_capacity(size);
                let v = *i;
                for _ in 0..size {
                    builder.append_value(v);
                }
                Ok(ArrowVector::new(Arc::new(builder.finish()), PhysicalTypeID::Int64))
            }
            Constant::Float(f) => {
                let mut builder = arrow::array::Float64Builder::with_capacity(size);
                let v = *f;
                for _ in 0..size {
                    builder.append_value(v);
                }
                Ok(ArrowVector::new(Arc::new(builder.finish()), PhysicalTypeID::Double))
            }
            Constant::String(s) => {
                let mut builder = arrow::array::StringBuilder::with_capacity(size, size * s.len().max(1));
                for _ in 0..size {
                    builder.append_value(s);
                }
                Ok(ArrowVector::new(Arc::new(builder.finish()), PhysicalTypeID::String))
            }
        }
    }

    /// Evaluate a variable expression, converting the ValueVector to ArrowVector.
    fn evaluate_arrow_variable(&self, name: &str, chunk: &DataChunk) -> Result<ArrowVector, String> {
        let idx = if let Ok(idx) = name.parse::<usize>() {
            idx
        } else if !chunk.field_names.is_empty() {
            if let Some(idx) = chunk.field_names.iter().position(|n| n == name) {
                idx
            } else {
                return Err(format!("Variable '{}' not found in field_names", name));
            }
        } else {
            return Err(format!("Variable '{}' not found (chunk has no field_names)", name));
        };

        let field = chunk.fields.get(idx).ok_or_else(|| {
            format!("Variable '{}' (index {}) not found in chunk fields", name, idx)
        })?;
        Ok(ArrowVector::from_legacy(field))
    }

    /// Evaluate a property access expression, returning ArrowVector.
    fn evaluate_arrow_property_access(&self, obj: &Expression, prop: &str, chunk: &DataChunk) -> Result<ArrowVector, String> {
        let qualified_prop = if let Expression::Variable(var_name) = obj {
            format!("{}.{}", var_name, prop)
        } else {
            prop.to_string()
        };

        if !chunk.field_names.is_empty()
            && let Some(idx) = chunk.field_names.iter().position(|n| n == &qualified_prop || n == prop)
        {
            let field = chunk.fields.get(idx).ok_or_else(|| {
                format!("Property '{}' not found in chunk", prop)
            })?;
            return Ok(ArrowVector::from_legacy(field));
        }
        let legacy = self.evaluate(obj, chunk)?;
        Ok(ArrowVector::from_legacy(&legacy))
    }

    /// Evaluate a binary operation using Arrow compute kernels when possible.
    fn evaluate_arrow_binary_op(
        &self,
        op: &BinaryOp,
        left: &Expression,
        right: &Expression,
        chunk: &DataChunk,
    ) -> Result<ArrowVector, String> {
        match op {
            BinaryOp::In | BinaryOp::NotIn => {
                let legacy = self.evaluate_in_op(op, left, right, chunk)?;
                return Ok(ArrowVector::from_legacy(&legacy));
            }
            BinaryOp::Concat | BinaryOp::StartsWith | BinaryOp::EndsWith | BinaryOp::Contains => {
                let func_name = match op {
                    BinaryOp::Concat => "concat",
                    BinaryOp::StartsWith => "starts_with",
                    BinaryOp::EndsWith => "ends_with",
                    BinaryOp::Contains => "contains",
                    _ => unreachable!(),
                };
                return self.evaluate_arrow_function_call(func_name, &[left.clone(), right.clone()], chunk);
            }
            _ => {}
        }

        let kernel_name = match op {
            BinaryOp::Add => "add",
            BinaryOp::Subtract => "sub",
            BinaryOp::Multiply => "mul",
            BinaryOp::Divide => "div",
            BinaryOp::Modulo => "mod",
            BinaryOp::Equal => "eq",
            BinaryOp::NotEqual => "neq",
            BinaryOp::LessThan => "lt",
            BinaryOp::LessThanOrEqual => "lt_eq",
            BinaryOp::GreaterThan => "gt",
            BinaryOp::GreaterThanOrEqual => "gt_eq",
            BinaryOp::And => "and",
            BinaryOp::Or => "or",
            BinaryOp::Xor => "xor",
            _ => return Err(format!("Unsupported binary op: {:?}", op)),
        };

        let left_arrow = self.evaluate_arrow(left, chunk)?;
        let right_arrow = self.evaluate_arrow(right, chunk)?;

        match self.apply_arrow_kernel(kernel_name, &left_arrow, &right_arrow) {
            Ok(result) => Ok(result),
            Err(_) => {
                let fallback = Expression::BinaryOp(op.clone(), Box::new((*left).clone()), Box::new((*right).clone()));
                let legacy = self.evaluate(&fallback, chunk)?;
                Ok(ArrowVector::from_legacy(&legacy))
            }
        }
    }

    /// Evaluate a unary operation using Arrow compute kernels when possible.
    fn evaluate_arrow_unary_op(&self, op: &UnaryOp, inner: &Expression, chunk: &DataChunk) -> Result<ArrowVector, String> {
        match op {
            UnaryOp::Not => {
                let inner_arrow = self.evaluate_arrow(inner, chunk)?;
                self.apply_arrow_unary_kernel("not", &inner_arrow).or_else(|_| {
                    let fallback = Expression::UnaryOp(UnaryOp::Not, Box::new(inner.clone()));
                    let legacy = self.evaluate(&fallback, chunk)?;
                    Ok(ArrowVector::from_legacy(&legacy))
                })
            }
            UnaryOp::Negate => {
                let inner_arrow = self.evaluate_arrow(inner, chunk)?;
                self.apply_arrow_unary_kernel("negate", &inner_arrow).or_else(|_| {
                    let fallback = Expression::UnaryOp(UnaryOp::Negate, Box::new(inner.clone()));
                    let legacy = self.evaluate(&fallback, chunk)?;
                    Ok(ArrowVector::from_legacy(&legacy))
                })
            }
            UnaryOp::IsNull => {
                let inner_arrow = self.evaluate_arrow(inner, chunk)?;
                self.apply_arrow_unary_kernel("is_null", &inner_arrow)
            }
            UnaryOp::IsNotNull => {
                let inner_arrow = self.evaluate_arrow(inner, chunk)?;
                self.apply_arrow_unary_kernel("is_not_null", &inner_arrow)
            }
        }
    }

    /// Apply an Arrow binary compute kernel.
    fn apply_arrow_kernel(&self, name: &str, left: &ArrowVector, right: &ArrowVector) -> Result<ArrowVector, String> {
        use arrow::compute::kernels::boolean::{and_kleene, or_kleene};
        use arrow::compute::kernels::cmp::{eq, gt, gt_eq, lt, lt_eq, neq};
        use arrow::compute::kernels::numeric::{add, div, mul, rem, sub};

        let result: ArrayRef = match name {
            "add" => Arc::new(add(&left.array, &right.array).map_err(|e| format!("Arrow {name} failed: {e}"))?),
            "sub" => Arc::new(sub(&left.array, &right.array).map_err(|e| format!("Arrow {name} failed: {e}"))?),
            "mul" => Arc::new(mul(&left.array, &right.array).map_err(|e| format!("Arrow {name} failed: {e}"))?),
            "div" => Arc::new(div(&left.array, &right.array).map_err(|e| format!("Arrow {name} failed: {e}"))?),
            "mod" => Arc::new(rem(&left.array, &right.array).map_err(|e| format!("Arrow {name} failed: {e}"))?),
            "eq" => Arc::new(eq(&left.array, &right.array).map_err(|e| format!("Arrow {name} failed: {e}"))?),
            "neq" => Arc::new(neq(&left.array, &right.array).map_err(|e| format!("Arrow {name} failed: {e}"))?),
            "lt" => Arc::new(lt(&left.array, &right.array).map_err(|e| format!("Arrow {name} failed: {e}"))?),
            "lt_eq" => Arc::new(lt_eq(&left.array, &right.array).map_err(|e| format!("Arrow {name} failed: {e}"))?),
            "gt" => Arc::new(gt(&left.array, &right.array).map_err(|e| format!("Arrow {name} failed: {e}"))?),
            "gt_eq" => Arc::new(gt_eq(&left.array, &right.array).map_err(|e| format!("Arrow {name} failed: {e}"))?),
            "and" => {
                let l = left.array.as_any().downcast_ref::<arrow::array::BooleanArray>()
                    .ok_or_else(|| format!("Arrow {name}: expected BooleanArray, got {:?}", left.array.data_type()))?
                    .clone();
                let r = right.array.as_any().downcast_ref::<arrow::array::BooleanArray>()
                    .ok_or_else(|| format!("Arrow {name}: expected BooleanArray, got {:?}", right.array.data_type()))?
                    .clone();
                Arc::new(and_kleene(&l, &r).map_err(|e| format!("Arrow {name} failed: {e}"))?)
            }
            "or" => {
                let l = left.array.as_any().downcast_ref::<arrow::array::BooleanArray>()
                    .ok_or_else(|| format!("Arrow {name}: expected BooleanArray, got {:?}", left.array.data_type()))?
                    .clone();
                let r = right.array.as_any().downcast_ref::<arrow::array::BooleanArray>()
                    .ok_or_else(|| format!("Arrow {name}: expected BooleanArray, got {:?}", right.array.data_type()))?
                    .clone();
                Arc::new(or_kleene(&l, &r).map_err(|e| format!("Arrow {name} failed: {e}"))?)
            }
            "xor" => {
                // XOR = (l AND NOT r) OR (NOT l AND r)
                let l = left.array.as_any().downcast_ref::<arrow::array::BooleanArray>()
                    .ok_or_else(|| format!("Arrow {name}: expected BooleanArray, got {:?}", left.array.data_type()))?;
                let r = right.array.as_any().downcast_ref::<arrow::array::BooleanArray>()
                    .ok_or_else(|| format!("Arrow {name}: expected BooleanArray, got {:?}", right.array.data_type()))?;
                let not_r = arrow::compute::kernels::boolean::not(r)
                    .map_err(|e| format!("Arrow xor/not failed: {e}"))?;
                let not_l = arrow::compute::kernels::boolean::not(l)
                    .map_err(|e| format!("Arrow xor/not failed: {e}"))?;
                let l_and_not_r = and_kleene(l, &not_r)
                    .map_err(|e| format!("Arrow xor/and failed: {e}"))?;
                let not_l_and_r = and_kleene(&not_l, r)
                    .map_err(|e| format!("Arrow xor/and failed: {e}"))?;
                Arc::new(or_kleene(&l_and_not_r, &not_l_and_r)
                    .map_err(|e| format!("Arrow xor/or failed: {e}"))?)
            }
            _ => return Err(format!("Unknown binary kernel: {name}")),
        };

        let phys_type = match name {
            "eq" | "neq" | "lt" | "lt_eq" | "gt" | "gt_eq" | "and" | "or" | "xor" => PhysicalTypeID::Bool,
            _ => left.physical_type,
        };

        Ok(ArrowVector::new(result, phys_type))
    }

    /// Apply an Arrow unary compute kernel.
    fn apply_arrow_unary_kernel(&self, name: &str, arr: &ArrowVector) -> Result<ArrowVector, String> {
        use arrow::compute::kernels::boolean::{is_null, is_not_null, not};
        use arrow::compute::kernels::numeric::neg;

        let result: ArrayRef = match name {
            "not" => {
                let arr_ref = arr.array.as_any().downcast_ref::<arrow::array::BooleanArray>()
                    .ok_or_else(|| format!("Arrow {name}: expected BooleanArray, got {:?}", arr.array.data_type()))?;
                Arc::new(not(arr_ref).map_err(|e| format!("Arrow {name} failed: {e}"))?)
            }
            "negate" => Arc::new(neg(&*arr.array).map_err(|e| format!("Arrow {name} failed: {e}"))?),
            "is_null" => {
                Arc::new(is_null(&*arr.array).map_err(|e| format!("Arrow {name} failed: {e}"))?)
            }
            "is_not_null" => {
                Arc::new(is_not_null(&*arr.array).map_err(|e| format!("Arrow {name} failed: {e}"))?)
            }
            _ => return Err(format!("Unknown unary kernel: {name}")),
        };

        let phys_type = if matches!(name, "is_null" | "is_not_null" | "not") {
            PhysicalTypeID::Bool
        } else {
            arr.physical_type
        };

        Ok(ArrowVector::new(result, phys_type))
    }

    /// Evaluate a function call, producing ArrowVector directly.
    /// For fixed-width return types, uses typed Vec<T> collection to
    /// avoid the intermediate ValueVector allocation.
    fn evaluate_arrow_function_call(
        &self,
        name: &str,
        args: &[Expression],
        chunk: &DataChunk,
    ) -> Result<ArrowVector, String> {
        // Lambda-based functions use complex control flow — stick with evaluate
        if let Some(_lambda) = self.extract_lambda_arg(args) {
            match name {
                "list_transform" | "list_filter" | "list_reduce" => {
                    let legacy = self.evaluate_function_call(name, args, chunk)?;
                    return Ok(ArrowVector::from_legacy(&legacy));
                }
                _ => {}
            }
        }

        // Evaluate arguments as ArrowVectors
        let arg_arrows: Vec<ArrowVector> = args
            .iter()
            .map(|arg| self.evaluate_arrow(arg, chunk))
            .collect::<Result<Vec<_>, _>>()?;

        if arg_arrows.is_empty() {
            return Err(format!("Function '{}' requires at least one argument", name));
        }

        let num_rows = arg_arrows[0].size();

        // Look up the function
        let func = {
            let reg = self.registry.lock().unwrap();
            reg.get_scalar(name).cloned()
        };
        let func = match func {
            Some(f) => f,
            None => return Err(format!("Unknown function: '{}'", name)),
        };

        // SequenceOp needs the callback — delegate to evaluate
        if matches!(func, ScalarFunction::SequenceOp { .. }) {
            let legacy = self.evaluate_function_call(name, args, chunk)?;
            return Ok(ArrowVector::from_legacy(&legacy));
        }

        let mut first_error: Option<String> = None;
        let mut row_results: Vec<Value> = Vec::with_capacity(num_rows);

        for row in 0..num_rows {
            let arg_values: Vec<Value> = arg_arrows
                .iter()
                .map(|arr| {
                    if row < arr.size() && !arr.is_null(row) {
                        arr.get_value(row).unwrap_or(Value::Null)
                    } else {
                        Value::Null
                    }
                })
                .collect();

            if arg_values.iter().any(|v| matches!(v, Value::Null)) {
                row_results.push(Value::Null);
                continue;
            }

            match evaluate_scalar(&func, &arg_values) {
                Ok(val) => row_results.push(val),
                Err(e) => {
                    row_results.push(Value::Null);
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                }
            }
        }

        let result_type = row_results
            .iter()
            .find(|v| !matches!(v, Value::Null))
            .map(|v| v.physical_type())
            .unwrap_or(PhysicalTypeID::Int64);

        // Build Arrow array directly from typed Vec<T> — avoids ValueVector allocation
        let arrow_result = build_arrow_from_values(&row_results, result_type, num_rows)?;

        if row_results.iter().all(|v| matches!(v, Value::Null))
            && let Some(e) = first_error
        {
            return Err(e);
        }

        Ok(arrow_result)
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
    /// The variable name is matched against:
    /// 1. Numeric index (binder resolves names to positions, e.g., "0", "1")
    /// 2. Chunk field names (e.g., "title", "d.title")
    /// 3. Falls back to the first field (legacy compatibility) if no match found.
    fn evaluate_variable(&self, name: &str, chunk: &DataChunk) -> Result<ValueVector, String> {
        // Try to find the field by numeric position (the binder resolves names to positions)
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

        // Try to find the field by name in the chunk's field names.
        if !chunk.field_names.is_empty() {
            if let Some(idx) = chunk.field_names.iter().position(|n| n == name) {
                return chunk.fields.get(idx).cloned().ok_or_else(|| {
                    format!(
                        "Variable '{}' (field name match at index {}) not found in chunk",
                        name, idx
                    )
                });
            }
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

    /// Evaluate a property access expression — resolves the property name to a
    /// column index using `chunk.field_names`, then returns that column's data.
    ///
    /// Falls back to evaluating the object expression (legacy behaviour) if no
    /// `field_names` are available on the chunk.
    fn evaluate_property_access(&self, obj: &Expression, prop: &str, chunk: &DataChunk) -> Result<ValueVector, String> {
        // Build the qualified property name (e.g., "t.name")
        let qualified_prop = if let Expression::Variable(var_name) = obj {
            format!("{}.{}", var_name, prop)
        } else {
            prop.to_string()
        };

        // Fast path: look up the property by name in the chunk's field names.
        if !chunk.field_names.is_empty()
            && let Some(idx) = chunk.field_names.iter().position(|n| n == &qualified_prop || n == prop)
        {
            return chunk
                .fields
                .get(idx)
                .cloned()
                .ok_or_else(|| format!("Column '{}' (index {}) not found in chunk", prop, idx));
        }
        // Fallback: evaluate the object expression (returns first column — legacy behaviour).
        self.evaluate(obj, chunk)
    }

    /// Evaluate a function call expression.
    fn evaluate_function_call(
        &self,
        name: &str,
        args: &[Expression],
        chunk: &DataChunk,
    ) -> Result<ValueVector, String> {
        // Handle lambda-based list functions at expression level.
        // These cannot go through the normal scalar function pipeline because
        // lambda expressions are not Values and must be evaluated per-element.
        if let Some(lambda) = self.extract_lambda_arg(args) {
            match name {
                "list_transform" => return self.evaluate_list_transform(args, lambda, chunk),
                "list_filter" => return self.evaluate_list_filter(args, lambda, chunk),
                "list_reduce" => return self.evaluate_list_reduce(args, lambda, chunk),
                _ => {}
            }
        }

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

        // Evaluate each row exactly once, then infer result type from cached values.
        // This avoids double-invoking side-effecting functions (e.g., nextval custom scalar).
        let mut row_results: Vec<Value> = Vec::with_capacity(num_rows);
        let mut first_error: Option<String> = None;

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
                row_results.push(Value::Null);
                continue;
            }

            match evaluate_scalar(&func, &arg_values) {
                Ok(val) => row_results.push(val),
                Err(e) => {
                    row_results.push(Value::Null);
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                }
            }
        }

        let result_type = row_results
            .iter()
            .find(|v| !matches!(v, Value::Null))
            .map(|v| v.physical_type())
            .unwrap_or(kuzu_common::types::PhysicalTypeID::Int64);

        let mut result_vec = ValueVector::new(result_type, num_rows);
        result_vec.resize(num_rows);
        for (row, val) in row_results.iter().enumerate() {
            store_value_in_vector(&mut result_vec, row, val);
        }

        if row_results.iter().all(|v| matches!(v, Value::Null))
            && let Some(e) = first_error
        {
            return Err(e);
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
            // Handled inline — not mapped to scalar function
            BinaryOp::In | BinaryOp::NotIn => {
                return self.evaluate_in_op(op, left, right, chunk);
            }
            BinaryOp::StartsWith => "starts_with",
            BinaryOp::EndsWith => "ends_with",
            BinaryOp::Contains => "contains",
        };

        // Treat as a function call with two arguments
        self.evaluate_function_call(func_name, &[left.clone(), right.clone()], chunk)
    }

    /// Evaluate a unary operation.
    fn evaluate_unary_op(&self, op: &UnaryOp, inner: &Expression, chunk: &DataChunk) -> Result<ValueVector, String> {
        match op {
            UnaryOp::Not => self.evaluate_function_call("NOT", std::slice::from_ref(inner), chunk),
            UnaryOp::Negate => self.evaluate_function_call("-", std::slice::from_ref(inner), chunk),
            UnaryOp::IsNull => {
                let vec = self.evaluate(inner, chunk)?;
                let num_rows = vec.size();
                let mut result = ValueVector::new(kuzu_common::types::PhysicalTypeID::Bool, num_rows);
                result.resize(num_rows);
                for i in 0..num_rows {
                    let is_null = vec.is_null(i) || matches!(vec.get_value(i), Some(Value::Null) | None);
                    store_value_in_vector_simple(&mut result, i, &Value::Bool(is_null));
                }
                Ok(result)
            }
            UnaryOp::IsNotNull => {
                let vec = self.evaluate(inner, chunk)?;
                let num_rows = vec.size();
                let mut result = ValueVector::new(kuzu_common::types::PhysicalTypeID::Bool, num_rows);
                result.resize(num_rows);
                for i in 0..num_rows {
                    let is_null = vec.is_null(i) || matches!(vec.get_value(i), Some(Value::Null) | None);
                    store_value_in_vector_simple(&mut result, i, &Value::Bool(!is_null));
                }
                Ok(result)
            }
        }
    }

    /// Evaluate `x IN list` and `x NOT IN list` operators.
    fn evaluate_in_op(
        &self,
        op: &BinaryOp,
        left: &Expression,
        right: &Expression,
        chunk: &DataChunk,
    ) -> Result<ValueVector, String> {
        let left_vec = self.evaluate(left, chunk)?;
        let right_vec = self.evaluate(right, chunk)?;
        let num_rows = chunk.size;
        let mut result = ValueVector::new(kuzu_common::types::PhysicalTypeID::Bool, num_rows);
        result.resize(num_rows);
        for row in 0..num_rows {
            let lv = left_vec.get_value(row).unwrap_or(Value::Null);
            let rv = right_vec.get_value(row).unwrap_or(Value::Null);
            if matches!(lv, Value::Null) {
                result.set_null(row, true);
                continue;
            }
            let in_list = match &rv {
                Value::List(items) => items.contains(&lv),
                _ => lv == rv,
            };
            let result_val = if *op == BinaryOp::NotIn { !in_list } else { in_list };
            store_value_in_vector_simple(&mut result, row, &Value::Bool(result_val));
        }
        Ok(result)
    }

    /// Evaluate a `CASE [subject] WHEN ... THEN ... [ELSE ...] END` expression.
    fn evaluate_case(&self, case_expr: &kuzu_parser::ast::CaseExpr, chunk: &DataChunk) -> Result<ValueVector, String> {
        let num_rows = chunk.size;
        // Evaluate subject (if any)
        let subject_vec = if let Some(subj) = &case_expr.subject {
            Some(self.evaluate(subj, chunk)?)
        } else {
            None
        };

        // We need to find the result type first by speculatively checking THEN exprs
        // Strategy: evaluate all WHEN/THEN in order per row
        // Determine result type from first THEN expr evaluation
        let result_type = {
            let first_then = self.evaluate(&case_expr.alternatives[0].then, chunk)?;
            first_then.physical_type()
        };

        let mut result = ValueVector::new(result_type, num_rows);
        result.resize(num_rows);

        // For each row, find the matching branch
        for row in 0..num_rows {
            let subject_val = subject_vec.as_ref().and_then(|sv| sv.get_value(row));

            let mut matched = false;
            for alt in &case_expr.alternatives {
                let when_vec = self.evaluate(&alt.when, chunk)?;
                let when_val = when_vec.get_value(row).unwrap_or(Value::Null);

                // Simple CASE: compare subject == when_val
                // Searched CASE: when_val is a boolean condition
                let branch_taken = if let Some(ref sv) = subject_val {
                    when_val != Value::Null && when_val == *sv
                } else {
                    matches!(when_val, Value::Bool(true))
                };

                if branch_taken {
                    let then_vec = self.evaluate(&alt.then, chunk)?;
                    let then_val = then_vec.get_value(row).unwrap_or(Value::Null);
                    store_value_in_vector(&mut result, row, &then_val);
                    matched = true;
                    break;
                }
            }

            if !matched {
                if let Some(else_e) = &case_expr.else_expr {
                    let else_vec = self.evaluate(else_e, chunk)?;
                    let else_val = else_vec.get_value(row).unwrap_or(Value::Null);
                    store_value_in_vector(&mut result, row, &else_val);
                } else {
                    result.set_null(row, true);
                }
            }
        }

        Ok(result)
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

    /// Evaluate an ANY/ALL/NONE/SINGLE list predicate.
    /// Evaluates the list expression, then for each element evaluates the
    /// predicate and applies the quantifier logic.
    fn evaluate_list_predicate(
        &self,
        quantifier: &kuzu_parser::ast::Quantifier,
        list: &Expression,
        _var_name: &str,
        predicate: &Expression,
        chunk: &DataChunk,
    ) -> Result<ValueVector, String> {
        // Evaluate the list expression to get a ValueVector
        let list_vec = self.evaluate(list, chunk)?;
        let num_rows = chunk.size;
        let mut result = ValueVector::new(kuzu_common::types::PhysicalTypeID::Bool, num_rows);
        result.resize(num_rows);

        for row in 0..num_rows {
            let list_val = list_vec.get_value(row).unwrap_or(Value::Null);
            let items = match list_val {
                Value::List(ref items) => items.clone(),
                _ => {
                    // Not a list → false for all quantifiers
                    store_value_in_vector(&mut result, row, &Value::Bool(false));
                    continue;
                }
            };

            // For each element, create a mini-chunk with the variable bound
            // and evaluate the predicate
            let mut true_count = 0u64;
            for item in &items {
                // Create a single-row chunk with the variable as first field
                let mut elem_vec = ValueVector::new(item.physical_type(), 1);
                elem_vec.resize(1);
                store_value_in_vector(&mut elem_vec, 0, item);
                let mini_chunk = DataChunk::new(vec![elem_vec]);

                let pred_vec = self.evaluate(predicate, &mini_chunk)?;
                let pred_val = pred_vec.get_value(0).unwrap_or(Value::Null);

                if matches!(pred_val, Value::Bool(true)) {
                    true_count += 1;
                }
            }

            // Apply quantifier logic
            let elem_count = items.len() as u64;
            let bool_result = match quantifier {
                kuzu_parser::ast::Quantifier::Any => true_count > 0,
                kuzu_parser::ast::Quantifier::All => !items.is_empty() && true_count == elem_count,
                kuzu_parser::ast::Quantifier::None => true_count == 0,
                kuzu_parser::ast::Quantifier::Single => true_count == 1,
            };
            store_value_in_vector(&mut result, row, &Value::Bool(bool_result));
        }

        Ok(result)
    }

    /// Extract the Lambda expression from function call arguments, if present.
    fn extract_lambda_arg<'a>(&self, args: &'a [Expression]) -> Option<&'a Expression> {
        args.iter().find(|a| matches!(a, Expression::Lambda { .. }))
    }

    /// Evaluate `list_transform(list, x -> body)` — apply lambda to each element.
    fn evaluate_list_transform(
        &self,
        args: &[Expression],
        lambda: &Expression,
        chunk: &DataChunk,
    ) -> Result<ValueVector, String> {
        let list_expr = args.iter().find(|a| !matches!(a, Expression::Lambda { .. }))
            .ok_or("list_transform requires a list argument")?;

        let (var_name, body) = match lambda {
            Expression::Lambda { var_name, body } => (var_name, body),
            _ => return Err("Expected lambda expression".into()),
        };

        let list_vec = self.evaluate(list_expr, chunk)?;
        let num_rows = chunk.size;
        let mut result = ValueVector::new(kuzu_common::types::PhysicalTypeID::List, num_rows);
        result.resize(num_rows);

        for row in 0..num_rows {
            let list_val = list_vec.get_value(row).unwrap_or(Value::Null);
            let items = match list_val {
                Value::List(ref items) => items.clone(),
                _ => {
                    store_value_in_vector(&mut result, row, &Value::List(vec![]));
                    continue;
                }
            };

            let mut transformed: Vec<Value> = Vec::with_capacity(items.len());
            for item in &items {
                let mut elem_vec = ValueVector::new(item.physical_type(), 1);
                elem_vec.resize(1);
                store_value_in_vector(&mut elem_vec, 0, item);
                let mut mini_chunk = DataChunk::new(vec![elem_vec]);
                mini_chunk.field_names.push(var_name.clone());

                let body_vec = self.evaluate(body, &mini_chunk)?;
                let body_val = body_vec.get_value(0).unwrap_or(Value::Null);
                transformed.push(body_val);
            }
            store_value_in_vector(&mut result, row, &Value::List(transformed));
        }

        Ok(result)
    }

    /// Evaluate `list_filter(list, x -> predicate)` — keep elements where predicate is true.
    fn evaluate_list_filter(
        &self,
        args: &[Expression],
        lambda: &Expression,
        chunk: &DataChunk,
    ) -> Result<ValueVector, String> {
        let list_expr = args.iter().find(|a| !matches!(a, Expression::Lambda { .. }))
            .ok_or("list_filter requires a list argument")?;

        let (var_name, body) = match lambda {
            Expression::Lambda { var_name, body } => (var_name, body),
            _ => return Err("Expected lambda expression".into()),
        };

        let list_vec = self.evaluate(list_expr, chunk)?;
        let num_rows = chunk.size;
        let mut result = ValueVector::new(kuzu_common::types::PhysicalTypeID::List, num_rows);
        result.resize(num_rows);

        for row in 0..num_rows {
            let list_val = list_vec.get_value(row).unwrap_or(Value::Null);
            let items = match list_val {
                Value::List(ref items) => items.clone(),
                _ => {
                    store_value_in_vector(&mut result, row, &Value::List(vec![]));
                    continue;
                }
            };

            let mut filtered: Vec<Value> = Vec::new();
            for item in &items {
                let mut elem_vec = ValueVector::new(item.physical_type(), 1);
                elem_vec.resize(1);
                store_value_in_vector(&mut elem_vec, 0, item);
                let mut mini_chunk = DataChunk::new(vec![elem_vec]);
                mini_chunk.field_names.push(var_name.clone());

                let pred_vec = self.evaluate(body, &mini_chunk)?;
                let pred_val = pred_vec.get_value(0).unwrap_or(Value::Null);

                if matches!(pred_val, Value::Bool(true)) {
                    filtered.push(item.clone());
                } else if let Value::Int64(x) = pred_val {
                    if x != 0 {
                        filtered.push(item.clone());
                    }
                }
            }
            store_value_in_vector(&mut result, row, &Value::List(filtered));
        }

        Ok(result)
    }

    /// Evaluate `list_reduce(list, (acc, x) -> body, initial)` — fold over list.
    fn evaluate_list_reduce(
        &self,
        args: &[Expression],
        lambda: &Expression,
        chunk: &DataChunk,
    ) -> Result<ValueVector, String> {
        let list_expr = args.iter().find(|a| !matches!(a, Expression::Lambda { .. }))
            .ok_or("list_reduce requires a list argument")?;

        // Find initial value — the argument that is not the list and not the lambda
        let initial_expr = args.iter()
            .filter(|a| !matches!(a, Expression::Lambda { .. }))
            .nth(1) // Second non-lambda arg (first is list)
            .ok_or("list_reduce requires an initial value argument")?;

        let (var_name, body) = match lambda {
            Expression::Lambda { var_name, body } => (var_name, body),
            _ => return Err("Expected lambda expression".into()),
        };

        // list_reduce uses (acc, x) -> expr where acc is first var, x is second
        let acc_name = var_name.clone();
        let elem_name = match body.as_ref() {
            Expression::BinaryOp(_op, left, right) => {
                // Try to infer the element variable from the body pattern
                // Most common: acc + x where x is a Variable
                let left_var = if let Expression::Variable(v) = left.as_ref() { Some(v.clone()) } else { None };
                let right_var = if let Expression::Variable(v) = right.as_ref() { Some(v.clone()) } else { None };

                if left_var.as_deref() == Some(&acc_name) {
                    right_var.unwrap_or_default()
                } else if right_var.as_deref() == Some(&acc_name) {
                    left_var.unwrap_or_default()
                } else {
                    String::new()
                }
            }
            _ => String::new(),
        };

        let list_vec = self.evaluate(list_expr, chunk)?;
        let initial_vec = self.evaluate(initial_expr, chunk)?;
        let num_rows = chunk.size;
        let mut result = ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, num_rows);
        result.resize(num_rows);

        for row in 0..num_rows {
            let list_val = list_vec.get_value(row).unwrap_or(Value::Null);
            let items = match list_val {
                Value::List(ref items) => items.clone(),
                _ => {
                    store_value_in_vector(&mut result, row, &Value::Null);
                    continue;
                }
            };

            let mut acc = initial_vec.get_value(row).unwrap_or(Value::Null);
            for item in &items {
                // Create mini-chunk with acc as field 0 and item as field 1
                let mut acc_vec = ValueVector::new(acc.physical_type(), 1);
                acc_vec.resize(1);
                store_value_in_vector(&mut acc_vec, 0, &acc);
                let mut elem_vec = ValueVector::new(item.physical_type(), 1);
                elem_vec.resize(1);
                store_value_in_vector(&mut elem_vec, 0, item);
                let mut mini_chunk = DataChunk::new(vec![acc_vec, elem_vec]);
                mini_chunk.field_names.push(acc_name.clone());
                if !elem_name.is_empty() {
                    mini_chunk.field_names.push(elem_name.clone());
                }

                let body_vec = self.evaluate(body, &mini_chunk)?;
                acc = body_vec.get_value(0).unwrap_or(Value::Null);
            }
            store_value_in_vector(&mut result, row, &acc);
        }

        Ok(result)
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
            let offset = row * 256;
            let bytes = s.as_bytes();
            let len = bytes.len().min(255) as u8;
            if offset < v.data().len() {
                v.data_mut()[offset] = len;
                let copy_len = bytes.len().min(255);
                if offset + 1 + copy_len <= v.data().len() {
                    v.data_mut()[offset + 1..offset + 1 + copy_len].copy_from_slice(&bytes[..copy_len]);
                }
                v.set_null(row, false);
            }
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
            let offset = row * 256;
            let bytes = s.as_bytes();
            let len = bytes.len().min(255) as u8;
            if offset < v.data().len() {
                v.data_mut()[offset] = len;
                let copy_len = bytes.len().min(255);
                if offset + 1 + copy_len <= v.data().len() {
                    v.data_mut()[offset + 1..offset + 1 + copy_len].copy_from_slice(&bytes[..copy_len]);
                }
                v.set_null(row, false);
            }
        }
        _ => {
            // For complex types (List, Struct, etc.), store as null
            v.set_null(row, true);
        }
    }
}

/// Build an ArrowVector from a Vec<Value>, using typed builders to
/// avoid the intermediate ValueVector allocation.
fn build_arrow_from_values(values: &[Value], phys_type: PhysicalTypeID, num_rows: usize) -> Result<ArrowVector, String> {
    match phys_type {
        PhysicalTypeID::Bool => {
            let mut builder = arrow::array::BooleanBuilder::with_capacity(num_rows);
            for v in values {
                match v {
                    Value::Null => builder.append_null(),
                    Value::Bool(b) => builder.append_value(*b),
                    _ => builder.append_null(),
                }
            }
            Ok(ArrowVector::new(Arc::new(builder.finish()), phys_type))
        }
        PhysicalTypeID::Int64 => {
            let mut builder = arrow::array::Int64Builder::with_capacity(num_rows);
            for v in values {
                match v {
                    Value::Null => builder.append_null(),
                    Value::Int64(n) => builder.append_value(*n),
                    _ => builder.append_null(),
                }
            }
            Ok(ArrowVector::new(Arc::new(builder.finish()), phys_type))
        }
        PhysicalTypeID::Int32 => {
            let mut builder = arrow::array::Int32Builder::with_capacity(num_rows);
            for v in values {
                match v {
                    Value::Null => builder.append_null(),
                    Value::Int32(n) => builder.append_value(*n),
                    _ => builder.append_null(),
                }
            }
            Ok(ArrowVector::new(Arc::new(builder.finish()), phys_type))
        }
        PhysicalTypeID::Double => {
            let mut builder = arrow::array::Float64Builder::with_capacity(num_rows);
            for v in values {
                match v {
                    Value::Null => builder.append_null(),
                    Value::Double(n) => builder.append_value(*n),
                    _ => builder.append_null(),
                }
            }
            Ok(ArrowVector::new(Arc::new(builder.finish()), phys_type))
        }
        PhysicalTypeID::Float => {
            let mut builder = arrow::array::Float32Builder::with_capacity(num_rows);
            for v in values {
                match v {
                    Value::Null => builder.append_null(),
                    Value::Float(n) => builder.append_value(*n),
                    Value::Double(n) => builder.append_value(*n as f32),
                    _ => builder.append_null(),
                }
            }
            Ok(ArrowVector::new(Arc::new(builder.finish()), phys_type))
        }
        PhysicalTypeID::String => {
            let mut builder = arrow::array::StringBuilder::with_capacity(num_rows, num_rows * 16);
            for v in values {
                match v {
                    Value::Null => builder.append_null(),
                    Value::String(s) => builder.append_value(s),
                    _ => builder.append_null(),
                }
            }
            Ok(ArrowVector::new(Arc::new(builder.finish()), phys_type))
        }
        _ => {
            // Unsupported type — fall back to creating an Arrow array with all nulls
            let mut builder = arrow::array::Int64Builder::with_capacity(num_rows);
            builder.append_nulls(num_rows);
            Ok(ArrowVector::new(Arc::new(builder.finish()), PhysicalTypeID::Int64))
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

        let seq_fn = self
            .sequence_fn
            .as_ref()
            .ok_or_else(|| format!("No sequence callback configured for '{}'", name))?;

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
    use hashbrown::HashMap;
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

    #[test]
    fn test_sequence_nextval_currval_with_callback() {
        let state = Arc::new(Mutex::new(HashMap::new()));
        state.lock().unwrap().insert("my_seq".to_string(), 10_i64);

        let state_for_fn = state.clone();
        let seq_fn: Arc<dyn Fn(&str, bool) -> Result<Value, String> + Send + Sync> =
            Arc::new(move |seq_name: &str, is_nextval: bool| {
                let mut map = state_for_fn.lock().map_err(|e| format!("Lock error: {e}"))?;
                let current = map
                    .get_mut(seq_name)
                    .ok_or_else(|| format!("Sequence '{}' not found", seq_name))?;
                if is_nextval {
                    let out = *current;
                    *current += 2;
                    Ok(Value::Int64(out))
                } else {
                    Ok(Value::Int64(*current))
                }
            });

        let eval = ExpressionEvaluator::new(make_registry()).with_sequence_fn(seq_fn);
        let chunk = make_chunk(&[1, 2, 3]);

        let nextval_expr = Expression::FunctionCall(
            "nextval".into(),
            vec![Expression::Constant(Constant::String("my_seq".into()))],
        );
        let nextvals = eval.evaluate(&nextval_expr, &chunk).unwrap();
        assert_eq!(nextvals.get_value(0), Some(Value::Int64(10)));
        assert_eq!(nextvals.get_value(1), Some(Value::Int64(12)));
        assert_eq!(nextvals.get_value(2), Some(Value::Int64(14)));

        let currval_expr = Expression::FunctionCall(
            "currval".into(),
            vec![Expression::Constant(Constant::String("my_seq".into()))],
        );
        let curr = eval.evaluate(&currval_expr, &make_chunk(&[1])).unwrap();
        assert_eq!(curr.get_value(0), Some(Value::Int64(16)));
    }

    #[test]
    fn test_sequence_requires_callback() {
        let eval = ExpressionEvaluator::new(make_registry());
        let expr = Expression::FunctionCall(
            "nextval".into(),
            vec![Expression::Constant(Constant::String("my_seq".into()))],
        );
        let err = eval.evaluate(&expr, &make_chunk(&[1])).unwrap_err();
        assert!(
            err.contains("No sequence callback configured"),
            "Unexpected error: {err}"
        );
    }

    #[test]
    fn test_sequence_requires_string_arg() {
        let seq_fn: Arc<dyn Fn(&str, bool) -> Result<Value, String> + Send + Sync> =
            Arc::new(|_seq_name: &str, _is_nextval: bool| Ok(Value::Int64(1)));
        let eval = ExpressionEvaluator::new(make_registry()).with_sequence_fn(seq_fn);

        let expr = Expression::FunctionCall("nextval".into(), vec![Expression::Constant(Constant::Integer(42))]);
        let err = eval.evaluate(&expr, &make_chunk(&[1])).unwrap_err();
        assert!(err.contains("requires a string argument"), "Unexpected error: {err}");
    }
}
