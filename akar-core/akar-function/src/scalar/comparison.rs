use crate::registry::*;
use akar_common::types::Value;

// ==================== Comparison ====================

pub(crate) fn evaluate_comparison(op: ComparisonOp, args: &[Value]) -> Result<Value, String> {
    if args.len() < 2 && !matches!(op, ComparisonOp::IsNull | ComparisonOp::IsNotNull) {
        return Err("Comparison requires 2 arguments".into());
    }

    match op {
        ComparisonOp::Eq => Ok(Value::Bool(args[0] == args[1])),
        ComparisonOp::NotEq => Ok(Value::Bool(args[0] != args[1])),
        ComparisonOp::Lt => Ok(Value::Bool(compare_values(&args[0], &args[1])?.is_lt())),
        ComparisonOp::Lte => Ok(Value::Bool(!compare_values(&args[0], &args[1])?.is_gt())),
        ComparisonOp::Gt => Ok(Value::Bool(compare_values(&args[0], &args[1])?.is_gt())),
        ComparisonOp::Gte => Ok(Value::Bool(!compare_values(&args[0], &args[1])?.is_lt())),
        ComparisonOp::IsNull => Ok(Value::Bool(matches!(args[0], Value::Null))),
        ComparisonOp::IsNotNull => Ok(Value::Bool(!matches!(args[0], Value::Null))),
    }
}

pub(crate) fn compare_values(a: &Value, b: &Value) -> Result<std::cmp::Ordering, String> {
    match (a, b) {
        (Value::Int64(x), Value::Int64(y)) => Ok(x.cmp(y)),
        (Value::Int32(x), Value::Int32(y)) => Ok(x.cmp(y)),
        (Value::Int16(x), Value::Int16(y)) => Ok(x.cmp(y)),
        (Value::Int8(x), Value::Int8(y)) => Ok(x.cmp(y)),
        (Value::UInt64(x), Value::UInt64(y)) => Ok(x.cmp(y)),
        (Value::UInt32(x), Value::UInt32(y)) => Ok(x.cmp(y)),
        (Value::UInt16(x), Value::UInt16(y)) => Ok(x.cmp(y)),
        (Value::UInt8(x), Value::UInt8(y)) => Ok(x.cmp(y)),
        (Value::Double(x), Value::Double(y)) => Ok(x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)),
        (Value::Float(x), Value::Float(y)) => Ok(x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)),
        (Value::String(x), Value::String(y)) => Ok(x.cmp(y)),
        (Value::Bool(x), Value::Bool(y)) => Ok(x.cmp(y)),
        (Value::Date(x), Value::Date(y)) => Ok(x.cmp(y)),
        (Value::Timestamp(x), Value::Timestamp(y)) => Ok(x.cmp(y)),
        // Cross-type numeric promotion (int → float)
        (Value::Int64(x), Value::Double(y)) => x
            .partial_cmp(&(*y as i64))
            .map(|o| o.reverse())
            .ok_or_else(|| "Cannot compare Int64 with Double".into()),
        (Value::Double(x), Value::Int64(y)) => x
            .partial_cmp(&(*y as f64))
            .ok_or_else(|| "Cannot compare Double with Int64".into()),
        _ => Err("Cannot compare types".into()),
    }
}
