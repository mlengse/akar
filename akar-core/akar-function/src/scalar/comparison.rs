use crate::registry::*;
use akar_common::types::Value;

// ==================== Comparison ====================

pub(crate) fn evaluate_comparison(op: ComparisonOp, args: &[Value]) -> Result<Value, String> {
    if args.len() < 2 && !matches!(op, ComparisonOp::IsNull | ComparisonOp::IsNotNull) {
        return Err("Comparison requires 2 arguments".into());
    }

    match op {
        ComparisonOp::Eq => Ok(Value::Bool(values_equal(&args[0], &args[1]))),
        ComparisonOp::NotEq => Ok(Value::Bool(!values_equal(&args[0], &args[1]))),
        ComparisonOp::Lt => Ok(Value::Bool(compare_values(&args[0], &args[1])?.is_lt())),
        ComparisonOp::Lte => Ok(Value::Bool(!compare_values(&args[0], &args[1])?.is_gt())),
        ComparisonOp::Gt => Ok(Value::Bool(compare_values(&args[0], &args[1])?.is_gt())),
        ComparisonOp::Gte => Ok(Value::Bool(!compare_values(&args[0], &args[1])?.is_lt())),
        ComparisonOp::IsNull => Ok(Value::Bool(matches!(args[0], Value::Null))),
        ComparisonOp::IsNotNull => Ok(Value::Bool(!matches!(args[0], Value::Null))),
    }
}

/// Exact cross-type numeric equality.
///
/// `Value::UInt64` and `Value::Int64` derive-distinct `PartialEq` instances
/// (e.g. `UInt64(5) == Int64(5)` is `false`), so `WHERE uint64_col = 5` would
/// silently return zero rows. Mixed integer operands are compared via `i128`
/// promotion instead.
fn values_equal(a: &Value, b: &Value) -> bool {
    if a == b {
        return true;
    }
    if let (Some(x), Some(y)) = (integer_to_i128(a), integer_to_i128(b)) {
        return x == y;
    }
    false
}

/// Widened representation of any integer `Value` variant (exact, no overflow).
fn integer_to_i128(v: &Value) -> Option<i128> {
    match v {
        Value::Int64(x) => Some(*x as i128),
        Value::Int32(x) => Some(*x as i128),
        Value::Int16(x) => Some(*x as i128),
        Value::Int8(x) => Some(*x as i128),
        Value::UInt64(x) => Some(*x as i128),
        Value::UInt32(x) => Some(*x as i128),
        Value::UInt16(x) => Some(*x as i128),
        Value::UInt8(x) => Some(*x as i128),
        _ => None,
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
        // Mixed signed/unsigned integer promotion (exact via i128). A UInt64
        // column compared against an Int64 literal (e.g. `WHERE id > 5`) would
        // otherwise hit the generic "Cannot compare types" error below.
        _ => {
            if let (Some(x), Some(y)) = (integer_to_i128(a), integer_to_i128(b)) {
                Ok(x.cmp(&y))
            } else {
                Err("Cannot compare types".into())
            }
        }
    }
}
