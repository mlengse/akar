use crate::registry::*;
use crate::scalar::{evaluate_scalar, numeric_to_f64};
use arrow::array::{
    ArrayRef, Float32Array, Float32Builder, Float64Array, Float64Builder, Int16Array,
    Int16Builder, Int32Array, Int32Builder, Int64Array, Int64Builder, Int8Array, Int8Builder,
    PrimitiveArray, UInt16Array, UInt16Builder, UInt32Array, UInt32Builder, UInt64Array,
    UInt64Builder, UInt8Array, UInt8Builder,
};
use arrow::compute;
use arrow::datatypes::{
    Float32Type, Float64Type, Int16Type, Int32Type, Int64Type, Int8Type, UInt16Type, UInt32Type,
    UInt64Type, UInt8Type,
};
use kuzu_common::types::Value;
use std::sync::Arc;

// ==================== Aggregate ====================

/// State for aggregate function computation over Values.
#[derive(Debug, Clone)]
pub enum AggValueState {
    Count(u64),
    Sum(Value),
    Min(Value),
    Max(Value),
    Avg {
        sum: Value,
        count: u64,
    },
    Collect(Vec<Value>),
    StdDev {
        sum: f64,
        sum_sq: f64,
        count: u64,
    },
    Variance {
        sum: f64,
        sum_sq: f64,
        count: u64,
    },
    /// Percentile state — collects all non-null values for percentile computation.
    Percentile {
        values: Vec<f64>,
        percentile: f64,
    },
    /// COUNT_IF state — counts rows where the condition evaluated to TRUE.
    CountIf(u64),
    /// STRING_AGG state — collects string pieces to be concatenated.
    StringAgg {
        pieces: Vec<String>,
        delimiter: String,
    },
}

impl AggValueState {
    /// Create a new initial state for the given aggregate function.
    pub fn new(func: &AggregateFunction) -> Self {
        match func {
            AggregateFunction::Count | AggregateFunction::CountStar => AggValueState::Count(0),
            AggregateFunction::Sum => AggValueState::Sum(Value::Null),
            AggregateFunction::Min => AggValueState::Min(Value::Null),
            AggregateFunction::Max => AggValueState::Max(Value::Null),
            AggregateFunction::Avg => AggValueState::Avg {
                sum: Value::Int64(0),
                count: 0,
            },
            AggregateFunction::Collect => AggValueState::Collect(Vec::new()),
            AggregateFunction::StdDev => AggValueState::StdDev {
                sum: 0.0,
                sum_sq: 0.0,
                count: 0,
            },
            AggregateFunction::Variance => AggValueState::Variance {
                sum: 0.0,
                sum_sq: 0.0,
                count: 0,
            },
            AggregateFunction::PercentileDisc { percentile } => AggValueState::Percentile {
                values: Vec::new(),
                percentile: *percentile,
            },
            AggregateFunction::PercentileCont { percentile } => AggValueState::Percentile {
                values: Vec::new(),
                percentile: *percentile,
            },
            AggregateFunction::CountIf => AggValueState::CountIf(0),
            AggregateFunction::StringAgg { delimiter } => AggValueState::StringAgg {
                pieces: Vec::new(),
                delimiter: delimiter.clone(),
            },
        }
    }

    /// Update the state with a new input value.
    pub fn update(&mut self, val: &Value) {
        if matches!(val, Value::Null) {
            // Most aggregates skip NULLs (except COUNT which counts them)
            return;
        }
        match self {
            AggValueState::Count(n) => *n += 1,
            AggValueState::Sum(current) => {
                if matches!(current, Value::Null) {
                    *current = val.clone();
                } else {
                    *current = add_values_for_agg(current.clone(), val.clone());
                }
            }
            AggValueState::Min(current) => {
                if matches!(current, Value::Null) {
                    *current = val.clone();
                } else if let Ok(Value::Bool(true)) = evaluate_scalar(
                    &ScalarFunction::Comparison { op: ComparisonOp::Lt },
                    &[val.clone(), current.clone()],
                ) {
                    *current = val.clone();
                }
            }
            AggValueState::Max(current) => {
                if matches!(current, Value::Null) {
                    *current = val.clone();
                } else if let Ok(Value::Bool(true)) = evaluate_scalar(
                    &ScalarFunction::Comparison { op: ComparisonOp::Gt },
                    &[val.clone(), current.clone()],
                ) {
                    *current = val.clone();
                }
            }
            AggValueState::Avg { sum, count } => {
                if matches!(sum, Value::Int64(0)) {
                    *sum = val.clone();
                } else {
                    *sum = add_values_for_agg(sum.clone(), val.clone());
                }
                *count += 1;
            }
            AggValueState::Collect(items) => {
                items.push(val.clone());
            }
            AggValueState::StdDev { sum, sum_sq, count } | AggValueState::Variance { sum, sum_sq, count } => {
                let v = numeric_to_f64(val).unwrap_or(0.0);
                *sum += v;
                *sum_sq += v * v;
                *count += 1;
            }
            AggValueState::Percentile { values, .. } => {
                if let Ok(v) = numeric_to_f64(val) {
                    values.push(v);
                }
            }
            AggValueState::CountIf(n) => {
                // Only count if the condition value is TRUE (non-null and Bool(true))
                if matches!(val, Value::Bool(true)) {
                    *n += 1;
                }
                // NULL and false are ignored (not counted)
            }
            AggValueState::StringAgg { pieces, .. } => {
                match val {
                    Value::String(s) => pieces.push(s.clone()),
                    other => pieces.push(format!("{:?}", other)),
                }
            }
        }
    }

    /// Finalize the state into a Value.
    pub fn finalize(&self) -> Value {
        match self {
            AggValueState::Count(n) => Value::Int64(*n as i64),
            AggValueState::CountIf(n) => Value::Int64(*n as i64),
            AggValueState::Sum(v) => v.clone(),
            AggValueState::Min(v) => v.clone(),
            AggValueState::Max(v) => v.clone(),
            AggValueState::Avg { sum, count } => {
                if *count == 0 {
                    return Value::Null;
                }
                match sum {
                    Value::Int64(s) => Value::Double(*s as f64 / *count as f64),
                    Value::Double(s) => Value::Double(*s / *count as f64),
                    _ => sum.clone(),
                }
            }
            AggValueState::Collect(items) => Value::List(items.clone()),
            AggValueState::StdDev { sum, sum_sq, count } => {
                if *count == 0 {
                    return Value::Null;
                }
                let n = *count as f64;
                let variance = (sum_sq - (sum * sum) / n) / n;
                Value::Double(variance.sqrt())
            }
            AggValueState::Variance { sum, sum_sq, count } => {
                if *count == 0 {
                    return Value::Null;
                }
                let n = *count as f64;
                let variance = (sum_sq - (sum * sum) / n) / n;
                Value::Double(variance)
            }
            AggValueState::Percentile { values, percentile } => {
                if values.is_empty() {
                    return Value::Null;
                }
                let mut sorted = values.clone();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let n = sorted.len();
                let p = *percentile;
                // Discrete percentile: pick the value at ceil(p * n) - 1 index
                let idx = ((p * n as f64).ceil() as usize).saturating_sub(1).min(n - 1);
                Value::Double(sorted[idx])
            }
            AggValueState::StringAgg { pieces, delimiter } => {
                if pieces.is_empty() {
                    return Value::Null;
                }
                Value::String(pieces.join(delimiter))
            }
        }
    }

    /// Merge another AggValueState into this one (for parallel aggregation).
    pub fn merge(&mut self, other: &Self) {
        match (self, other) {
            (AggValueState::Count(a), AggValueState::Count(b)) => *a += b,
            (AggValueState::Sum(a), AggValueState::Sum(b)) => {
                if matches!(a, Value::Null) {
                    *a = b.clone();
                } else if !matches!(b, Value::Null) {
                    *a = add_values_for_agg(a.clone(), b.clone());
                }
            }
            (AggValueState::Min(a), AggValueState::Min(b)) => {
                if !matches!(b, Value::Null) {
                    if matches!(a, Value::Null) {
                        *a = b.clone();
                    } else if let Ok(Value::Bool(true)) = evaluate_scalar(
                        &ScalarFunction::Comparison { op: ComparisonOp::Lt },
                        &[b.clone(), a.clone()],
                    ) {
                        *a = b.clone();
                    }
                }
            }
            (AggValueState::Max(a), AggValueState::Max(b)) => {
                if !matches!(b, Value::Null) {
                    if matches!(a, Value::Null) {
                        *a = b.clone();
                    } else if let Ok(Value::Bool(true)) = evaluate_scalar(
                        &ScalarFunction::Comparison { op: ComparisonOp::Gt },
                        &[b.clone(), a.clone()],
                    ) {
                        *a = b.clone();
                    }
                }
            }
            (AggValueState::Avg { sum: s1, count: c1 }, AggValueState::Avg { sum: s2, count: c2 }) => {
                if *c2 > 0 {
                    if *c1 == 0 {
                        *s1 = s2.clone();
                    } else {
                        *s1 = add_values_for_agg(s1.clone(), s2.clone());
                    }
                    *c1 += c2;
                }
            }
            (AggValueState::Collect(a), AggValueState::Collect(b)) => a.extend(b.iter().cloned()),
            (
                AggValueState::StdDev {
                    sum: s1,
                    sum_sq: sq1,
                    count: c1,
                },
                AggValueState::StdDev {
                    sum: s2,
                    sum_sq: sq2,
                    count: c2,
                },
            ) => {
                *s1 += s2;
                *sq1 += sq2;
                *c1 += c2;
            }
            (
                AggValueState::Variance {
                    sum: s1,
                    sum_sq: sq1,
                    count: c1,
                },
                AggValueState::Variance {
                    sum: s2,
                    sum_sq: sq2,
                    count: c2,
                },
            ) => {
                *s1 += s2;
                *sq1 += sq2;
                *c1 += c2;
            }
            (AggValueState::Percentile { values: a_vals, .. }, AggValueState::Percentile { values: b_vals, .. }) => {
                a_vals.extend_from_slice(b_vals);
            }
            (AggValueState::CountIf(a), AggValueState::CountIf(b)) => *a += b,
            (AggValueState::StringAgg { pieces: a_pieces, .. }, AggValueState::StringAgg { pieces: b_pieces, .. }) => {
                a_pieces.extend_from_slice(b_pieces);
            }
            _ => {}
        }
    }
}

/// Add two Values for aggregate summation (numeric promotion).
fn add_values_for_agg(a: Value, b: Value) -> Value {
    match (&a, &b) {
        (Value::Int64(x), Value::Int64(y)) => Value::Int64(x + y),
        (Value::Double(x), Value::Double(y)) => Value::Double(x + y),
        (Value::Int64(x), Value::Double(y)) => Value::Double(*x as f64 + y),
        (Value::Double(x), Value::Int64(y)) => Value::Double(x + *y as f64),
        _ => a, // fallback
    }
}

/// Convert a slice of Values to an Arrow ArrayRef for numeric types.
/// Returns None if values are not numeric or have mixed incompatible types.
fn values_to_arrow_array(values: &[Value], first: &Value) -> Option<ArrayRef> {
    match first {
        Value::Int64(_) => {
            let mut builder = Int64Builder::new();
            for v in values {
                match v {
                    Value::Int64(n) => builder.append_value(*n),
                    Value::Null => builder.append_null(),
                    _ => return None,
                }
            }
            Some(Arc::new(builder.finish()))
        }
        Value::Double(_) => {
            let mut builder = Float64Builder::new();
            for v in values {
                match v {
                    Value::Double(n) => builder.append_value(*n),
                    Value::Null => builder.append_null(),
                    _ => return None,
                }
            }
            Some(Arc::new(builder.finish()))
        }
        Value::Int32(_) => {
            let mut builder = Int32Builder::new();
            for v in values {
                match v {
                    Value::Int32(n) => builder.append_value(*n),
                    Value::Null => builder.append_null(),
                    _ => return None,
                }
            }
            Some(Arc::new(builder.finish()))
        }
        Value::Float(_) => {
            let mut builder = Float32Builder::new();
            for v in values {
                match v {
                    Value::Float(n) => builder.append_value(*n),
                    Value::Null => builder.append_null(),
                    _ => return None,
                }
            }
            Some(Arc::new(builder.finish()))
        }
        Value::Int16(_) => {
            let mut builder = Int16Builder::new();
            for v in values {
                match v {
                    Value::Int16(n) => builder.append_value(*n),
                    Value::Null => builder.append_null(),
                    _ => return None,
                }
            }
            Some(Arc::new(builder.finish()))
        }
        Value::Int8(_) => {
            let mut builder = Int8Builder::new();
            for v in values {
                match v {
                    Value::Int8(n) => builder.append_value(*n),
                    Value::Null => builder.append_null(),
                    _ => return None,
                }
            }
            Some(Arc::new(builder.finish()))
        }
        Value::UInt64(_) => {
            let mut builder = UInt64Builder::new();
            for v in values {
                match v {
                    Value::UInt64(n) => builder.append_value(*n),
                    Value::Null => builder.append_null(),
                    _ => return None,
                }
            }
            Some(Arc::new(builder.finish()))
        }
        Value::UInt32(_) => {
            let mut builder = UInt32Builder::new();
            for v in values {
                match v {
                    Value::UInt32(n) => builder.append_value(*n),
                    Value::Null => builder.append_null(),
                    _ => return None,
                }
            }
            Some(Arc::new(builder.finish()))
        }
        Value::UInt16(_) => {
            let mut builder = UInt16Builder::new();
            for v in values {
                match v {
                    Value::UInt16(n) => builder.append_value(*n),
                    Value::Null => builder.append_null(),
                    _ => return None,
                }
            }
            Some(Arc::new(builder.finish()))
        }
        Value::UInt8(_) => {
            let mut builder = UInt8Builder::new();
            for v in values {
                match v {
                    Value::UInt8(n) => builder.append_value(*n),
                    Value::Null => builder.append_null(),
                    _ => return None,
                }
            }
            Some(Arc::new(builder.finish()))
        }
        _ => None,
    }
}

/// Try to evaluate aggregate using Arrow compute SIMD kernels.
/// Returns None if the aggregate type or column type doesn't support SIMD.
fn try_simd_aggregate(func: &AggregateFunction, args: &[Value]) -> Option<Value> {
    match func {
        AggregateFunction::Sum | AggregateFunction::Min | AggregateFunction::Max => {}
        _ => return None,
    }

    if args.is_empty() {
        return None;
    }

    let first = args.iter().find(|v| !matches!(v, Value::Null))?;
    let array = values_to_arrow_array(args, first)?;

    macro_rules! simd_dispatch {
        ($arr:expr, $func:expr, $ty:ty, $variant:ident) => {{
            let typed: &PrimitiveArray<$ty> = $arr.as_any().downcast_ref()?;
            match $func {
                AggregateFunction::Sum => compute::sum(typed).map(Value::$variant),
                AggregateFunction::Min => compute::min(typed).map(Value::$variant),
                AggregateFunction::Max => compute::max(typed).map(Value::$variant),
                _ => return None,
            }
        }};
    }

    match first {
        Value::Int64(_) => simd_dispatch!(array, func, Int64Type, Int64),
        Value::Double(_) => simd_dispatch!(array, func, Float64Type, Double),
        Value::Int32(_) => simd_dispatch!(array, func, Int32Type, Int32),
        Value::Float(_) => simd_dispatch!(array, func, Float32Type, Float),
        Value::Int16(_) => simd_dispatch!(array, func, Int16Type, Int16),
        Value::Int8(_) => simd_dispatch!(array, func, Int8Type, Int8),
        Value::UInt64(_) => simd_dispatch!(array, func, UInt64Type, UInt64),
        Value::UInt32(_) => simd_dispatch!(array, func, UInt32Type, UInt32),
        Value::UInt16(_) => simd_dispatch!(array, func, UInt16Type, UInt16),
        Value::UInt8(_) => simd_dispatch!(array, func, UInt8Type, UInt8),
        _ => None,
    }
}

/// Evaluate an aggregate function across a slice of Values.
/// Returns the final aggregate value.
pub fn evaluate_aggregate(func: &AggregateFunction, args: &[Value]) -> Result<Value, String> {
    // Try SIMD-accelerated path for numeric aggregates (Sum, Min, Max)
    if let Some(result) = try_simd_aggregate(func, args) {
        return Ok(result);
    }

    let mut state = AggValueState::new(func);

    // COUNT(*) counts all rows regardless of arguments
    if matches!(func, AggregateFunction::CountStar) {
        if let AggValueState::Count(n) = &mut state {
            *n = args.len() as u64;
        }
        return Ok(state.finalize());
    }

    for arg in args {
        state.update(arg);
    }

    Ok(state.finalize())
}
