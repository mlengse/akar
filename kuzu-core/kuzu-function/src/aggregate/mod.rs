use crate::registry::*;
use crate::scalar::{evaluate_scalar, numeric_to_f64};
use kuzu_common::types::Value;

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
            (AggValueState::Percentile { values: a, .. }, AggValueState::Percentile { values: b, .. }) => {
                a.extend(b.iter().cloned());
            }
            (AggValueState::CountIf(a), AggValueState::CountIf(b)) => *a += b,
            _ => { /* type mismatch — should not happen in practice */ }
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

/// Evaluate an aggregate function across a slice of Values.
/// Returns the final aggregate value.
pub fn evaluate_aggregate(func: &AggregateFunction, args: &[Value]) -> Result<Value, String> {
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
