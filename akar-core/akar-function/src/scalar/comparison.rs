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
/// promotion instead. Floats follow the NaN convention: NaN = NaN is true.
fn values_equal(a: &Value, b: &Value) -> bool {
    if a == b {
        return true;
    }
    if let (Value::Double(x), Value::Double(y)) = (a, b) {
        return x.is_nan() && y.is_nan();
    }
    if let (Value::Float(x), Value::Float(y)) = (a, b) {
        return x.is_nan() && y.is_nan();
    }
    if let (Some(x), Some(y)) = (integer_to_i128(a), integer_to_i128(b)) {
        return x == y;
    }
    // Cross-type float promotion: compare a float against any numeric via f64
    // (e.g. a FLOAT column value against an integer/Double literal).
    if let (Ok(x), Ok(y)) = (super::arithmetic::numeric_to_f64(a), super::arithmetic::numeric_to_f64(b)) {
        return x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan());
    }
    false
}

/// Total-order comparison for floats with a NaN convention: NaN sorts greater
/// than every finite value, and NaN == NaN.
#[inline]
pub(crate) fn double_cmp(a: f64, b: f64) -> std::cmp::Ordering {
    if a.is_nan() {
        if b.is_nan() {
            std::cmp::Ordering::Equal
        } else {
            std::cmp::Ordering::Greater
        }
    } else if b.is_nan() {
        std::cmp::Ordering::Less
    } else {
        a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
    }
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
        (Value::Double(x), Value::Double(y)) => Ok(double_cmp(*x, *y)),
        (Value::Float(x), Value::Float(y)) => Ok(double_cmp(*x as f64, *y as f64)),
        (Value::String(x), Value::String(y)) => Ok(x.cmp(y)),
        (Value::Bool(x), Value::Bool(y)) => Ok(x.cmp(y)),
        (Value::Date(x), Value::Date(y)) => Ok(x.cmp(y)),
        (Value::Timestamp(x), Value::Timestamp(y)) => Ok(x.cmp(y)),
        // Cross-type numeric promotion (int ↔ float)
        (Value::Int64(x), Value::Double(y)) => Ok(double_cmp(*x as f64, *y)),
        (Value::Double(x), Value::Int64(y)) => Ok(double_cmp(*x, *y as f64)),
        (Value::Float(x), Value::Double(y)) => Ok(double_cmp(*x as f64, *y)),
        (Value::Double(x), Value::Float(y)) => Ok(double_cmp(*x, *y as f64)),
        (Value::Float(x), Value::Int64(y)) => Ok(double_cmp(*x as f64, *y as f64)),
        (Value::Int64(x), Value::Float(y)) => Ok(double_cmp(*x as f64, *y as f64)),
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
