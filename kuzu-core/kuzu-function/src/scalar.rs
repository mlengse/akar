//! Scalar function evaluation.
//!
//! Each function takes input `Value` slices and produces an output `Value`.

use crate::registry::*;
use kuzu_common::types::Value;

/// Evaluate a scalar function with the given arguments.
pub fn evaluate_scalar(func: &ScalarFunction, args: &[Value]) -> Result<Value, String> {
    match func {
        ScalarFunction::Arithmetic { op } => evaluate_arithmetic(*op, args),
        ScalarFunction::Comparison { op } => evaluate_comparison(*op, args),
        ScalarFunction::String { op } => evaluate_string(*op, args),
        ScalarFunction::Cast { target_type } => evaluate_cast(*target_type, args),
        ScalarFunction::Date { op } => evaluate_date(*op, args),
        ScalarFunction::List { op } => evaluate_list(*op, args),
        ScalarFunction::Map { op } => evaluate_map(*op, args),
        ScalarFunction::Struct { op } => evaluate_struct(*op, args),
        ScalarFunction::Boolean { op } => evaluate_boolean(*op, args),
        ScalarFunction::Utility { op } => evaluate_utility(*op, args),
    }
}

// ==================== Arithmetic ====================

fn evaluate_arithmetic(op: ArithmeticOp, args: &[Value]) -> Result<Value, String> {
    if args.is_empty() {
        return Err("Arithmetic requires at least one argument".into());
    }

    match op {
        ArithmeticOp::Negate => {
            let v = args[0].clone();
            match v {
                Value::Int64(x) => Ok(Value::Int64(-x)),
                Value::Double(x) => Ok(Value::Double(-x)),
                _ => Err("Cannot negate non-numeric".into()),
            }
        }
        ArithmeticOp::Abs => {
            let v = args[0].clone();
            match v {
                Value::Int64(x) => Ok(Value::Int64(x.checked_abs().unwrap_or(i64::MAX))),
                Value::Double(x) => Ok(Value::Double(x.abs())),
                _ => Err("Abs requires numeric".into()),
            }
        }
        ArithmeticOp::Ceil => match args[0] {
            Value::Double(x) => Ok(Value::Double(x.ceil())),
            Value::Int64(x) => Ok(Value::Double((x as f64).ceil())),
            _ => Err("Ceil requires numeric".into()),
        },
        ArithmeticOp::Floor => match args[0] {
            Value::Double(x) => Ok(Value::Double(x.floor())),
            Value::Int64(x) => Ok(Value::Double((x as f64).floor())),
            _ => Err("Floor requires numeric".into()),
        },
        ArithmeticOp::Round => match args[0] {
            Value::Double(x) => Ok(Value::Double(x.round())),
            _ => Err("Round requires numeric".into()),
        },
        ArithmeticOp::Sqrt => {
            let v = numeric_to_f64(&args[0])?;
            Ok(Value::Double(v.sqrt()))
        }
        ArithmeticOp::Log => {
            let v = numeric_to_f64(&args[0])?;
            Ok(Value::Double(v.ln()))
        }
        ArithmeticOp::Exp => {
            let v = numeric_to_f64(&args[0])?;
            Ok(Value::Double(v.exp()))
        }
        ArithmeticOp::Power => {
            if args.len() < 2 {
                return Err("Power requires 2 arguments".into());
            }
            let base = numeric_to_f64(&args[0])?;
            let exp = numeric_to_f64(&args[1])?;
            Ok(Value::Double(base.powf(exp)))
        }
        // Binary arithmetic ops
        _ => {
            if args.len() < 2 {
                return Err("Binary arithmetic requires 2 arguments".into());
            }
            let a = args[0].clone();
            let b = args[1].clone();
            match op {
                ArithmeticOp::Add => add_values(a, b),
                ArithmeticOp::Sub => sub_values(a, b),
                ArithmeticOp::Mul => mul_values(a, b),
                ArithmeticOp::Div => div_values(a, b),
                ArithmeticOp::Mod => mod_values(a, b),
                _ => Err(format!("Unimplemented arithmetic op: {:?}", op)),
            }
        }
    }
}

fn numeric_to_f64(v: &Value) -> Result<f64, String> {
    match v {
        Value::Int64(x) => Ok(*x as f64),
        Value::Double(x) => Ok(*x),
        Value::Float(x) => Ok(*x as f64),
        Value::Int32(x) => Ok(*x as f64),
        _ => Err("Expected numeric value".into()),
    }
}

fn add_values(a: Value, b: Value) -> Result<Value, String> {
    match (&a, &b) {
        (Value::Int64(x), Value::Int64(y)) => Ok(Value::Int64(x + y)),
        (Value::Double(x), Value::Double(y)) => Ok(Value::Double(x + y)),
        (Value::Int64(x), Value::Double(y)) => Ok(Value::Double(*x as f64 + y)),
        (Value::Double(x), Value::Int64(y)) => Ok(Value::Double(x + *y as f64)),
        (Value::String(x), Value::String(y)) => Ok(Value::String(format!("{}{}", x, y))),
        _ => Err(format!("Cannot add {:?} and {:?}", a.logical_type(), b.logical_type())),
    }
}

fn sub_values(a: Value, b: Value) -> Result<Value, String> {
    match (&a, &b) {
        (Value::Int64(x), Value::Int64(y)) => Ok(Value::Int64(x - y)),
        (Value::Double(x), Value::Double(y)) => Ok(Value::Double(x - y)),
        _ => Err("Cannot subtract non-numeric".into()),
    }
}

fn mul_values(a: Value, b: Value) -> Result<Value, String> {
    match (&a, &b) {
        (Value::Int64(x), Value::Int64(y)) => Ok(Value::Int64(x * y)),
        (Value::Double(x), Value::Double(y)) => Ok(Value::Double(x * y)),
        _ => Err("Cannot multiply non-numeric".into()),
    }
}

fn div_values(a: Value, b: Value) -> Result<Value, String> {
    match (&a, &b) {
        (Value::Int64(x), Value::Int64(y)) => {
            if *y == 0 {
                return Err("Division by zero".into());
            }
            Ok(Value::Int64(x / y))
        }
        (Value::Double(x), Value::Double(y)) => {
            if *y == 0.0 {
                return Err("Division by zero".into());
            }
            Ok(Value::Double(x / y))
        }
        _ => Err("Cannot divide non-numeric".into()),
    }
}

fn mod_values(a: Value, b: Value) -> Result<Value, String> {
    match (&a, &b) {
        (Value::Int64(x), Value::Int64(y)) => {
            if *y == 0 {
                return Err("Modulo by zero".into());
            }
            Ok(Value::Int64(x % y))
        }
        _ => Err("Cannot modulo non-integer".into()),
    }
}

// ==================== Comparison ====================

fn evaluate_comparison(op: ComparisonOp, args: &[Value]) -> Result<Value, String> {
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

fn compare_values(a: &Value, b: &Value) -> Result<std::cmp::Ordering, String> {
    match (a, b) {
        (Value::Int64(x), Value::Int64(y)) => Ok(x.cmp(y)),
        (Value::Double(x), Value::Double(y)) => Ok(x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)),
        (Value::String(x), Value::String(y)) => Ok(x.cmp(y)),
        (Value::Bool(x), Value::Bool(y)) => Ok(x.cmp(y)),
        _ => Err("Cannot compare types".into()),
    }
}

// ==================== String ====================

fn evaluate_string(op: StringOp, args: &[Value]) -> Result<Value, String> {
    if args.is_empty() {
        return Err("String function requires arguments".into());
    }

    match op {
        StringOp::Concat => {
            let s: String = args.iter().map(|v| match v {
                Value::String(s) => s.clone(),
                Value::Null => "NULL".into(),
                other => format!("{:?}", other),
            }).collect();
            Ok(Value::String(s))
        }
        StringOp::Contains => {
            let s = get_string(&args[0])?;
            let pat = get_string(&args[1])?;
            Ok(Value::Bool(s.contains(&pat)))
        }
        StringOp::StartsWith => {
            let s = get_string(&args[0])?;
            let pat = get_string(&args[1])?;
            Ok(Value::Bool(s.starts_with(&pat)))
        }
        StringOp::EndsWith => {
            let s = get_string(&args[0])?;
            let pat = get_string(&args[1])?;
            Ok(Value::Bool(s.ends_with(&pat)))
        }
        StringOp::ToUpper => {
            let s = get_string(&args[0])?;
            Ok(Value::String(s.to_uppercase()))
        }
        StringOp::ToLower => {
            let s = get_string(&args[0])?;
            Ok(Value::String(s.to_lowercase()))
        }
        StringOp::Trim => {
            let s = get_string(&args[0])?;
            Ok(Value::String(s.trim().to_string()))
        }
        StringOp::LTrim => {
            let s = get_string(&args[0])?;
            Ok(Value::String(s.trim_start().to_string()))
        }
        StringOp::RTrim => {
            let s = get_string(&args[0])?;
            Ok(Value::String(s.trim_end().to_string()))
        }
        StringOp::Length => {
            let s = get_string(&args[0])?;
            Ok(Value::Int64(s.len() as i64))
        }
        StringOp::Reverse => {
            let s = get_string(&args[0])?;
            Ok(Value::String(s.chars().rev().collect()))
        }
        StringOp::Repeat => {
            let s = get_string(&args[0])?;
            let n = match &args[1] { Value::Int64(x) => *x as usize, _ => return Err("Repeat count must be integer".into()) };
            Ok(Value::String(s.repeat(n)))
        }
        StringOp::Replace => {
            let s = get_string(&args[0])?;
            let from = get_string(&args[1])?;
            let to = get_string(&args[2])?;
            Ok(Value::String(s.replace(&from, &to)))
        }
        StringOp::Substring => {
            let s = get_string(&args[0])?;
            let start = match &args[1] { Value::Int64(x) => *x as usize, _ => return Err("Start must be integer".into()) };
            let len = if args.len() > 2 {
                match &args[2] { Value::Int64(x) => Some(*x as usize), _ => None }
            } else { None };
            let result = match len {
                Some(l) => s.chars().skip(start).take(l).collect(),
                None => s.chars().skip(start).collect(),
            };
            Ok(Value::String(result))
        }
        StringOp::RegexMatches => {
            let s = get_string(&args[0])?;
            let pat = get_string(&args[1])?;
            let re = regex::Regex::new(&pat).map_err(|e| format!("Regex error: {e}"))?;
            Ok(Value::Bool(re.is_match(&s)))
        }
        StringOp::RegexReplace => {
            let s = get_string(&args[0])?;
            let pat = get_string(&args[1])?;
            let repl = get_string(&args[2])?;
            let re = regex::Regex::new(&pat).map_err(|e| format!("Regex error: {e}"))?;
            Ok(Value::String(re.replace_all(&s, repl).to_string()))
        }
    }
}

fn get_string(v: &Value) -> Result<String, String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Null => Ok("NULL".into()),
        _ => Err(format!("Expected string, got {:?}", v.logical_type())),
    }
}

// ==================== Date ====================

fn evaluate_date(op: DateOp, args: &[Value]) -> Result<Value, String> {
    match op {
        DateOp::CurrentDate => {
            use std::time::{SystemTime, UNIX_EPOCH};
            let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
            let days = (secs / 86400) as i32;
            Ok(Value::Date(kuzu_common::types::Date(days)))
        }
        DateOp::CurrentTimestamp => {
            use std::time::{SystemTime, UNIX_EPOCH};
            let micros = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_micros() as i64;
            Ok(Value::Timestamp(kuzu_common::types::Timestamp(micros)))
        }
        DateOp::Year | DateOp::Month | DateOp::Day => {
            // Simplified: extract from Date value
            match &args[0] {
                Value::Date(d) => {
                    let days = d.0;
                    // Approximate year/month/day from days since epoch
                    let year = 1970 + (days as f64 / 365.25) as i32;
                    match op {
                        DateOp::Year => Ok(Value::Int64(year as i64)),
                        DateOp::Month => Ok(Value::Int64(1)),  // simplified
                        DateOp::Day => Ok(Value::Int64(1)),     // simplified
                        _ => unreachable!(),
                    }
                }
                _ => Err("Date expected".into()),
            }
        }
        _ => Err(format!("Date op {:?} not yet implemented", op)),
    }
}

// ==================== List ====================

fn evaluate_list(op: ListOp, args: &[Value]) -> Result<Value, String> {
    match op {
        ListOp::Len => match &args[0] {
            Value::List(items) => Ok(Value::Int64(items.len() as i64)),
            _ => Err("Expected list".into()),
        },
        ListOp::Extract => {
            let list = match &args[0] { Value::List(items) => items, _ => return Err("Expected list".into()) };
            let idx = match &args[1] { Value::Int64(i) => *i as usize, _ => return Err("Index must be integer".into()) };
            list.get(idx).cloned().ok_or_else(|| format!("Index {idx} out of bounds"))
        }
        ListOp::Contains => {
            let list = match &args[0] { Value::List(items) => items, _ => return Err("Expected list".into()) };
            Ok(Value::Bool(list.contains(&args[1])))
        }
        ListOp::Append => {
            let mut list = match args[0].clone() { Value::List(items) => items, _ => return Err("Expected list".into()) };
            list.push(args[1].clone());
            Ok(Value::List(list))
        }
        ListOp::Prepend => {
            let mut list = match args[0].clone() { Value::List(items) => items, _ => return Err("Expected list".into()) };
            list.insert(0, args[1].clone());
            Ok(Value::List(list))
        }
        ListOp::Reverse => {
            let mut list = match args[0].clone() { Value::List(items) => items, _ => return Err("Expected list".into()) };
            list.reverse();
            Ok(Value::List(list))
        }
        _ => Err(format!("List op {:?} not yet implemented", op)),
    }
}

// ==================== Map & Struct ====================

fn evaluate_map(op: MapOp, args: &[Value]) -> Result<Value, String> {
    match op {
        MapOp::Keys => match &args[0] {
            Value::Struct(entries) => Ok(Value::List(entries.iter().map(|(k, _)| Value::String(k.clone())).collect())),
            _ => Err("Expected map/struct".into()),
        },
        MapOp::Values => match &args[0] {
            Value::Struct(entries) => Ok(Value::List(entries.iter().map(|(_, v)| v.clone()).collect())),
            _ => Err("Expected map/struct".into()),
        },
        _ => Err(format!("Map op {:?} not yet implemented", op)),
    }
}

fn evaluate_struct(op: StructOp, args: &[Value]) -> Result<Value, String> {
    match op {
        StructOp::Extract => {
            let struct_val = &args[0];
            let key = get_string(&args[1])?;
            match struct_val {
                Value::Struct(entries) => {
                    for (k, v) in entries {
                        if *k == key { return Ok(v.clone()); }
                    }
                    Err(format!("Key '{}' not found in struct", key))
                }
                _ => Err("Expected struct".into()),
            }
        }
        _ => Err("Struct op not implemented".into()),
    }
}

// ==================== Boolean ====================

fn evaluate_boolean(op: BooleanOp, args: &[Value]) -> Result<Value, String> {
    if args.len() < 2 && !matches!(op, BooleanOp::Not) {
        return Err("Boolean op requires 2 arguments".into());
    }
    let a = match &args[0] { Value::Bool(b) => *b, _ => return Err("Expected boolean".into()) };
    match op {
        BooleanOp::And => {
            let b = match &args[1] { Value::Bool(x) => *x, _ => return Err("Expected boolean".into()) };
            Ok(Value::Bool(a && b))
        }
        BooleanOp::Or => {
            let b = match &args[1] { Value::Bool(x) => *x, _ => return Err("Expected boolean".into()) };
            Ok(Value::Bool(a || b))
        }
        BooleanOp::Xor => {
            let b = match &args[1] { Value::Bool(x) => *x, _ => return Err("Expected boolean".into()) };
            Ok(Value::Bool(a ^ b))
        }
        BooleanOp::Not => Ok(Value::Bool(!a)),
    }
}

// ==================== Cast ====================

fn evaluate_cast(target: CastTarget, args: &[Value]) -> Result<Value, String> {
    if args.is_empty() { return Err("Cast requires an argument".into()); }
    let v = &args[0];

    match target {
        CastTarget::String => Ok(Value::String(format!("{:?}", v))),
        CastTarget::Int64 => match v {
            Value::Int64(x) => Ok(Value::Int64(*x)),
            Value::Int32(x) => Ok(Value::Int64(*x as i64)),
            Value::Double(x) => Ok(Value::Int64(*x as i64)),
            Value::String(s) => s.parse::<i64>().map(Value::Int64).map_err(|e| format!("Cannot cast string to int: {e}")),
            _ => Err("Cannot cast to Int64".into()),
        },
        CastTarget::Double => match v {
            Value::Int64(x) => Ok(Value::Double(*x as f64)),
            Value::Double(x) => Ok(Value::Double(*x)),
            Value::String(s) => s.parse::<f64>().map(Value::Double).map_err(|e| format!("Cannot cast string to double: {e}")),
            _ => Err("Cannot cast to Double".into()),
        },
        CastTarget::Bool => match v {
            Value::Bool(x) => Ok(Value::Bool(*x)),
            Value::Int64(x) => Ok(Value::Bool(*x != 0)),
            _ => Err("Cannot cast to Bool".into()),
        },
        _ => Err(format!("Cast to {:?} not implemented", target)),
    }
}

// ==================== Utility ====================

fn evaluate_utility(op: UtilityOp, args: &[Value]) -> Result<Value, String> {
    match op {
        UtilityOp::Coalesce | UtilityOp::IfNull => {
            for arg in args {
                if !matches!(arg, Value::Null) {
                    return Ok(arg.clone());
                }
            }
            Ok(Value::Null)
        }
        UtilityOp::TypeOf => {
            if args.is_empty() { return Ok(Value::String("NULL".into())); }
            Ok(Value::String(format!("{:?}", args[0].logical_type())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Add };
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(3), Value::Int64(4)]).unwrap(), Value::Int64(7));
        assert_eq!(evaluate_scalar(&func, &[Value::Double(1.5), Value::Double(2.5)]).unwrap(), Value::Double(4.0));
        assert_eq!(evaluate_scalar(&func, &[Value::String("a".into()), Value::String("b".into())]).unwrap(), Value::String("ab".into()));
    }

    #[test]
    fn test_sub() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Sub };
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(10), Value::Int64(3)]).unwrap(), Value::Int64(7));
    }

    #[test]
    fn test_mul() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Mul };
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(5), Value::Int64(6)]).unwrap(), Value::Int64(30));
    }

    #[test]
    fn test_div() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Div };
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(10), Value::Int64(3)]).unwrap(), Value::Int64(3));
    }

    #[test]
    fn test_div_by_zero() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Div };
        assert!(evaluate_scalar(&func, &[Value::Int64(1), Value::Int64(0)]).is_err());
    }

    #[test]
    fn test_mod() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Mod };
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(10), Value::Int64(3)]).unwrap(), Value::Int64(1));
    }

    #[test]
    fn test_abs() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Abs };
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(-5)]).unwrap(), Value::Int64(5));
    }

    #[test]
    fn test_negate() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Negate };
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(42)]).unwrap(), Value::Int64(-42));
    }

    #[test]
    fn test_comparison_eq() {
        let func = ScalarFunction::Comparison { op: ComparisonOp::Eq };
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(1), Value::Int64(1)]).unwrap(), Value::Bool(true));
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(1), Value::Int64(2)]).unwrap(), Value::Bool(false));
    }

    #[test]
    fn test_comparison_gt() {
        let func = ScalarFunction::Comparison { op: ComparisonOp::Gt };
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(5), Value::Int64(3)]).unwrap(), Value::Bool(true));
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(3), Value::Int64(5)]).unwrap(), Value::Bool(false));
    }

    #[test]
    fn test_string_concat() {
        let func = ScalarFunction::String { op: StringOp::Concat };
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("hello ".into()), Value::String("world".into())]).unwrap(),
            Value::String("hello world".into())
        );
    }

    #[test]
    fn test_string_to_upper() {
        let func = ScalarFunction::String { op: StringOp::ToUpper };
        assert_eq!(evaluate_scalar(&func, &[Value::String("hello".into())]).unwrap(), Value::String("HELLO".into()));
    }

    #[test]
    fn test_string_to_lower() {
        let func = ScalarFunction::String { op: StringOp::ToLower };
        assert_eq!(evaluate_scalar(&func, &[Value::String("HELLO".into())]).unwrap(), Value::String("hello".into()));
    }

    #[test]
    fn test_string_trim() {
        let func = ScalarFunction::String { op: StringOp::Trim };
        assert_eq!(evaluate_scalar(&func, &[Value::String("  hello  ".into())]).unwrap(), Value::String("hello".into()));
    }

    #[test]
    fn test_string_length() {
        let func = ScalarFunction::String { op: StringOp::Length };
        assert_eq!(evaluate_scalar(&func, &[Value::String("hello".into())]).unwrap(), Value::Int64(5));
    }

    #[test]
    fn test_string_contains() {
        let func = ScalarFunction::String { op: StringOp::Contains };
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("hello world".into()), Value::String("world".into())]).unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn test_boolean_and() {
        let func = ScalarFunction::Boolean { op: BooleanOp::And };
        assert_eq!(evaluate_scalar(&func, &[Value::Bool(true), Value::Bool(true)]).unwrap(), Value::Bool(true));
        assert_eq!(evaluate_scalar(&func, &[Value::Bool(true), Value::Bool(false)]).unwrap(), Value::Bool(false));
    }

    #[test]
    fn test_boolean_or() {
        let func = ScalarFunction::Boolean { op: BooleanOp::Or };
        assert_eq!(evaluate_scalar(&func, &[Value::Bool(true), Value::Bool(false)]).unwrap(), Value::Bool(true));
    }

    #[test]
    fn test_boolean_not() {
        let func = ScalarFunction::Boolean { op: BooleanOp::Not };
        assert_eq!(evaluate_scalar(&func, &[Value::Bool(true)]).unwrap(), Value::Bool(false));
        assert_eq!(evaluate_scalar(&func, &[Value::Bool(false)]).unwrap(), Value::Bool(true));
    }

    #[test]
    fn test_is_null() {
        let func = ScalarFunction::Comparison { op: ComparisonOp::IsNull };
        assert_eq!(evaluate_scalar(&func, &[Value::Null]).unwrap(), Value::Bool(true));
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(5)]).unwrap(), Value::Bool(false));
    }

    #[test]
    fn test_coalesce() {
        let func = ScalarFunction::Utility { op: UtilityOp::Coalesce };
        assert_eq!(
            evaluate_scalar(&func, &[Value::Null, Value::Int64(42)]).unwrap(),
            Value::Int64(42)
        );
    }

    #[test]
    fn test_cast_int64() {
        let func = ScalarFunction::Cast { target_type: CastTarget::Int64 };
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(42)]).unwrap(), Value::Int64(42));
        assert_eq!(evaluate_scalar(&func, &[Value::Double(3.14)]).unwrap(), Value::Int64(3));
    }

    #[test]
    fn test_cast_string() {
        let func = ScalarFunction::Cast { target_type: CastTarget::String };
        let result = evaluate_scalar(&func, &[Value::Int64(42)]).unwrap();
        assert!(matches!(result, Value::String(_)));
    }

    #[test]
    fn test_list_len() {
        let func = ScalarFunction::List { op: ListOp::Len };
        assert_eq!(evaluate_scalar(&func, &[Value::List(vec![Value::Int64(1), Value::Int64(2)])]).unwrap(), Value::Int64(2));
    }

    #[test]
    fn test_list_contains() {
        let func = ScalarFunction::List { op: ListOp::Contains };
        let list = Value::List(vec![Value::Int64(1), Value::Int64(2), Value::Int64(3)]);
        assert_eq!(evaluate_scalar(&func, &[list.clone(), Value::Int64(2)]).unwrap(), Value::Bool(true));
        assert_eq!(evaluate_scalar(&func, &[list, Value::Int64(99)]).unwrap(), Value::Bool(false));
    }

    #[test]
    fn test_list_append() {
        let func = ScalarFunction::List { op: ListOp::Append };
        let result = evaluate_scalar(&func, &[Value::List(vec![Value::Int64(1)]), Value::Int64(2)]).unwrap();
        match result {
            Value::List(items) => assert_eq!(items.len(), 2),
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn test_typeof() {
        let func = ScalarFunction::Utility { op: UtilityOp::TypeOf };
        let result = evaluate_scalar(&func, &[Value::Int64(42)]).unwrap();
        assert!(matches!(result, Value::String(_)));
    }

    #[test]
    fn test_regex_matches() {
        let func = ScalarFunction::String { op: StringOp::RegexMatches };
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("hello123".into()), Value::String(r"\d+".into())]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("hello".into()), Value::String(r"\d+".into())]).unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn test_regex_replace() {
        let func = ScalarFunction::String { op: StringOp::RegexReplace };
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("hello 123 world".into()), Value::String(r"\d+".into()), Value::String("NUM".into())]).unwrap(),
            Value::String("hello NUM world".into())
        );
    }

    #[test]
    fn test_function_registry_lookup() {
        let reg = FunctionRegistry::new();
        assert!(reg.contains("+"));
        assert!(reg.contains("COUNT"));
        assert!(reg.contains("trim"));
        assert!(reg.contains("list_tables"));
        assert!(reg.scalar_count() > 30);
        assert!(reg.aggregate_count() >= 7);
        assert!(reg.total_count() >= 40);
    }
}
