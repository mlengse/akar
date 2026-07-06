use crate::registry::*;
use kuzu_common::types::Value;

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
        (Value::Double(x), Value::Double(y)) => Ok(x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)),
        (Value::String(x), Value::String(y)) => Ok(x.cmp(y)),
        (Value::Bool(x), Value::Bool(y)) => Ok(x.cmp(y)),
        _ => Err("Cannot compare types".into()),
    }
}
