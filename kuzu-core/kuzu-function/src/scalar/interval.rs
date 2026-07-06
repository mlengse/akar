use crate::registry::*;
use kuzu_common::types::{Interval, Value};

// ==================== Interval constructor functions ====================

/// Evaluate an interval constructor function.
/// Each takes a single INT64 argument and returns an INTERVAL value.
pub(crate) fn evaluate_interval(op: IntervalOp, args: &[Value]) -> Result<Value, String> {
    let n = match &args[0] {
        Value::Int64(x) => *x,
        _ => return Err("Interval functions require integer argument".into()),
    };
    let interval = match op {
        IntervalOp::ToYears => Interval::new((n * 12) as i32, 0, 0),
        IntervalOp::ToMonths => Interval::new(n as i32, 0, 0),
        IntervalOp::ToDays => Interval::new(0, n as i32, 0),
        IntervalOp::ToHours => Interval::new(0, 0, n * 3_600_000_000),
        IntervalOp::ToMinutes => Interval::new(0, 0, n * 60_000_000),
        IntervalOp::ToSeconds => Interval::new(0, 0, n * 1_000_000),
        IntervalOp::ToMilliseconds => Interval::new(0, 0, n * 1000),
        IntervalOp::ToMicroseconds => Interval::new(0, 0, n),
    };
    Ok(Value::Interval(interval))
}
