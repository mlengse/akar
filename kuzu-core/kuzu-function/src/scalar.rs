//! Scalar function evaluation.
//!
//! Each function takes input `Value` slices and produces an output `Value`.

#![allow(
    clippy::unnecessary_cast,
    clippy::approx_constant,
    clippy::manual_is_multiple_of,
    clippy::clone_on_ref_ptr,
    clippy::collapsible_if,
    clippy::never_loop
)]

use crate::registry::*;
use kuzu_common::types::{Date, Interval, Timestamp, Value};
use md5::{Digest, Md5};
use sha2::Sha256;
use time::{Date as TimeDate, Month, OffsetDateTime, Time as TimeTime};

// ==================== Module-level utilities ====================

thread_local! {
    static RNG_STATE: std::cell::Cell<u64> = std::cell::Cell::new(
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(12345)
    );
}

/// Get next random f64 in [0, 1) from the thread-local LCG.
fn rng_next() -> f64 {
    RNG_STATE.with(|state| {
        let old = state.get();
        let new = old.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        state.set(new);
        (new >> 11) as f64 / (1u64 << 53) as f64
    })
}

/// Set the thread-local RNG seed.
pub fn set_rng_seed(seed: u64) {
    RNG_STATE.with(|state| state.set(seed));
}

/// Lanczos approximation for log-gamma ln(|Γ(x)|).
#[allow(clippy::excessive_precision)]
fn log_gamma(x: f64) -> f64 {
    if x < 0.5 {
        let pi = std::f64::consts::PI;
        let reflection = pi / (pi * x).sin();
        reflection.abs().ln() - log_gamma(1.0 - x)
    } else {
        let xm1 = x - 1.0;
        let g = 7.0;
        let c = [
            0.99999999999980993,
            676.5203681218851,
            -1259.1392167224028,
            771.32342877765313,
            -176.61502916214059,
            12.507343278686905,
            -0.13857109526572012,
            9.9843695780195716e-6,
            1.5056327351493116e-7,
        ];
        let t = xm1 + g + 0.5;
        let mut s = c[0];
        for (i, &ci) in c[1..].iter().enumerate() {
            s += ci / (xm1 + (i as f64) + 1.0);
        }
        let sqrt_2pi = (2.0 * std::f64::consts::PI).sqrt();
        (sqrt_2pi * s).ln() + (xm1 + 0.5) * t.ln() - t
    }
}

/// Lanczos approximation for Gamma(x) — computed via exp(log_gamma(x)).
fn gamma_func(x: f64) -> f64 {
    log_gamma(x).exp()
}

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
        ScalarFunction::Schema { op } => evaluate_schema(*op, args),
        ScalarFunction::Array { op } => evaluate_array(*op, args),
        ScalarFunction::Path { op } => evaluate_path(*op, args),
        ScalarFunction::Hash { op } => evaluate_hash(*op, args),
        ScalarFunction::Interval { op } => evaluate_interval(*op, args),
        ScalarFunction::Blob { op } => evaluate_blob(*op, args),
        ScalarFunction::Union { op } => evaluate_union(*op, args),
        ScalarFunction::Uuid => evaluate_uuid(args),
        ScalarFunction::CustomScalar { execute, .. } => (execute)(args),
        ScalarFunction::SequenceOp { .. } => {
            Err("Sequence operations (nextval/currval) require catalog access — handle at connection/processor level".into())
        }
    }
}

// ==================== Arithmetic ====================

fn evaluate_arithmetic(op: ArithmeticOp, args: &[Value]) -> Result<Value, String> {
    // Allow empty args for ops that take no arguments (Pi, Rand)
    let needs_args = !matches!(op, ArithmeticOp::Pi | ArithmeticOp::Rand);
    if args.is_empty() && needs_args {
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
        ArithmeticOp::Sin => {
            let v = numeric_to_f64(&args[0])?;
            Ok(Value::Double(v.sin()))
        }
        ArithmeticOp::Cos => {
            let v = numeric_to_f64(&args[0])?;
            Ok(Value::Double(v.cos()))
        }
        ArithmeticOp::Tan => {
            let v = numeric_to_f64(&args[0])?;
            Ok(Value::Double(v.tan()))
        }
        ArithmeticOp::Asin => {
            let v = numeric_to_f64(&args[0])?;
            Ok(Value::Double(v.asin()))
        }
        ArithmeticOp::Acos => {
            let v = numeric_to_f64(&args[0])?;
            Ok(Value::Double(v.acos()))
        }
        ArithmeticOp::Atan => {
            let v = numeric_to_f64(&args[0])?;
            Ok(Value::Double(v.atan()))
        }
        ArithmeticOp::Atan2 => {
            if args.len() < 2 {
                return Err("Atan2 requires 2 arguments".into());
            }
            let y = numeric_to_f64(&args[0])?;
            let x = numeric_to_f64(&args[1])?;
            Ok(Value::Double(y.atan2(x)))
        }
        ArithmeticOp::Degrees => {
            let v = numeric_to_f64(&args[0])?;
            Ok(Value::Double(v.to_degrees()))
        }
        ArithmeticOp::Radians => {
            let v = numeric_to_f64(&args[0])?;
            Ok(Value::Double(v.to_radians()))
        }
        ArithmeticOp::Sign => {
            let v = numeric_to_f64(&args[0])?;
            Ok(Value::Int64(if v > 0.0 { 1 } else if v < 0.0 { -1 } else { 0 }))
        }
        ArithmeticOp::Pi => {
            Ok(Value::Double(std::f64::consts::PI))
        }
        ArithmeticOp::Rand => {
            Ok(Value::Double(rng_next()))
        }
        ArithmeticOp::Power => {
            if args.len() < 2 {
                return Err("Power requires 2 arguments".into());
            }
            let base = numeric_to_f64(&args[0])?;
            let exp = numeric_to_f64(&args[1])?;
            Ok(Value::Double(base.powf(exp)))
        }
        // Math functions (f64-based, single argument)
        ArithmeticOp::Cbrt => {
            let v = numeric_to_f64(&args[0])?;
            Ok(Value::Double(v.cbrt()))
        }
        ArithmeticOp::Cot => {
            let v = numeric_to_f64(&args[0])?;
            Ok(Value::Double(1.0 / v.tan()))
        }
        ArithmeticOp::Log2 => {
            let v = numeric_to_f64(&args[0])?;
            Ok(Value::Double(v.log2()))
        }
        ArithmeticOp::Even => {
            let v = args[0].clone();
            match v {
                Value::Int64(x) => {
                    // Round up to nearest even integer
                    let rounded = if x % 2 == 0 { x } else { x + 1 };
                    Ok(Value::Int64(rounded))
                }
                Value::Double(x) => {
                    let rounded = x.ceil() as i64;
                    let result = if rounded % 2 == 0 { rounded } else { rounded + 1 };
                    Ok(Value::Int64(result))
                }
                _ => Err("Even requires numeric argument".into()),
            }
        }
        // Heavy math functions (C++ port)
        ArithmeticOp::Factorial => {
            let n = match &args[0] {
                Value::Int64(x) if *x >= 0 => *x,
                Value::Int64(_) => return Err("Factorial requires non-negative integer".into()),
                _ => return Err("Factorial requires integer argument".into()),
            };
            let mut result: i64 = 1;
            for i in 2..=n {
                result = result.wrapping_mul(i);
            }
            Ok(Value::Int64(result))
        }
        ArithmeticOp::Gamma => {
            let v = numeric_to_f64(&args[0])?;
            // Poles at non-positive integers
            if v <= 0.0 && (v - v.round()).abs() < 1e-12 {
                return Ok(Value::Double(f64::INFINITY));
            }
            Ok(Value::Double(gamma_func(v)))
        }
        ArithmeticOp::Lgamma => {
            let v = numeric_to_f64(&args[0])?;
            // Poles at non-positive integers: return infinity
            if v <= 0.0 && (v - v.round()).abs() < 1e-12 {
                return Ok(Value::Double(f64::INFINITY));
            }
            Ok(Value::Double(log_gamma(v)))
        }
        ArithmeticOp::SetSeed => {
            let v = match &args[0] {
                Value::Double(x) => x,
                Value::Int64(x) => &(*x as f64),
                _ => return Err("SetSeed requires numeric argument".into()),
            };
            let seed = (v * (u64::MAX as f64)) as u64;
            set_rng_seed(seed);
            // Return INT32(0) to match C++ semantics
            Ok(Value::Int32(0))
        }
        // Bitwise operations (int64-only, matching C++ hardcoded int64_t)
        ArithmeticOp::BitwiseAnd => {
            if args.len() < 2 {
                return Err("Bitwise AND requires 2 arguments".into());
            }
            match (&args[0], &args[1]) {
                (Value::Int64(x), Value::Int64(y)) => Ok(Value::Int64(x & y)),
                _ => Err("Bitwise AND requires integer arguments".into()),
            }
        }
        ArithmeticOp::BitwiseOr => {
            if args.len() < 2 {
                return Err("Bitwise OR requires 2 arguments".into());
            }
            match (&args[0], &args[1]) {
                (Value::Int64(x), Value::Int64(y)) => Ok(Value::Int64(x | y)),
                _ => Err("Bitwise OR requires integer arguments".into()),
            }
        }
        ArithmeticOp::BitwiseXor => {
            if args.len() < 2 {
                return Err("Bitwise XOR requires 2 arguments".into());
            }
            match (&args[0], &args[1]) {
                (Value::Int64(x), Value::Int64(y)) => Ok(Value::Int64(x ^ y)),
                _ => Err("Bitwise XOR requires integer arguments".into()),
            }
        }
        ArithmeticOp::BitShiftLeft => {
            if args.len() < 2 {
                return Err("Bit shift left requires 2 arguments".into());
            }
            match (&args[0], &args[1]) {
                (Value::Int64(x), Value::Int64(y)) => Ok(Value::Int64(x << y)),
                _ => Err("Bit shift left requires integer arguments".into()),
            }
        }
        ArithmeticOp::BitShiftRight => {
            if args.len() < 2 {
                return Err("Bit shift right requires 2 arguments".into());
            }
            match (&args[0], &args[1]) {
                (Value::Int64(x), Value::Int64(y)) => Ok(Value::Int64(x >> y)),
                _ => Err("Bit shift right requires integer arguments".into()),
            }
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
            let s: String = args
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Null => "NULL".into(),
                    other => format!("{:?}", other),
                })
                .collect();
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
            let n = match &args[1] {
                Value::Int64(x) => *x as usize,
                _ => return Err("Repeat count must be integer".into()),
            };
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
            // Cypher uses 1-based indexing
            let start = match &args[1] {
                Value::Int64(x) => {
                    if *x < 1 {
                        return Err("Substring start must be >= 1".into());
                    }
                    (*x - 1) as usize
                }
                _ => return Err("Start must be integer".into()),
            };
            let len = if args.len() > 2 {
                match &args[2] {
                    Value::Int64(x) => Some(*x as usize),
                    _ => None,
                }
            } else {
                None
            };
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
        StringOp::Split => {
            let s = get_string(&args[0])?;
            let delim = if args.len() > 1 { get_string(&args[1])? } else { ",".to_string() };
            let parts: Vec<Value> = s.split(&delim).map(|p| Value::String(p.to_string())).collect();
            Ok(Value::List(parts))
        }
        StringOp::Head => {
            let s = get_string(&args[0])?;
            let n = if args.len() > 1 {
                match &args[1] { Value::Int64(x) => *x as usize, _ => 1 }
            } else { 1 };
            Ok(Value::String(s.chars().take(n).collect()))
        }
        StringOp::Tail => {
            let s = get_string(&args[0])?;
            let n = if args.len() > 1 {
                match &args[1] { Value::Int64(x) => *x as usize, _ => 1 }
            } else { 1 };
            let chars: String = s.chars().collect();
            let start = chars.len().saturating_sub(n);
            Ok(Value::String(chars.chars().skip(start).collect()))
        }
        StringOp::Left => {
            let s = get_string(&args[0])?;
            let n = match &args[1] { Value::Int64(x) => *x as usize, _ => return Err("left requires integer length".into()) };
            Ok(Value::String(s.chars().take(n).collect()))
        }
        StringOp::Right => {
            let s = get_string(&args[0])?;
            let n = match &args[1] { Value::Int64(x) => *x as usize, _ => return Err("right requires integer length".into()) };
            let chars: Vec<char> = s.chars().collect();
            let start = chars.len().saturating_sub(n);
            Ok(Value::String(chars[start..].iter().collect()))
        }
        StringOp::Lpad => {
            let s = get_string(&args[0])?;
            let len = match &args[1] { Value::Int64(x) => *x as usize, _ => return Err("lpad requires integer length".into()) };
            let pad = if args.len() >= 3 { get_string(&args[2])? } else { " ".into() };
            if s.len() >= len { return Ok(Value::String(s[..len].to_string())); }
            let pad_needed = len - s.len();
            let pad_repeat = pad.repeat((pad_needed / pad.len()) + 1);
            Ok(Value::String(format!("{}{}", &pad_repeat[..pad_needed], s)))
        }
        StringOp::Rpad => {
            let s = get_string(&args[0])?;
            let len = match &args[1] { Value::Int64(x) => *x as usize, _ => return Err("rpad requires integer length".into()) };
            let pad = if args.len() >= 3 { get_string(&args[2])? } else { " ".into() };
            if s.len() >= len { return Ok(Value::String(s[..len].to_string())); }
            let pad_needed = len - s.len();
            let pad_repeat = pad.repeat((pad_needed / pad.len()) + 1);
            Ok(Value::String(format!("{}{}", s, &pad_repeat[..pad_needed])))
        }
        // --- String basic (C++ port) ---
        StringOp::InitCap => {
            let s = get_string(&args[0])?;
            let lower = s.to_lowercase();
            let mut chars = lower.chars();
            match chars.next() {
                None => Ok(Value::String(String::new())),
                Some(c) => Ok(Value::String(c.to_uppercase().collect::<String>() + chars.as_str())),
            }
        }
        StringOp::ConcatWs => {
            if args.len() < 2 {
                return Err("concat_ws requires at least 2 arguments (separator + strings)".into());
            }
            let separator = get_string(&args[0])?;
            let mut result = String::new();
            let mut first = true;
            for arg in args.iter().skip(1) {
                match arg {
                    Value::Null => {
                        // Skip NULL elements (no separator before or after)
                        continue;
                    }
                    Value::String(s) => {
                        if !first {
                            result.push_str(&separator);
                        }
                        result.push_str(s);
                        first = false;
                    }
                    _ => {
                        if !first {
                            result.push_str(&separator);
                        }
                        result.push_str(&format!("{:?}", arg));
                        first = false;
                    }
                }
            }
            Ok(Value::String(result))
        }
        StringOp::SplitPart => {
            if args.len() < 3 {
                return Err("split_part requires 3 arguments (string, delimiter, index)".into());
            }
            let s = get_string(&args[0])?;
            let delim = get_string(&args[1])?;
            let idx = match &args[2] {
                Value::Int64(x) => *x,
                _ => return Err("split_part index must be integer".into()),
            };
            // 1-based index, matching C++ semantics
            let parts: Vec<&str> = s.split(&delim).collect();
            if idx <= 0 || (idx as usize) > parts.len() {
                Ok(Value::String(String::new()))
            } else {
                Ok(Value::String(parts[(idx - 1) as usize].to_string()))
            }
        }
        StringOp::ArrayExtract => {
            if args.len() < 2 {
                return Err("array_extract requires 2 arguments (string, index)".into());
            }
            let s = get_string(&args[0])?;
            let idx = match &args[1] {
                Value::Int64(x) => *x,
                _ => return Err("array_extract index must be integer".into()),
            };
            let chars: Vec<char> = s.chars().collect();
            if idx == 0 || chars.is_empty() {
                Ok(Value::String(String::new()))
            } else if idx > 0 {
                // 1-based: clamp to string length
                let pos = (idx as usize).saturating_sub(1).min(chars.len() - 1);
                Ok(Value::String(chars[pos].to_string()))
            } else {
                // Negative: from end (-1 = last char)
                let abs_idx = (-idx) as usize;
                if abs_idx > chars.len() {
                    Ok(Value::String(String::new()))
                } else {
                    let pos = chars.len() - abs_idx;
                    Ok(Value::String(chars[pos].to_string()))
                }
            }
        }
        // --- Regex string functions (C++ port) ---
        StringOp::RegexpFullMatch => {
            let s = get_string(&args[0])?;
            let pat = get_string(&args[1])?;
            let re = regex::Regex::new(&pat).map_err(|e| format!("Regex error: {e}"))?;
            Ok(Value::Bool(re.find(&s).is_some_and(|m| m.start() == 0 && m.end() == s.len())))
        }
        StringOp::RegexpExtract => {
            let s = get_string(&args[0])?;
            let pat = get_string(&args[1])?;
            let group = if args.len() > 2 {
                match &args[2] {
                    Value::Int64(x) => *x as usize,
                    _ => return Err("RegexpExtract group must be integer".into()),
                }
            } else {
                0
            };
            let re = regex::Regex::new(&pat).map_err(|e| format!("Regex error: {e}"))?;
            let result = re.captures(&s)
                .and_then(|caps| caps.get(group))
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            Ok(Value::String(result))
        }
        StringOp::RegexpExtractAll => {
            let s = get_string(&args[0])?;
            let pat = get_string(&args[1])?;
            let group = if args.len() > 2 {
                match &args[2] {
                    Value::Int64(x) => *x as usize,
                    _ => return Err("RegexpExtractAll group must be integer".into()),
                }
            } else {
                0
            };
            let re = regex::Regex::new(&pat).map_err(|e| format!("Regex error: {e}"))?;
            let matches: Vec<Value> = re.captures_iter(&s)
                .filter_map(|caps| caps.get(group))
                .map(|m| Value::String(m.as_str().to_string()))
                .collect();
            Ok(Value::List(matches))
        }
        StringOp::RegexpSplitToArray => {
            let s = get_string(&args[0])?;
            let pat = get_string(&args[1])?;
            let re = regex::Regex::new(&pat).map_err(|e| format!("Regex error: {e}"))?;
            let parts: Vec<Value> = re.split(&s).map(|p| Value::String(p.to_string())).collect();
            Ok(Value::List(parts))
        }
        StringOp::Levenshtein => {
            let a = get_string(&args[0])?;
            let b = get_string(&args[1])?;
            let a_chars: Vec<char> = a.chars().collect();
            let b_chars: Vec<char> = b.chars().collect();
            let n = b_chars.len();
            let mut prev_row: Vec<usize> = (0..=n).collect();
            let mut curr_row = vec![0usize; n + 1];
            for (i, ca) in a_chars.iter().enumerate() {
                curr_row[0] = i + 1;
                for (j, cb) in b_chars.iter().enumerate() {
                    let cost = if ca == cb { 0 } else { 1 };
                    curr_row[j + 1] = (curr_row[j] + 1)
                        .min(prev_row[j + 1] + 1)
                        .min(prev_row[j] + cost);
                }
                std::mem::swap(&mut prev_row, &mut curr_row);
            }
            Ok(Value::Int64(prev_row[n] as i64))
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

/// Helper: convert Date (days since epoch) to time::Date.
fn epoch_days_to_date(days: i32) -> Result<TimeDate, String> {
    TimeDate::from_calendar_date(1970, Month::January, 1)
        .map_err(|e| format!("Date error: {e}"))?
        .checked_add(time::Duration::days(days as i64))
        .ok_or_else(|| "Date overflow".into())
}

/// Helper: convert Timestamp (micros since epoch) to OffsetDateTime.
fn epoch_micros_to_datetime(micros: i64) -> Result<OffsetDateTime, String> {
    let secs = micros.div_euclid(1_000_000);
    let nanos = (micros.rem_euclid(1_000_000) * 1000) as u32;
    OffsetDateTime::from_unix_timestamp(secs)
        .map_err(|e| format!("Timestamp error: {e}"))?
        .replace_nanosecond(nanos)
        .map_err(|e| format!("Timestamp nanos error: {e}"))
}

/// Helper: get a numeric value from args (i64 or f64) for date math.
fn extract_numeric_value(v: &Value) -> Result<i64, String> {
    match v {
        Value::Int64(x) => Ok(*x),
        Value::Int32(x) => Ok(*x as i64),
        _ => Err("Expected numeric value for date operation".into()),
    }
}

fn evaluate_date(op: DateOp, args: &[Value]) -> Result<Value, String> {
    match op {
        DateOp::CurrentDate => {
            let now = OffsetDateTime::now_utc();
            let epoch_start =
                TimeDate::from_calendar_date(1970, Month::January, 1).map_err(|e| format!("Date error: {e}"))?;
            let days = (now.date() - epoch_start).whole_days() as i32;
            Ok(Value::Date(Date(days)))
        }
        DateOp::CurrentTimestamp => {
            let now = OffsetDateTime::now_utc();
            let micros = now.unix_timestamp() * 1_000_000 + now.nanosecond() as i64 / 1000;
            Ok(Value::Timestamp(Timestamp(micros)))
        }
        DateOp::Year => {
            let (date, _) = extract_date_or_timestamp(&args[0])?;
            Ok(Value::Int64(date.year() as i64))
        }
        DateOp::Month => {
            let (date, _) = extract_date_or_timestamp(&args[0])?;
            Ok(Value::Int64(date.month() as u8 as i64))
        }
        DateOp::Day => {
            let (date, _) = extract_date_or_timestamp(&args[0])?;
            Ok(Value::Int64(date.day() as i64))
        }
        DateOp::Hour => {
            let (_, time) = extract_date_or_timestamp(&args[0])?;
            Ok(Value::Int64(time.hour() as i64))
        }
        DateOp::Minute => {
            let (_, time) = extract_date_or_timestamp(&args[0])?;
            Ok(Value::Int64(time.minute() as i64))
        }
        DateOp::Second => {
            let (_, time) = extract_date_or_timestamp(&args[0])?;
            Ok(Value::Int64(time.second() as i64))
        }
        DateOp::DayName => {
            let (date, _) = extract_date_or_timestamp(&args[0])?;
            let weekday = date.weekday();
            Ok(Value::String(weekday.to_string()))
        }
        DateOp::MonthName => {
            let (date, _) = extract_date_or_timestamp(&args[0])?;
            Ok(Value::String(date.month().to_string()))
        }
        DateOp::LastDay => {
            let (date, _) = extract_date_or_timestamp(&args[0])?;
            // Go to first day of next month, then subtract one day
            let next_month_first = TimeDate::from_calendar_date(
                date.year() + if date.month() == time::Month::December { 1 } else { 0 },
                date.month().next(),
                1,
            ).map_err(|e| format!("Date error: {e}"))?;
            let last_day = next_month_first.previous_day()
                .ok_or("Could not compute previous day")?;
            let epoch = TimeDate::from_calendar_date(1970, Month::January, 1).unwrap();
            let days = (last_day - epoch).whole_days() as i32;
            Ok(Value::Date(Date(days)))
        }
        DateOp::MakeDate => {
            if args.len() < 3 { return Err("make_date requires 3 arguments (year, month, day)".into()); }
            let year = match &args[0] { Value::Int64(x) => *x as i32, _ => return Err("make_date year must be integer".into()) };
            let month_val = match &args[1] { Value::Int64(x) => *x as u8, _ => return Err("make_date month must be integer".into()) };
            let day = match &args[2] { Value::Int64(x) => *x as u8, _ => return Err("make_date day must be integer".into()) };
            let month_enum = Month::try_from(month_val).map_err(|_| format!("Invalid month: {month_val}"))?;
            let d = TimeDate::from_calendar_date(year, month_enum, day)
                .map_err(|e| format!("Invalid date: {e}"))?;
            let epoch = TimeDate::from_calendar_date(1970, Month::January, 1).unwrap();
            let days = (d - epoch).whole_days() as i32;
            Ok(Value::Date(Date(days)))
        }
        // --- Timestamp functions (C++ port) ---
        DateOp::Century => {
            let (date, _) = extract_date_or_timestamp(&args[0])?;
            let year = date.year();
            // PostgreSQL semantics: year 1→century 1, year 2000→20, year 2001→21
            let century = if year > 0 { (year - 1) / 100 + 1 } else { year / 100 - 1 };
            Ok(Value::Int64(century as i64))
        }
        DateOp::EpochMs => {
            // EPOCH_MS(ms): convert milliseconds since epoch → Timestamp
            let ms = match &args[0] {
                Value::Int64(x) => *x,
                _ => return Err("epoch_ms requires integer milliseconds".into()),
            };
            Ok(Value::Timestamp(Timestamp(ms * 1000)))
        }
        DateOp::ToTimestamp => {
            // TO_TIMESTAMP(sec): convert seconds since epoch (double) → Timestamp
            let secs = match &args[0] {
                Value::Double(x) => *x,
                Value::Int64(x) => *x as f64,
                _ => return Err("to_timestamp requires numeric seconds".into()),
            };
            let micros = (secs * 1_000_000.0) as i64;
            Ok(Value::Timestamp(Timestamp(micros)))
        }
        DateOp::ToEpochMs => {
            // TO_EPOCH_MS(timestamp): convert Timestamp → milliseconds since epoch
            let micros = match &args[0] {
                Value::Timestamp(t) | Value::TimestampMs(t) | Value::TimestampNs(t) | Value::TimestampSec(t) => t.0,
                _ => return Err("to_epoch_ms requires a timestamp argument".into()),
            };
            Ok(Value::Int64(micros / 1000))
        }
        DateOp::DatePart => {
            if args.len() < 2 {
                return Err("date_part requires 2 arguments".into());
            }
            let part = get_string(&args[0])?.to_lowercase();
            let (date, time) = extract_date_or_timestamp(&args[1])?;
            date_part_value(&part, &date, &time)
        }
        DateOp::DateTrunc => {
            if args.len() < 2 {
                return Err("date_trunc requires 2 arguments".into());
            }
            let part = get_string(&args[0])?.to_lowercase();
            let (date, _) = extract_date_or_timestamp(&args[1])?;
            date_trunc_value(&part, &date)
        }
        DateOp::DateDiff => {
            if args.len() < 3 {
                return Err("date_diff requires 3 arguments".into());
            }
            let part = get_string(&args[0])?.to_lowercase();
            let (d1, _) = extract_date_or_timestamp(&args[1])?;
            let (d2, _) = extract_date_or_timestamp(&args[2])?;
            date_diff_value(&part, &d1, &d2)
        }
        DateOp::DateAdd => {
            if args.len() < 3 {
                return Err("date_add requires 3 arguments".into());
            }
            let part = get_string(&args[0])?.to_lowercase();
            let count = extract_numeric_value(&args[1])?;
            let (date, _) = extract_date_or_timestamp(&args[2])?;
            date_add_value(&part, count, &date)
        }
    }
}

/// Extract date (and optionally time) from a Value that is Date or Timestamp.
fn extract_date_or_timestamp(v: &Value) -> Result<(TimeDate, TimeTime), String> {
    match v {
        Value::Date(d) => {
            let date = epoch_days_to_date(d.0)?;
            Ok((
                date,
                TimeTime::from_hms(0, 0, 0).map_err(|e| format!("Time error: {e}"))?,
            ))
        }
        Value::Timestamp(t) | Value::TimestampMs(t) | Value::TimestampNs(t) | Value::TimestampSec(t) => {
            let dt = epoch_micros_to_datetime(t.0)?;
            Ok((dt.date(), dt.time()))
        }
        Value::TimestampTz(t) => {
            let dt = epoch_micros_to_datetime(t.0)?;
            Ok((dt.date(), dt.time()))
        }
        _ => Err(format!("Expected date/timestamp, got {:?}", v.logical_type())),
    }
}

fn date_part_value(part: &str, date: &TimeDate, time: &TimeTime) -> Result<Value, String> {
    match part {
        "year" => Ok(Value::Int64(date.year() as i64)),
        "month" => Ok(Value::Int64(date.month() as u8 as i64)),
        "day" => Ok(Value::Int64(date.day() as i64)),
        "hour" => Ok(Value::Int64(time.hour() as i64)),
        "minute" => Ok(Value::Int64(time.minute() as i64)),
        "second" => Ok(Value::Int64(time.second() as i64)),
        "millisecond" => Ok(Value::Int64(time.millisecond() as i64)),
        "microsecond" => Ok(Value::Int64(time.microsecond() as i64)),
        "quarter" => Ok(Value::Int64((date.month() as u8 as i64 - 1) / 3 + 1)),
        "dayofyear" => Ok(Value::Int64(date.ordinal() as i64)),
        "week" | "weekofyear" => {
            let (_, iso_week, _) = date.to_iso_week_date();
            Ok(Value::Int64(iso_week as i64))
        }
        "dayofweek" | "dow" => {
            let w = date.weekday().number_from_monday();
            Ok(Value::Int64(w as i64))
        }
        "isodow" => {
            let w = date.weekday().number_from_monday();
            Ok(Value::Int64(w as i64))
        }
        "epoch" => Ok(Value::Int64(date.midnight().nanosecond() as i64)),
        _ => Err(format!("Unknown date_part: {}", part)),
    }
}

fn date_trunc_value(part: &str, date: &TimeDate) -> Result<Value, String> {
    let truncated = match part {
        "year" => {
            TimeDate::from_calendar_date(date.year(), Month::January, 1).map_err(|e| format!("Date error: {e}"))?
        }
        "month" => {
            TimeDate::from_calendar_date(date.year(), date.month(), 1).map_err(|e| format!("Date error: {e}"))?
        }
        "day" => *date,
        "week" => {
            // Truncate to Monday of the week
            let wd = date.weekday().number_from_monday() - 1;
            (*date) - time::Duration::days(wd as i64)
        }
        "quarter" => {
            let q = (date.month() as u8 as i64 - 1) / 3;
            let month = (q * 3 + 1) as u8;
            let m = Month::try_from(month).map_err(|_| "Invalid month".to_string())?;
            TimeDate::from_calendar_date(date.year(), m, 1).map_err(|e| format!("Date error: {e}"))?
        }
        _ => return Err(format!("date_trunc not supported for: {part}")),
    };
    let epoch_start = TimeDate::from_calendar_date(1970, Month::January, 1).map_err(|e| format!("Date error: {e}"))?;
    let days = (truncated - epoch_start).whole_days() as i32;
    Ok(Value::Date(Date(days)))
}

fn date_diff_value(part: &str, d1: &TimeDate, d2: &TimeDate) -> Result<Value, String> {
    let diff = match part {
        "year" => (d2.year() - d1.year()) as i64,
        "month" => ((d2.year() - d1.year()) * 12 + (d2.month() as i32 - d1.month() as i32)) as i64,
        "day" => (*d2 - *d1).whole_days(),
        "week" => (*d2 - *d1).whole_days() / 7,
        "hour" => (*d2 - *d1).whole_hours(),
        "minute" => (*d2 - *d1).whole_minutes(),
        "second" => (*d2 - *d1).whole_seconds(),
        "millisecond" => (*d2 - *d1).whole_milliseconds() as i64,
        "microsecond" => (*d2 - *d1).whole_microseconds() as i64,
        _ => return Err(format!("date_diff not supported for: {part}")),
    };
    Ok(Value::Int64(diff))
}

fn date_add_value(part: &str, count: i64, date: &TimeDate) -> Result<Value, String> {
    let result = match part {
        "year" => {
            let new_year = date.year() + count as i32;
            // Keep same month/day, clamp if needed
            let m = date.month();
            let d = date.day().min(days_in_month(new_year, m));
            TimeDate::from_calendar_date(new_year, m, d).map_err(|e| format!("Date error: {e}"))?
        }
        "month" => {
            let total_months = (date.year() * 12 + date.month() as i32 - 1) + count as i32;
            let new_year = (total_months.div_euclid(12)) as i32;
            let new_month = (total_months.rem_euclid(12) + 1) as u8;
            let m = Month::try_from(new_month).map_err(|_| "Invalid month".to_string())?;
            let d = date.day().min(days_in_month(new_year, m));
            TimeDate::from_calendar_date(new_year, m, d).map_err(|e| format!("Date error: {e}"))?
        }
        "day" => *date + time::Duration::days(count),
        "week" => *date + time::Duration::weeks(count),
        _ => return Err(format!("date_add not supported for: {part}")),
    };
    let epoch_start = TimeDate::from_calendar_date(1970, Month::January, 1).map_err(|e| format!("Date error: {e}"))?;
    let days = (result - epoch_start).whole_days() as i32;
    Ok(Value::Date(Date(days)))
}

fn days_in_month(year: i32, month: Month) -> u8 {
    match month {
        Month::January | Month::March | Month::May | Month::July | Month::August | Month::October | Month::December => {
            31
        }
        Month::April | Month::June | Month::September | Month::November => 30,
        Month::February => {
            if (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0) {
                29
            } else {
                28
            }
        }
    }
}

// ==================== List ====================

fn evaluate_list(op: ListOp, args: &[Value]) -> Result<Value, String> {
    match op {
        ListOp::Creation => {
            // list_creation just collects all args into a list
            Ok(Value::List(args.to_vec()))
        }
        ListOp::Len => match &args[0] {
            Value::List(items) => Ok(Value::Int64(items.len() as i64)),
            _ => Err("Expected list".into()),
        },
        ListOp::Extract => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let idx = match &args[1] {
                Value::Int64(i) => {
                    // Cypher uses 1-based indexing
                    if *i < 1 {
                        return Err("List index must be >= 1".into());
                    }
                    (*i - 1) as usize
                }
                _ => return Err("Index must be integer".into()),
            };
            list.get(idx)
                .cloned()
                .ok_or_else(|| format!("Index {idx} out of bounds"))
        }
        ListOp::Concat => {
            let mut result = Vec::new();
            for arg in args {
                match arg {
                    Value::List(items) => result.extend(items.clone()),
                    _ => result.push(arg.clone()),
                }
            }
            Ok(Value::List(result))
        }
        ListOp::Sort => {
            let mut list = match args[0].clone() {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            // Sort using Value's PartialOrd implementation (lexicographic)
            list.sort_by(|a, b| {
                match compare_values_for_sort(a, b) {
                    Ok(ord) => ord,
                    Err(_) => std::cmp::Ordering::Equal, // fallback for incomparable types
                }
            });
            Ok(Value::List(list))
        }
        ListOp::Contains => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            Ok(Value::Bool(list.contains(&args[1])))
        }
        ListOp::Append => {
            let mut list = match args[0].clone() {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            list.push(args[1].clone());
            Ok(Value::List(list))
        }
        ListOp::Prepend => {
            let mut list = match args[0].clone() {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            list.insert(0, args[1].clone());
            Ok(Value::List(list))
        }
        ListOp::Reverse => {
            let mut list = match args[0].clone() {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            list.reverse();
            Ok(Value::List(list))
        }
        ListOp::Slice => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let start = match &args[1] {
                Value::Int64(i) => {
                    if *i < 1 {
                        return Err("Slice start index must be >= 1".into());
                    }
                    (*i - 1) as usize
                }
                _ => return Err("Slice start must be integer".into()),
            };
            if start >= list.len() {
                return Err("Slice start index out of bounds".into());
            }
            if args.len() >= 3 {
                // Explicit end (1-based inclusive)
                let end = match &args[2] {
                    Value::Int64(i) => {
                        if *i < 1 {
                            return Err("Slice end index must be >= 1".into());
                        }
                        (*i - 1) as usize
                    }
                    _ => return Err("Slice end must be integer".into()),
                };
                if end >= list.len() || end < start {
                    return Err("Slice end index out of bounds".into());
                }
                Ok(Value::List(list[start..=end].to_vec()))
            } else {
                // No end specified — slice to the end of the list
                Ok(Value::List(list[start..].to_vec()))
            }
        }
        // --- List functions (C++ port) ---
        ListOp::Range => {
            let step = if args.len() >= 3 {
                match &args[2] { Value::Int64(s) => *s, _ => 1i64 }
            } else { 1i64 };
            let (start, end) = if args.len() >= 2 {
                match (&args[0], &args[1]) {
                    (Value::Int64(s), Value::Int64(e)) => (*s, *e),
                    _ => return Err("RANGE requires integer arguments".into()),
                }
            } else {
                match &args[0] {
                    Value::Int64(e) => (0i64, *e),
                    _ => return Err("RANGE requires integer arguments".into()),
                }
            };
            if step == 0 {
                return Err("Step of range cannot be 0".into());
            }
            if (end - start).signum() != step.signum() && end != start {
                Ok(Value::List(vec![]))
            } else {
                let size = ((end - start).unsigned_abs() / step.unsigned_abs()) + 1;
                let items: Vec<Value> = (0..size)
                    .map(|i| Value::Int64(start + step * i as i64))
                    .collect();
                Ok(Value::List(items))
            }
        }
        ListOp::Distinct => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let mut seen = hashbrown::HashSet::new();
            let mut result = Vec::new();
            for item in list {
                if !matches!(item, Value::Null) && seen.insert(format!("{:?}", item)) {
                    result.push(item.clone());
                }
            }
            Ok(Value::List(result))
        }
        ListOp::Unique => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let mut seen = hashbrown::HashSet::new();
            for item in list {
                if !matches!(item, Value::Null) {
                    seen.insert(format!("{:?}", item));
                }
            }
            Ok(Value::Int64(seen.len() as i64))
        }
        ListOp::Sum => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let mut sum: f64 = 0.0;
            let mut is_int = true;
            for item in list {
                match item {
                    Value::Null => continue,
                    Value::Int64(x) => sum += *x as f64,
                    Value::Double(x) => { sum += x; is_int = false; }
                    _ => return Err("LIST_SUM requires numeric list".into()),
                }
            }
            if is_int {
                Ok(Value::Int64(sum as i64))
            } else {
                Ok(Value::Double(sum))
            }
        }
        ListOp::Product => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let mut prod: f64 = 1.0;
            let mut is_int = true;
            for item in list {
                match item {
                    Value::Null => continue,
                    Value::Int64(x) => prod *= *x as f64,
                    Value::Double(x) => { prod *= x; is_int = false; }
                    _ => return Err("LIST_PRODUCT requires numeric list".into()),
                }
            }
            if is_int {
                Ok(Value::Int64(prod as i64))
            } else {
                Ok(Value::Double(prod))
            }
        }
        ListOp::AnyValue => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            // Return first non-null element
            match list.iter().find(|v| !matches!(v, Value::Null)) {
                Some(v) => Ok(v.clone()),
                None => Ok(Value::Null),
            }
        }
        ListOp::ToString => {
            // Parameters: (delimiter: STRING, list: LIST)
            if args.len() < 2 {
                return Err("list_to_string requires delimiter and list arguments".into());
            }
            let delim = get_string(&args[0])?;
            let list = match &args[1] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let mut result = String::new();
            let mut first = true;
            for item in list {
                if matches!(item, Value::Null) { continue; }
                if !first { result.push_str(&delim); }
                match item {
                    Value::String(s) => result.push_str(s),
                    other => result.push_str(&format!("{:?}", other)),
                }
                first = false;
            }
            Ok(Value::String(result))
        }
        ListOp::Position => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let target = &args[1];
            // 1-based index, returns 0 if not found
            for (i, item) in list.iter().enumerate() {
                if item == target {
                    return Ok(Value::Int64((i + 1) as i64));
                }
            }
            Ok(Value::Int64(0))
        }
        ListOp::HasAll => {
            let left = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let right = match &args[1] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            for target in right {
                if matches!(target, Value::Null) { continue; }
                if !left.contains(target) {
                    return Ok(Value::Bool(false));
                }
            }
            Ok(Value::Bool(true))
        }
        ListOp::ReverseSort => {
            let mut list = match args[0].clone() {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            list.sort_by(|a, b| {
                match compare_values_for_sort(a, b) {
                    Ok(ord) => ord.reverse(),
                    Err(_) => std::cmp::Ordering::Equal,
                }
            });
            Ok(Value::List(list))
        }
        // --- List predicate functions (non-lambda) ---
        ListOp::Any => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            Ok(Value::Bool(list.iter().any(is_truthy)))
        }
        ListOp::All => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            Ok(Value::Bool(!list.is_empty() && list.iter().all(is_truthy)))
        }
        ListOp::None => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            Ok(Value::Bool(list.iter().all(|v| !is_truthy(v))))
        }
        ListOp::Single => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let count = list.iter().filter(|v| is_truthy(v)).count();
            Ok(Value::Bool(count == 1))
        }
    }
}

/// Check if a Value is "truthy": Bool(true) or non-zero Int64/Double.
fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Int64(x) => *x != 0,
        Value::Double(x) => *x != 0.0,
        _ => false,
    }
}

/// Compare two Values for sorting purposes. Supports numeric, string, bool, date, timestamp.
fn compare_values_for_sort(a: &Value, b: &Value) -> Result<std::cmp::Ordering, String> {
    match (a, b) {
        (Value::Null, Value::Null) => Ok(std::cmp::Ordering::Equal),
        (Value::Null, _) => Ok(std::cmp::Ordering::Less),
        (_, Value::Null) => Ok(std::cmp::Ordering::Greater),
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
        // Cross-type numeric promotion
        (Value::Int64(x), Value::Double(y)) => Ok(x
            .partial_cmp(&(*y as i64))
            .map(|o| o.reverse())
            .unwrap_or(std::cmp::Ordering::Equal)),
        (Value::Double(x), Value::Int64(y)) => Ok(x.partial_cmp(&(*y as f64)).unwrap_or(std::cmp::Ordering::Equal)),
        _ => Err("Cannot compare types for sort".into()),
    }
}

// ==================== Map & Struct ====================

fn evaluate_map(op: MapOp, args: &[Value]) -> Result<Value, String> {
    match op {
        MapOp::Creation => {
            // map_creation(args): args are alternating key, value, key, value, ...
            if args.len() % 2 != 0 {
                return Err("Map creation requires an even number of arguments (key-value pairs)".into());
            }
            let mut entries = Vec::new();
            let mut i = 0;
            while i < args.len() {
                let key = match &args[i] {
                    Value::String(s) => s.clone(),
                    other => return Err(format!("Map key must be a string, got {:?}", other.logical_type())),
                };
                let val = args[i + 1].clone();
                entries.push((key, val));
                i += 2;
            }
            Ok(Value::Struct(entries))
        }
        MapOp::Extract => {
            let map_val = &args[0];
            let key = get_string(&args[1])?;
            match map_val {
                Value::Struct(entries) => {
                    for (k, v) in entries {
                        if *k == key {
                            return Ok(v.clone());
                        }
                    }
                    Err(format!("Key '{}' not found in map", key))
                }
                _ => Err("Expected map/struct".into()),
            }
        }
        MapOp::Keys => match &args[0] {
            Value::Struct(entries) => Ok(Value::List(
                entries.iter().map(|(k, _)| Value::String(k.clone())).collect(),
            )),
            _ => Err("Expected map/struct".into()),
        },
        MapOp::Values => match &args[0] {
            Value::Struct(entries) => Ok(Value::List(entries.iter().map(|(_, v)| v.clone()).collect())),
            _ => Err("Expected map/struct".into()),
        },
        MapOp::Contains => match &args[0] {
            Value::Struct(entries) => {
                let key = get_string(&args[1])?;
                Ok(Value::Bool(entries.iter().any(|(k, _)| *k == key)))
            }
            _ => Err("Expected map/struct".into()),
        },
    }
}

fn evaluate_struct(op: StructOp, args: &[Value]) -> Result<Value, String> {
    match op {
        StructOp::Creation => {
            // struct_creation(args): args are alternating field_name (string), value, ...
            if args.len() % 2 != 0 {
                return Err("Struct creation requires an even number of arguments (field-value pairs)".into());
            }
            let mut entries = Vec::new();
            let mut i = 0;
            while i < args.len() {
                let field_name = match &args[i] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "Struct field name must be a string, got {:?}",
                            other.logical_type()
                        ));
                    }
                };
                let val = args[i + 1].clone();
                entries.push((field_name, val));
                i += 2;
            }
            Ok(Value::Struct(entries))
        }
        StructOp::Extract => {
            let struct_val = &args[0];
            let key = get_string(&args[1])?;
            match struct_val {
                Value::Struct(entries) => {
                    for (k, v) in entries {
                        if *k == key {
                            return Ok(v.clone());
                        }
                    }
                    Err(format!("Key '{}' not found in struct", key))
                }
                _ => Err("Expected struct".into()),
            }
        }
    }
}

// ==================== Boolean ====================

// ==================== Path ====================

fn evaluate_path(op: PathOp, args: &[Value]) -> Result<Value, String> {
    match op {
        PathOp::Nodes => {
            let path = &args[0];
            match path {
                Value::Struct(fields) => {
                    // Look for "_nodes" field in struct
                    if let Some((_, nodes_val)) = fields.iter().find(|(k, _)| k == "_nodes") {
                        Ok(nodes_val.clone())
                    } else if let Some((_, first)) = fields.first() {
                        // Fallback: return first field (usually nodes list)
                        Ok(first.clone())
                    } else {
                        Ok(Value::Null)
                    }
                }
                Value::List(_) => Ok(path.clone()),
                _ => Err(format!("NODES() requires a path/recursive rel, got {:?}", path)),
            }
        }
        PathOp::Rels => {
            let path = &args[0];
            match path {
                Value::Struct(fields) => {
                    if let Some((_, rels_val)) = fields.iter().find(|(k, _)| k == "_rels") {
                        Ok(rels_val.clone())
                    } else if fields.len() >= 2 {
                        Ok(fields[1].1.clone())
                    } else {
                        Ok(Value::Null)
                    }
                }
                _ => Err(format!("RELS() requires a path/recursive rel, got {:?}", path)),
            }
        }
        PathOp::Length => {
            let path = &args[0];
            match path {
                Value::List(items) => Ok(Value::Int64(items.len() as i64)),
                Value::Struct(fields) => {
                    // Count entries in _rels or _nodes minus 1
                    if let Some((_, Value::List(rels))) = fields.iter().find(|(k, _)| k == "_rels") {
                        Ok(Value::Int64(rels.len() as i64))
                    } else {
                        Ok(Value::Int64(0))
                    }
                }
                _ => Err(format!("LENGTH() requires a path/recursive rel, got {:?}", path)),
            }
        }
    }
}

/// Generate a random UUID v4 string.
fn evaluate_uuid(_args: &[Value]) -> Result<Value, String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    // Simple UUID v4 generation without external crate dependency
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let mut seed = now.as_nanos() as u64;
    // Simple PRNG (xorshift64*)
    seed ^= seed >> 12;
    seed ^= seed << 25;
    seed ^= seed >> 27;
    let r1 = seed.wrapping_mul(0x2545F4914F6CDD1Du64);
    seed ^= seed >> 12;
    seed ^= seed << 25;
    seed ^= seed >> 27;
    let r2 = seed.wrapping_mul(0x2545F4914F6CDD1Du64);
    
    // Format as UUID v4: 8-4-4-4-12 hex digits
    let time_low = (r1 & 0xFFFFFFFF) as u32;
    let time_mid = ((r1 >> 32) & 0xFFFF) as u16;
    let time_hi_and_version = (((r1 >> 48) & 0x0FFF) | 0x4000) as u16; // version 4
    let clock_seq = ((r2 & 0x3FFF) | 0x8000) as u16; // variant 1
    let node_low = ((r2 >> 14) & 0xFFFFFFFF) as u32;
    let node_hi = ((r2 >> 46) & 0xFFFF) as u16;
    
    Ok(Value::String(format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:04x}{:08x}",
        time_low, time_mid, time_hi_and_version, clock_seq, node_hi, node_low
    )))
}

// ==================== Hash functions ====================

/// Simple non-cryptographic hash for any Value (matching C++ murmurhash64 semantics).
fn hash_value(v: &Value) -> u64 {
    match v {
        Value::Null => u64::MAX,
        Value::Bool(b) => murmur64(*b as u64),
        Value::Int64(x) => murmur64(*x as u64),
        Value::Int32(x) => murmur64(*x as u64),
        Value::Double(x) => {
            if *x == 0.0 { murmur64(0) } else { murmur64(x.to_bits()) }
        }
        Value::String(s) => hash_string(s),
        Value::List(items) => {
            let mut h: u64 = 0;
            for item in items {
                h = combine_hash(h, hash_value(item));
            }
            h
        }
        _ => {
            let s = format!("{:?}", v);
            hash_string(&s)
        }
    }
}

fn murmur64(mut x: u64) -> u64 {
    x ^= x >> 32;
    x = x.wrapping_mul(0xd6e8feb86659fd93);
    x ^= x >> 32;
    x = x.wrapping_mul(0xd6e8feb86659fd93);
    x ^= x >> 32;
    x
}

fn combine_hash(a: u64, b: u64) -> u64 {
    a.wrapping_mul(0xbf58476d1ce4e5b9) ^ b
}

fn hash_string(s: &str) -> u64 {
    let bytes = s.as_bytes();
    let mut h: u64 = 0;
    for chunk in bytes.chunks(8) {
        let mut val: u64 = 0;
        for (i, &b) in chunk.iter().enumerate() {
            val |= (b as u64) << (i * 8);
        }
        h = combine_hash(h, murmur64(val));
    }
    h
}

fn evaluate_hash(op: HashOp, args: &[Value]) -> Result<Value, String> {
    match op {
        HashOp::Md5 => {
            let s = get_string(&args[0])?;
            let mut hasher = Md5::new();
            hasher.update(s.as_bytes());
            let result = hasher.finalize();
            Ok(Value::String(format!("{:x}", result)))
        }
        HashOp::Sha256 => {
            let s = get_string(&args[0])?;
            let mut hasher = Sha256::new();
            hasher.update(s.as_bytes());
            let result = hasher.finalize();
            Ok(Value::String(format!("{:x}", result)))
        }
        HashOp::Hash => {
            if args.is_empty() {
                return Err("hash requires at least one argument".into());
            }
            let h = hash_value(&args[0]);
            Ok(Value::Int64(h as i64))
        }
    }
}

// ==================== Interval constructor functions ====================

/// Evaluate an interval constructor function.
/// Each takes a single INT64 argument and returns an INTERVAL value.
fn evaluate_interval(op: IntervalOp, args: &[Value]) -> Result<Value, String> {
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

// ==================== Union functions ====================

/// Evaluate a union function.
fn evaluate_union(op: UnionOp, args: &[Value]) -> Result<Value, String> {
    match op {
        UnionOp::UnionValue => {
            // UNION_VALUE(val) → create a union wrapping the value as a single variant
            let val = args[0].clone();
            Ok(Value::Struct(vec![
                ("tag".to_string(), Value::UInt16(0)),
                ("_value".to_string(), val),
            ]))
        }
        UnionOp::UnionTag => {
            // UNION_TAG(union) → return the active tag name as a string
            let entries = match &args[0] {
                Value::Struct(entries) => entries,
                _ => return Err("UNION_TAG requires a union argument".into()),
            };
            // Find the tag field (should be first entry)
            let tag_val = entries.iter().find(|(k, _)| k == "tag")
                .ok_or("Union has no tag field".to_string())?;
            let tag_idx = match &tag_val.1 {
                Value::UInt16(x) => *x as usize,
                _ => return Err("Invalid tag field type".into()),
            };
            // The active variant name is at entries[tag_idx + 1]
            let field_idx = tag_idx + 1;
            if field_idx >= entries.len() {
                return Err(format!("Union tag index {} out of range", tag_idx));
            }
            Ok(Value::String(entries[field_idx].0.clone()))
        }
        UnionOp::UnionExtract => {
            // UNION_EXTRACT(union, key) → same as struct_extract
            let struct_val = &args[0];
            let key = get_string(&args[1])?;
            match struct_val {
                Value::Struct(entries) => {
                    for (k, v) in entries {
                        if *k == key {
                            return Ok(v.clone());
                        }
                    }
                    Err(format!("Key '{}' not found in union", key))
                }
                _ => Err("UNION_EXTRACT requires a union argument".into()),
            }
        }
    }
}

// ==================== Blob functions ====================

/// Evaluate a blob function.
fn evaluate_blob(op: BlobOp, args: &[Value]) -> Result<Value, String> {
    match op {
        BlobOp::Encode => {
            let s = get_string(&args[0])?;
            Ok(Value::Blob(s.into_bytes()))
        }
        BlobOp::Decode => {
            let bytes = match &args[0] {
                Value::Blob(b) => b.clone(),
                _ => return Err("DECODE requires a blob argument".into()),
            };
            let s = String::from_utf8(bytes).map_err(|_| {
                "Failure in decode: could not convert blob to UTF8 string, the blob contained invalid UTF8 characters".to_string()
            })?;
            Ok(Value::String(s))
        }
        BlobOp::OctetLength => {
            let len = match &args[0] {
                Value::Blob(b) => b.len() as i64,
                _ => return Err("OCTET_LENGTH requires a blob argument".into()),
            };
            Ok(Value::Int64(len))
        }
    }
}

fn evaluate_boolean(op: BooleanOp, args: &[Value]) -> Result<Value, String> {
    if args.len() < 2 && !matches!(op, BooleanOp::Not) {
        return Err("Boolean op requires 2 arguments".into());
    }
    let a = match &args[0] {
        Value::Bool(b) => *b,
        _ => return Err("Expected boolean".into()),
    };
    match op {
        BooleanOp::And => {
            let b = match &args[1] {
                Value::Bool(x) => *x,
                _ => return Err("Expected boolean".into()),
            };
            Ok(Value::Bool(a && b))
        }
        BooleanOp::Or => {
            let b = match &args[1] {
                Value::Bool(x) => *x,
                _ => return Err("Expected boolean".into()),
            };
            Ok(Value::Bool(a || b))
        }
        BooleanOp::Xor => {
            let b = match &args[1] {
                Value::Bool(x) => *x,
                _ => return Err("Expected boolean".into()),
            };
            Ok(Value::Bool(a ^ b))
        }
        BooleanOp::Not => Ok(Value::Bool(!a)),
    }
}

// ==================== Cast ====================

fn evaluate_cast(target: CastTarget, args: &[Value]) -> Result<Value, String> {
    if args.is_empty() {
        return Err("Cast requires an argument".into());
    }
    let v = &args[0];

    match target {
        CastTarget::String => Ok(Value::String(format!("{:?}", v))),
        CastTarget::Int64 => match v {
            Value::Int64(x) => Ok(Value::Int64(*x)),
            Value::Int32(x) => Ok(Value::Int64(*x as i64)),
            Value::Int16(x) => Ok(Value::Int64(*x as i64)),
            Value::Int8(x) => Ok(Value::Int64(*x as i64)),
            Value::UInt64(x) => Ok(Value::Int64(*x as i64)),
            Value::UInt32(x) => Ok(Value::Int64(*x as i64)),
            Value::UInt16(x) => Ok(Value::Int64(*x as i64)),
            Value::UInt8(x) => Ok(Value::Int64(*x as i64)),
            Value::Double(x) => Ok(Value::Int64(*x as i64)),
            Value::Float(x) => Ok(Value::Int64(*x as i64)),
            Value::Bool(x) => Ok(Value::Int64(if *x { 1 } else { 0 })),
            Value::String(s) => s
                .parse::<i64>()
                .map(Value::Int64)
                .map_err(|e| format!("Cannot cast string to int: {e}")),
            _ => Err("Cannot cast to Int64".into()),
        },
        CastTarget::Int32 => match v {
            Value::Int32(x) => Ok(Value::Int32(*x)),
            Value::Int64(x) => Ok(Value::Int32(*x as i32)),
            Value::Int16(x) => Ok(Value::Int32(*x as i32)),
            Value::Int8(x) => Ok(Value::Int32(*x as i32)),
            Value::Double(x) => Ok(Value::Int32(*x as i32)),
            Value::Float(x) => Ok(Value::Int32(*x as i32)),
            Value::String(s) => s
                .parse::<i32>()
                .map(Value::Int32)
                .map_err(|e| format!("Cannot cast string to int32: {e}")),
            _ => Err("Cannot cast to Int32".into()),
        },
        CastTarget::Double => match v {
            Value::Int64(x) => Ok(Value::Double(*x as f64)),
            Value::Int32(x) => Ok(Value::Double(*x as f64)),
            Value::Int16(x) => Ok(Value::Double(*x as f64)),
            Value::Int8(x) => Ok(Value::Double(*x as f64)),
            Value::Double(x) => Ok(Value::Double(*x)),
            Value::Float(x) => Ok(Value::Double(*x as f64)),
            Value::String(s) => s
                .parse::<f64>()
                .map(Value::Double)
                .map_err(|e| format!("Cannot cast string to double: {e}")),
            _ => Err("Cannot cast to Double".into()),
        },
        CastTarget::Float => match v {
            Value::Float(x) => Ok(Value::Float(*x)),
            Value::Int64(x) => Ok(Value::Float(*x as f32)),
            Value::Int32(x) => Ok(Value::Float(*x as f32)),
            Value::Double(x) => Ok(Value::Float(*x as f32)),
            Value::String(s) => s
                .parse::<f32>()
                .map(Value::Float)
                .map_err(|e| format!("Cannot cast string to float: {e}")),
            _ => Err("Cannot cast to Float".into()),
        },
        CastTarget::Bool => match v {
            Value::Bool(x) => Ok(Value::Bool(*x)),
            Value::Int64(x) => Ok(Value::Bool(*x != 0)),
            Value::Int32(x) => Ok(Value::Bool(*x != 0)),
            Value::String(s) => {
                let lower = s.to_lowercase();
                match lower.as_str() {
                    "true" | "yes" | "1" => Ok(Value::Bool(true)),
                    "false" | "no" | "0" => Ok(Value::Bool(false)),
                    _ => Err(format!("Cannot cast string '{}' to Bool", s)),
                }
            }
            _ => Err("Cannot cast to Bool".into()),
        },
        CastTarget::Date => match v {
            Value::Date(x) => Ok(Value::Date(*x)),
            Value::Timestamp(t) => {
                // Convert timestamp to date by extracting days from micros
                let days = (t.0.div_euclid(1_000_000) / 86400) as i32;
                Ok(Value::Date(Date(days)))
            }
            _ => Err("Cannot cast to Date".into()),
        },
        CastTarget::Timestamp => match v {
            Value::Timestamp(x) => Ok(Value::Timestamp(*x)),
            Value::Date(d) => Ok(Value::Timestamp(Timestamp(d.0 as i64 * 86400 * 1_000_000))),
            _ => Err("Cannot cast to Timestamp".into()),
        },
        CastTarget::Interval => match v {
            Value::Interval(x) => Ok(Value::Interval(*x)),
            Value::Int64(x) => {
                // Treat as microseconds
                Ok(Value::Interval(Interval { months: 0, days: 0, micros: *x }))
            }
            _ => Err("Cannot cast to Interval".into()),
        },
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
            if args.is_empty() {
                return Ok(Value::String("NULL".into()));
            }
            Ok(Value::String(format!("{:?}", args[0].logical_type())))
        }
    }
}

// ==================== Schema Functions ====================

/// Evaluate a schema function: OFFSET, ID, START_NODE, END_NODE, LABEL.
fn evaluate_schema(op: SchemaOp, args: &[Value]) -> Result<Value, String> {
    if args.is_empty() {
        return Err(format!("Schema function {:?} requires an argument", op));
    }

    match op {
        SchemaOp::Offset => {
            // OFFSET(v) → returns the internal offset (row number) of a node/rel ID
            match &args[0] {
                Value::InternalID(id) => Ok(Value::Int64(id.offset as i64)),
                Value::Struct(entries) => {
                    // Try to extract offset from a struct with "_id" field
                    for (k, v) in entries {
                        if k == "_id" {
                            if let Value::InternalID(inner) = v {
                                return Ok(Value::Int64(inner.offset as i64));
                            }
                        }
                    }
                    Err("OFFSET: argument struct has no _id field".into())
                }
                other => Err(format!(
                    "OFFSET requires a node/rel value, got {:?}",
                    other.logical_type()
                )),
            }
        }
        SchemaOp::Id => {
            // ID(v) → returns the InternalID (offset + table_id)
            match &args[0] {
                Value::InternalID(id) => Ok(Value::InternalID(*id)),
                Value::Struct(entries) => {
                    // Try to extract id from a struct with "_id" field
                    for (k, v) in entries {
                        if k == "_id" {
                            return Ok(v.clone());
                        }
                    }
                    Err("ID: argument struct has no _id field".into())
                }
                other => Err(format!(
                    "ID requires a node/rel value, got {:?}",
                    other.logical_type()
                )),
            }
        }
        SchemaOp::StartNode => {
            // START_NODE(r) → returns the source node of a relationship
            match &args[0] {
                Value::Struct(entries) => {
                    for (k, v) in entries {
                        if k == "_src" {
                            return Ok(v.clone());
                        }
                    }
                    Err("START_NODE: rel struct has no _src field".into())
                }
                other => Err(format!(
                    "START_NODE requires a relationship value, got {:?}",
                    other.logical_type()
                )),
            }
        }
        SchemaOp::EndNode => {
            // END_NODE(r) → returns the target node of a relationship
            match &args[0] {
                Value::Struct(entries) => {
                    for (k, v) in entries {
                        if k == "_dst" {
                            return Ok(v.clone());
                        }
                    }
                    Err("END_NODE: rel struct has no _dst field".into())
                }
                other => Err(format!(
                    "END_NODE requires a relationship value, got {:?}",
                    other.logical_type()
                )),
            }
        }
        SchemaOp::Label => {
            // LABEL(v) → returns the table/label name as a string
            match &args[0] {
                Value::String(s) => Ok(Value::String(s.clone())),
                Value::Struct(entries) => {
                    // Try _label field first
                    for (k, v) in entries {
                        if k == "_label" {
                            return Ok(v.clone());
                        }
                    }
                    // Fallback: try _id and look up by table_id
                    for (k, v) in entries {
                        if k == "_id" {
                            if let Value::InternalID(id) = v {
                                return Ok(Value::String(format!("Table({})", id.table_id)));
                            }
                        }
                    }
                    Err("LABEL: argument struct has no _label field".into())
                }
                Value::InternalID(id) => {
                    Ok(Value::String(format!("Table({})", id.table_id)))
                }
                other => Err(format!(
                    "LABEL requires a node/rel/string value, got {:?}",
                    other.logical_type()
                )),
            }
        }
    }
}

// ==================== Array Math Functions ====================

/// Evaluate an array math function: cosine_similarity, distance, inner_product,
/// cross_product, squared_distance.
fn evaluate_array(op: ArrayOp, args: &[Value]) -> Result<Value, String> {
    if args.len() < 2 {
        return Err(format!("Array function {:?} requires 2 arguments", op));
    }

    /// Extract a Vec<f64> from a Value::List or return an error.
    fn extract_f64s(v: &Value) -> Result<Vec<f64>, String> {
        match v {
            Value::List(items) => {
                items.iter().map(|item| match item {
                    Value::Int64(i) => Ok(*i as f64),
                    Value::Double(f) => Ok(*f),
                    Value::Float(f) => Ok(*f as f64),
                    _ => Err(format!("Expected numeric value in array, got {:?}", item.logical_type())),
                }).collect()
            }
            _ => Err("Expected list/array".into()),
        }
    }

    let a = extract_f64s(&args[0])?;
    let b = extract_f64s(&args[1])?;

    if a.len() != b.len() {
        return Err("Arrays must have the same length".into());
    }

    match op {
        ArrayOp::CosineSimilarity => {
            let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
            let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
            let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
            if norm_a == 0.0 || norm_b == 0.0 {
                return Ok(Value::Double(1.0));
            }
            Ok(Value::Double(dot / (norm_a * norm_b)))
        }
        ArrayOp::Distance => {
            let sum_sq: f64 = a.iter().zip(b.iter()).map(|(x, y)| (x - y) * (x - y)).sum();
            Ok(Value::Double(sum_sq.sqrt()))
        }
        ArrayOp::InnerProduct => {
            let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
            Ok(Value::Double(dot))
        }
        ArrayOp::CrossProduct => {
            if a.len() != 3 || b.len() != 3 {
                return Err("Cross product requires 3D arrays".into());
            }
            let result = vec![
                Value::Double(a[1] * b[2] - a[2] * b[1]),
                Value::Double(a[2] * b[0] - a[0] * b[2]),
                Value::Double(a[0] * b[1] - a[1] * b[0]),
            ];
            Ok(Value::List(result))
        }
        ArrayOp::SquaredDistance => {
            let sum_sq: f64 = a.iter().zip(b.iter()).map(|(x, y)| (x - y) * (x - y)).sum();
            Ok(Value::Double(sum_sq))
        }
    }
}

// ==================== Aggregate ====================

/// State for aggregate function computation over Values.
#[derive(Debug, Clone)]
pub enum AggValueState {
    Count(u64),
    Sum(Value),
    Min(Value),
    Max(Value),
    Avg { sum: Value, count: u64 },
    Collect(Vec<Value>),
    StdDev { sum: f64, sum_sq: f64, count: u64 },
    Variance { sum: f64, sum_sq: f64, count: u64 },
    /// Percentile state — collects all non-null values for percentile computation.
    Percentile { values: Vec<f64>, percentile: f64 },
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
        }
    }

    /// Finalize the state into a Value.
    pub fn finalize(&self) -> Value {
        match self {
            AggValueState::Count(n) => Value::Int64(*n as i64),
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
            (AggValueState::StdDev { sum: s1, sum_sq: sq1, count: c1 },
             AggValueState::StdDev { sum: s2, sum_sq: sq2, count: c2 }) => {
                *s1 += s2;
                *sq1 += sq2;
                *c1 += c2;
            }
            (AggValueState::Variance { sum: s1, sum_sq: sq1, count: c1 },
             AggValueState::Variance { sum: s2, sum_sq: sq2, count: c2 }) => {
                *s1 += s2;
                *sq1 += sq2;
                *c1 += c2;
            }
            (AggValueState::Percentile { values: a, .. }, AggValueState::Percentile { values: b, .. }) => {
                a.extend(b.iter().cloned());
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Add };
        assert_eq!(
            evaluate_scalar(&func, &[Value::Int64(3), Value::Int64(4)]).unwrap(),
            Value::Int64(7)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::Double(1.5), Value::Double(2.5)]).unwrap(),
            Value::Double(4.0)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("a".into()), Value::String("b".into())]).unwrap(),
            Value::String("ab".into())
        );
    }

    #[test]
    fn test_sub() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Sub };
        assert_eq!(
            evaluate_scalar(&func, &[Value::Int64(10), Value::Int64(3)]).unwrap(),
            Value::Int64(7)
        );
    }

    #[test]
    fn test_mul() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Mul };
        assert_eq!(
            evaluate_scalar(&func, &[Value::Int64(5), Value::Int64(6)]).unwrap(),
            Value::Int64(30)
        );
    }

    #[test]
    fn test_div() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Div };
        assert_eq!(
            evaluate_scalar(&func, &[Value::Int64(10), Value::Int64(3)]).unwrap(),
            Value::Int64(3)
        );
    }

    #[test]
    fn test_div_by_zero() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Div };
        assert!(evaluate_scalar(&func, &[Value::Int64(1), Value::Int64(0)]).is_err());
    }

    #[test]
    fn test_mod() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Mod };
        assert_eq!(
            evaluate_scalar(&func, &[Value::Int64(10), Value::Int64(3)]).unwrap(),
            Value::Int64(1)
        );
    }

    #[test]
    fn test_abs() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Abs };
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(-5)]).unwrap(), Value::Int64(5));
    }

    #[test]
    fn test_negate() {
        let func = ScalarFunction::Arithmetic {
            op: ArithmeticOp::Negate,
        };
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(42)]).unwrap(), Value::Int64(-42));
    }

    // --- Light Math tests ---

    #[test]
    fn test_cbrt() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Cbrt };
        let get_f64 = |v: Value| -> f64 {
            if let Value::Double(d) = v { d } else { panic!("Expected Double") }
        };
        assert!((get_f64(evaluate_scalar(&func, &[Value::Double(27.0)]).unwrap()) - 3.0).abs() < 1e-10);
        assert!((get_f64(evaluate_scalar(&func, &[Value::Double(8.0)]).unwrap()) - 2.0).abs() < 1e-10);
        assert!((get_f64(evaluate_scalar(&func, &[Value::Int64(27)]).unwrap()) - 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_cot() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Cot };
        // cot(pi/4) = 1
        let pi_4 = std::f64::consts::PI / 4.0;
        let result = evaluate_scalar(&func, &[Value::Double(pi_4)]).unwrap();
        if let Value::Double(v) = result {
            assert!((v - 1.0).abs() < 1e-10);
        } else {
            panic!("Expected Double");
        }
    }

    #[test]
    fn test_log2() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Log2 };
        let get_f64 = |v: Value| -> f64 {
            if let Value::Double(d) = v { d } else { panic!("Expected Double") }
        };
        assert!((get_f64(evaluate_scalar(&func, &[Value::Double(8.0)]).unwrap()) - 3.0).abs() < 1e-10);
        assert!((get_f64(evaluate_scalar(&func, &[Value::Int64(16)]).unwrap()) - 4.0).abs() < 1e-10);
    }

    #[test]
    fn test_even() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Even };
        // Int64: even numbers unchanged, odd rounded up
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(4)]).unwrap(), Value::Int64(4));
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(5)]).unwrap(), Value::Int64(6));
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(-2)]).unwrap(), Value::Int64(-2));
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(-3)]).unwrap(), Value::Int64(-2));
        // Double
        assert_eq!(evaluate_scalar(&func, &[Value::Double(2.3)]).unwrap(), Value::Int64(4));
        assert_eq!(evaluate_scalar(&func, &[Value::Double(3.8)]).unwrap(), Value::Int64(4));
    }

    // --- Heavy Math tests ---

    #[test]
    fn test_factorial() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Factorial };
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(0)]).unwrap(), Value::Int64(1));
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(1)]).unwrap(), Value::Int64(1));
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(5)]).unwrap(), Value::Int64(120));
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(10)]).unwrap(), Value::Int64(3628800));
        // Negative input
        assert!(evaluate_scalar(&func, &[Value::Int64(-1)]).is_err());
    }

    #[test]
    fn test_gamma() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Gamma };
        let get_f64 = |v: Value| -> f64 {
            if let Value::Double(d) = v { d } else { panic!("Expected Double") }
        };
        // Gamma(1) = 1
        assert!((get_f64(evaluate_scalar(&func, &[Value::Double(1.0)]).unwrap()) - 1.0).abs() < 1e-10);
        // Gamma(2) = 1
        assert!((get_f64(evaluate_scalar(&func, &[Value::Double(2.0)]).unwrap()) - 1.0).abs() < 1e-10);
        // Gamma(3) = 2! = 2
        assert!((get_f64(evaluate_scalar(&func, &[Value::Double(3.0)]).unwrap()) - 2.0).abs() < 1e-8);
        // Non-positive integer → infinity
        assert_eq!(
            evaluate_scalar(&func, &[Value::Double(0.0)]).unwrap(),
            Value::Double(f64::INFINITY)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::Double(-1.0)]).unwrap(),
            Value::Double(f64::INFINITY)
        );
    }

    #[test]
    fn test_lgamma() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::Lgamma };
        let get_f64 = |v: Value| -> f64 {
            if let Value::Double(d) = v { d } else { panic!("Expected Double") }
        };
        // ln(Gamma(1)) = ln(1) = 0
        assert!((get_f64(evaluate_scalar(&func, &[Value::Double(1.0)]).unwrap()) - 0.0).abs() < 1e-10);
        // ln(Gamma(2)) = ln(1) = 0
        assert!((get_f64(evaluate_scalar(&func, &[Value::Double(2.0)]).unwrap()) - 0.0).abs() < 1e-10);
        // Non-positive integer → infinity
        assert_eq!(
            evaluate_scalar(&func, &[Value::Double(0.0)]).unwrap(),
            Value::Double(f64::INFINITY)
        );
    }

    #[test]
    fn test_set_seed() {
        let set_seed = ScalarFunction::Arithmetic { op: ArithmeticOp::SetSeed };
        let rand_func = ScalarFunction::Arithmetic { op: ArithmeticOp::Rand };
        // Set seed to known value
        assert_eq!(
            evaluate_scalar(&set_seed, &[Value::Double(0.5)]).unwrap(),
            Value::Int32(0)
        );
        let first = evaluate_scalar(&rand_func, &[]).unwrap();
        // Same seed should produce same sequence
        assert_eq!(
            evaluate_scalar(&set_seed, &[Value::Double(0.5)]).unwrap(),
            Value::Int32(0)
        );
        let second = evaluate_scalar(&rand_func, &[]).unwrap();
        assert_eq!(first, second);
    }

    // --- Hash function tests ---

    #[test]
    fn test_md5() {
        let func = ScalarFunction::Hash { op: HashOp::Md5 };
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("hello".into())]).unwrap(),
            Value::String("5d41402abc4b2a76b9719d911017c592".into())
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("".into())]).unwrap(),
            Value::String("d41d8cd98f00b204e9800998ecf8427e".into())
        );
    }

    #[test]
    fn test_sha256() {
        let func = ScalarFunction::Hash { op: HashOp::Sha256 };
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("hello".into())]).unwrap(),
            Value::String(
                "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".into()
            )
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("".into())]).unwrap(),
            Value::String(
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into()
            )
        );
    }

    #[test]
    fn test_hash_generic() {
        let func = ScalarFunction::Hash { op: HashOp::Hash };
        let h1 = evaluate_scalar(&func, &[Value::Int64(42)]).unwrap();
        let h2 = evaluate_scalar(&func, &[Value::Int64(42)]).unwrap();
        assert_eq!(h1, h2);
        let h3 = evaluate_scalar(&func, &[Value::Int64(43)]).unwrap();
        assert_ne!(h1, h3);
        let hs = evaluate_scalar(&func, &[Value::String("test".into())]).unwrap();
        assert!(matches!(hs, Value::Int64(_)));
    }

    // --- Regex string function tests ---

    #[test]
    fn test_regexp_full_match() {
        let func = ScalarFunction::String { op: StringOp::RegexpFullMatch };
        // Full match
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("hello".into()), Value::String("hello".into())]).unwrap(),
            Value::Bool(true)
        );
        // Partial match should be false
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("hello123".into()), Value::String(r"\d+".into())]).unwrap(),
            Value::Bool(false)
        );
        // Full match with pattern
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("123".into()), Value::String(r"\d+".into())]).unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn test_regexp_extract() {
        let func = ScalarFunction::String { op: StringOp::RegexpExtract };
        // Extract first digit sequence
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("abc123def".into()), Value::String(r"\d+".into())]).unwrap(),
            Value::String("123".into())
        );
        // No match
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("abcdef".into()), Value::String(r"\d+".into())]).unwrap(),
            Value::String("".into())
        );
        // With capture group (0-based: group 0 = full match)
        assert_eq!(
            evaluate_scalar(&func, &[
                Value::String("hello@example.com".into()),
                Value::String(r"(\w+)@(\w+\.\w+)".into()),
                Value::Int64(1),
            ]).unwrap(),
            Value::String("hello".into())
        );
        assert_eq!(
            evaluate_scalar(&func, &[
                Value::String("hello@example.com".into()),
                Value::String(r"(\w+)@(\w+\.\w+)".into()),
                Value::Int64(2),
            ]).unwrap(),
            Value::String("example.com".into())
        );
    }

    #[test]
    fn test_regexp_extract_all() {
        let func = ScalarFunction::String { op: StringOp::RegexpExtractAll };
        // Extract all digits
        let result = evaluate_scalar(&func, &[Value::String("a1b2c3".into()), Value::String(r"\d+".into())]).unwrap();
        assert_eq!(result, Value::List(vec![
            Value::String("1".into()),
            Value::String("2".into()),
            Value::String("3".into()),
        ]));
        // With group
        let result = evaluate_scalar(&func, &[
            Value::String("a1b2c3".into()),
            Value::String(r"(\d)".into()),
            Value::Int64(1),
        ]).unwrap();
        assert_eq!(result, Value::List(vec![
            Value::String("1".into()),
            Value::String("2".into()),
            Value::String("3".into()),
        ]));
        // No matches
        let result = evaluate_scalar(&func, &[Value::String("abc".into()), Value::String(r"\d+".into())]).unwrap();
        assert_eq!(result, Value::List(vec![]));
    }

    #[test]
    fn test_regexp_split_to_array() {
        let func = ScalarFunction::String { op: StringOp::RegexpSplitToArray };
        // Split on digits
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("a1b2c".into()), Value::String(r"\d".into())]).unwrap(),
            Value::List(vec![
                Value::String("a".into()),
                Value::String("b".into()),
                Value::String("c".into()),
            ])
        );
        // No match: single element
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("abc".into()), Value::String(r"\d+".into())]).unwrap(),
            Value::List(vec![Value::String("abc".into())])
        );
    }

    #[test]
    fn test_levenshtein() {
        let func = ScalarFunction::String { op: StringOp::Levenshtein };
        // Same strings
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("hello".into()), Value::String("hello".into())]).unwrap(),
            Value::Int64(0)
        );
        // One substitution
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("kitten".into()), Value::String("sitten".into())]).unwrap(),
            Value::Int64(1)
        );
        // Known distance
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("kitten".into()), Value::String("sitting".into())]).unwrap(),
            Value::Int64(3)
        );
        // Empty string
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("".into()), Value::String("abc".into())]).unwrap(),
            Value::Int64(3)
        );
    }

    // --- Bitwise tests ---

    #[test]
    fn test_bitwise_and() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::BitwiseAnd };
        assert_eq!(
            evaluate_scalar(&func, &[Value::Int64(6), Value::Int64(3)]).unwrap(),
            Value::Int64(2)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::Int64(0xFF), Value::Int64(0x0F)]).unwrap(),
            Value::Int64(0x0F)
        );
        // Error: non-integer
        assert!(evaluate_scalar(&func, &[Value::Double(1.0), Value::Int64(2)]).is_err());
        // Error: wrong number of args
        assert!(evaluate_scalar(&func, &[Value::Int64(1)]).is_err());
    }

    #[test]
    fn test_bitwise_or() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::BitwiseOr };
        assert_eq!(
            evaluate_scalar(&func, &[Value::Int64(6), Value::Int64(3)]).unwrap(),
            Value::Int64(7)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::Int64(0xF0), Value::Int64(0x0F)]).unwrap(),
            Value::Int64(0xFF)
        );
        assert!(evaluate_scalar(&func, &[Value::Int64(1)]).is_err());
    }

    #[test]
    fn test_bitwise_xor() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::BitwiseXor };
        assert_eq!(
            evaluate_scalar(&func, &[Value::Int64(6), Value::Int64(3)]).unwrap(),
            Value::Int64(5)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::Int64(0xFF), Value::Int64(0x0F)]).unwrap(),
            Value::Int64(0xF0)
        );
        assert!(evaluate_scalar(&func, &[Value::Int64(1)]).is_err());
    }

    #[test]
    fn test_bit_shift_left() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::BitShiftLeft };
        assert_eq!(
            evaluate_scalar(&func, &[Value::Int64(1), Value::Int64(3)]).unwrap(),
            Value::Int64(8)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::Int64(0xFF), Value::Int64(4)]).unwrap(),
            Value::Int64(0xFF0)
        );
        assert!(evaluate_scalar(&func, &[Value::Int64(1)]).is_err());
    }

    #[test]
    fn test_bit_shift_right() {
        let func = ScalarFunction::Arithmetic { op: ArithmeticOp::BitShiftRight };
        assert_eq!(
            evaluate_scalar(&func, &[Value::Int64(8), Value::Int64(3)]).unwrap(),
            Value::Int64(1)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::Int64(0xFF0), Value::Int64(4)]).unwrap(),
            Value::Int64(0xFF)
        );
        assert!(evaluate_scalar(&func, &[Value::Int64(1)]).is_err());
    }

    #[test]
    fn test_comparison_eq() {
        let func = ScalarFunction::Comparison { op: ComparisonOp::Eq };
        assert_eq!(
            evaluate_scalar(&func, &[Value::Int64(1), Value::Int64(1)]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::Int64(1), Value::Int64(2)]).unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn test_comparison_gt() {
        let func = ScalarFunction::Comparison { op: ComparisonOp::Gt };
        assert_eq!(
            evaluate_scalar(&func, &[Value::Int64(5), Value::Int64(3)]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::Int64(3), Value::Int64(5)]).unwrap(),
            Value::Bool(false)
        );
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
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("hello".into())]).unwrap(),
            Value::String("HELLO".into())
        );
    }

    #[test]
    fn test_string_to_lower() {
        let func = ScalarFunction::String { op: StringOp::ToLower };
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("HELLO".into())]).unwrap(),
            Value::String("hello".into())
        );
    }

    #[test]
    fn test_string_trim() {
        let func = ScalarFunction::String { op: StringOp::Trim };
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("  hello  ".into())]).unwrap(),
            Value::String("hello".into())
        );
    }

    #[test]
    fn test_string_length() {
        let func = ScalarFunction::String { op: StringOp::Length };
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("hello".into())]).unwrap(),
            Value::Int64(5)
        );
    }

    #[test]
    fn test_string_contains() {
        let func = ScalarFunction::String { op: StringOp::Contains };
        assert_eq!(
            evaluate_scalar(
                &func,
                &[Value::String("hello world".into()), Value::String("world".into())]
            )
            .unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn test_boolean_and() {
        let func = ScalarFunction::Boolean { op: BooleanOp::And };
        assert_eq!(
            evaluate_scalar(&func, &[Value::Bool(true), Value::Bool(true)]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::Bool(true), Value::Bool(false)]).unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn test_boolean_or() {
        let func = ScalarFunction::Boolean { op: BooleanOp::Or };
        assert_eq!(
            evaluate_scalar(&func, &[Value::Bool(true), Value::Bool(false)]).unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn test_boolean_not() {
        let func = ScalarFunction::Boolean { op: BooleanOp::Not };
        assert_eq!(
            evaluate_scalar(&func, &[Value::Bool(true)]).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::Bool(false)]).unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn test_is_null() {
        let func = ScalarFunction::Comparison {
            op: ComparisonOp::IsNull,
        };
        assert_eq!(evaluate_scalar(&func, &[Value::Null]).unwrap(), Value::Bool(true));
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(5)]).unwrap(), Value::Bool(false));
    }

    #[test]
    fn test_coalesce() {
        let func = ScalarFunction::Utility {
            op: UtilityOp::Coalesce,
        };
        assert_eq!(
            evaluate_scalar(&func, &[Value::Null, Value::Int64(42)]).unwrap(),
            Value::Int64(42)
        );
    }

    #[test]
    fn test_cast_int64() {
        let func = ScalarFunction::Cast {
            target_type: CastTarget::Int64,
        };
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(42)]).unwrap(), Value::Int64(42));
        assert_eq!(evaluate_scalar(&func, &[Value::Double(3.14)]).unwrap(), Value::Int64(3));
    }

    #[test]
    fn test_cast_string() {
        let func = ScalarFunction::Cast {
            target_type: CastTarget::String,
        };
        let result = evaluate_scalar(&func, &[Value::Int64(42)]).unwrap();
        assert!(matches!(result, Value::String(_)));
    }

    #[test]
    fn test_list_len() {
        let func = ScalarFunction::List { op: ListOp::Len };
        assert_eq!(
            evaluate_scalar(&func, &[Value::List(vec![Value::Int64(1), Value::Int64(2)])]).unwrap(),
            Value::Int64(2)
        );
    }

    #[test]
    fn test_list_contains() {
        let func = ScalarFunction::List { op: ListOp::Contains };
        let list = Value::List(vec![Value::Int64(1), Value::Int64(2), Value::Int64(3)]);
        assert_eq!(
            evaluate_scalar(&func, &[list.clone(), Value::Int64(2)]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_scalar(&func, &[list, Value::Int64(99)]).unwrap(),
            Value::Bool(false)
        );
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
        let func = ScalarFunction::String {
            op: StringOp::RegexMatches,
        };
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
        let func = ScalarFunction::String {
            op: StringOp::RegexReplace,
        };
        assert_eq!(
            evaluate_scalar(
                &func,
                &[
                    Value::String("hello 123 world".into()),
                    Value::String(r"\d+".into()),
                    Value::String("NUM".into())
                ]
            )
            .unwrap(),
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

    // ==================== New Fase 1 Tests ====================

    // --- String function tests ---
    #[test]
    fn test_string_reverse() {
        let func = ScalarFunction::String { op: StringOp::Reverse };
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("hello".into())]).unwrap(),
            Value::String("olleh".into())
        );
    }

    #[test]
    fn test_string_repeat() {
        let func = ScalarFunction::String { op: StringOp::Repeat };
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("ab".into()), Value::Int64(3)]).unwrap(),
            Value::String("ababab".into())
        );
    }

    #[test]
    fn test_string_replace() {
        let func = ScalarFunction::String { op: StringOp::Replace };
        assert_eq!(
            evaluate_scalar(
                &func,
                &[
                    Value::String("hello world".into()),
                    Value::String("world".into()),
                    Value::String("there".into())
                ]
            )
            .unwrap(),
            Value::String("hello there".into())
        );
    }

    #[test]
    fn test_string_substring() {
        let func = ScalarFunction::String {
            op: StringOp::Substring,
        };
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("hello".into()), Value::Int64(2)]).unwrap(),
            Value::String("ello".into())
        );
        assert_eq!(
            evaluate_scalar(
                &func,
                &[Value::String("hello".into()), Value::Int64(1), Value::Int64(3)]
            )
            .unwrap(),
            Value::String("hel".into())
        );
    }

    #[test]
    fn test_string_starts_ends_with() {
        let starts = ScalarFunction::String {
            op: StringOp::StartsWith,
        };
        let ends = ScalarFunction::String { op: StringOp::EndsWith };
        assert_eq!(
            evaluate_scalar(&starts, &[Value::String("hello".into()), Value::String("he".into())]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_scalar(&ends, &[Value::String("hello".into()), Value::String("lo".into())]).unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn test_string_trim_variants() {
        let ltrim = ScalarFunction::String { op: StringOp::LTrim };
        let rtrim = ScalarFunction::String { op: StringOp::RTrim };
        assert_eq!(
            evaluate_scalar(&ltrim, &[Value::String("  hello".into())]).unwrap(),
            Value::String("hello".into())
        );
        assert_eq!(
            evaluate_scalar(&rtrim, &[Value::String("hello  ".into())]).unwrap(),
            Value::String("hello".into())
        );
    }

    // --- String basic tests (C++ port) ---

    #[test]
    fn test_initcap() {
        let func = ScalarFunction::String { op: StringOp::InitCap };
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("hello world".into())]).unwrap(),
            Value::String("Hello world".into())
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("HELLO".into())]).unwrap(),
            Value::String("Hello".into())
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("".into())]).unwrap(),
            Value::String("".into())
        );
    }

    #[test]
    fn test_concat_ws() {
        let func = ScalarFunction::String { op: StringOp::ConcatWs };
        // Basic concat
        assert_eq!(
            evaluate_scalar(&func, &[
                Value::String(",".into()),
                Value::String("a".into()),
                Value::String("b".into()),
                Value::String("c".into()),
            ]).unwrap(),
            Value::String("a,b,c".into())
        );
        // Skip NULL
        assert_eq!(
            evaluate_scalar(&func, &[
                Value::String("-".into()),
                Value::String("a".into()),
                Value::Null,
                Value::String("b".into()),
            ]).unwrap(),
            Value::String("a-b".into())
        );
        // Single element (no separator)
        assert_eq!(
            evaluate_scalar(&func, &[
                Value::String(",".into()),
                Value::String("only".into()),
            ]).unwrap(),
            Value::String("only".into())
        );
    }

    #[test]
    fn test_split_part() {
        let func = ScalarFunction::String { op: StringOp::SplitPart };
        // Normal case
        assert_eq!(
            evaluate_scalar(&func, &[
                Value::String("a,b,c".into()),
                Value::String(",".into()),
                Value::Int64(2),
            ]).unwrap(),
            Value::String("b".into())
        );
        // Out of range (too high)
        assert_eq!(
            evaluate_scalar(&func, &[
                Value::String("a,b".into()),
                Value::String(",".into()),
                Value::Int64(5),
            ]).unwrap(),
            Value::String("".into())
        );
        // Index <= 0
        assert_eq!(
            evaluate_scalar(&func, &[
                Value::String("a,b".into()),
                Value::String(",".into()),
                Value::Int64(0),
            ]).unwrap(),
            Value::String("".into())
        );
    }

    #[test]
    fn test_array_extract() {
        let func = ScalarFunction::String { op: StringOp::ArrayExtract };
        // Positive 1-based index
        assert_eq!(
            evaluate_scalar(&func, &[
                Value::String("hello".into()),
                Value::Int64(1),
            ]).unwrap(),
            Value::String("h".into())
        );
        assert_eq!(
            evaluate_scalar(&func, &[
                Value::String("hello".into()),
                Value::Int64(5),
            ]).unwrap(),
            Value::String("o".into())
        );
        // Index 0 returns empty
        assert_eq!(
            evaluate_scalar(&func, &[
                Value::String("hello".into()),
                Value::Int64(0),
            ]).unwrap(),
            Value::String("".into())
        );
        // Negative index (from end)
        assert_eq!(
            evaluate_scalar(&func, &[
                Value::String("hello".into()),
                Value::Int64(-1),
            ]).unwrap(),
            Value::String("o".into())
        );
        assert_eq!(
            evaluate_scalar(&func, &[
                Value::String("hello".into()),
                Value::Int64(-2),
            ]).unwrap(),
            Value::String("l".into())
        );
    }

    // --- Date function tests ---
    #[test]
    fn test_date_current() {
        let cur_date = ScalarFunction::Date {
            op: DateOp::CurrentDate,
        };
        let cur_ts = ScalarFunction::Date {
            op: DateOp::CurrentTimestamp,
        };
        let d = evaluate_scalar(&cur_date, &[]).unwrap();
        let ts = evaluate_scalar(&cur_ts, &[]).unwrap();
        assert!(matches!(d, Value::Date(_)));
        assert!(matches!(ts, Value::Timestamp(_)));
    }

    #[test]
    fn test_date_year_month_day() {
        // Use a known date: 2023-06-15 = days since epoch ~ 19523
        let date_val = Value::Date(Date(19523)); // approx 2023-06-15
        let year = ScalarFunction::Date { op: DateOp::Year };
        let month = ScalarFunction::Date { op: DateOp::Month };
        let day = ScalarFunction::Date { op: DateOp::Day };
        assert_eq!(evaluate_scalar(&year, &[date_val.clone()]).unwrap(), Value::Int64(2023));
        assert_eq!(evaluate_scalar(&month, &[date_val.clone()]).unwrap(), Value::Int64(6));
        assert_eq!(evaluate_scalar(&day, &[date_val]).unwrap(), Value::Int64(15));
    }

    #[test]
    fn test_date_part() {
        let date_val = Value::Date(Date(19523)); // 2023-06-15
        let dp = ScalarFunction::Date { op: DateOp::DatePart };
        assert_eq!(
            evaluate_scalar(&dp, &[Value::String("year".into()), date_val.clone()]).unwrap(),
            Value::Int64(2023)
        );
        assert_eq!(
            evaluate_scalar(&dp, &[Value::String("month".into()), date_val.clone()]).unwrap(),
            Value::Int64(6)
        );
        assert_eq!(
            evaluate_scalar(&dp, &[Value::String("day".into()), date_val]).unwrap(),
            Value::Int64(15)
        );
    }

    #[test]
    fn test_date_trunc() {
        let date_val = Value::Date(Date(19600)); // some date in 2023
        let dt = ScalarFunction::Date { op: DateOp::DateTrunc };
        let result = evaluate_scalar(&dt, &[Value::String("year".into()), date_val]).unwrap();
        assert!(matches!(result, Value::Date(_)));
    }

    #[test]
    fn test_date_diff() {
        let d1 = Value::Date(Date(19000));
        let d2 = Value::Date(Date(19500));
        let dd = ScalarFunction::Date { op: DateOp::DateDiff };
        let days = evaluate_scalar(&dd, &[Value::String("day".into()), d1, d2]).unwrap();
        assert_eq!(days, Value::Int64(500));
    }

    #[test]
    fn test_date_add() {
        let date_val = Value::Date(Date(19523)); // 2023-06-15
        let da = ScalarFunction::Date { op: DateOp::DateAdd };
        let result = evaluate_scalar(&da, &[Value::String("day".into()), Value::Int64(7), date_val]).unwrap();
        assert!(matches!(result, Value::Date(_)));
    }

    // --- Timestamp function tests ---

    #[test]
    fn test_century() {
        let func = ScalarFunction::Date { op: DateOp::Century };
        // Use a date in the 21st century (e.g., 2023-06-15 = ~19523 days from epoch)
        let date_val = Value::Date(Date(19523));
        assert_eq!(evaluate_scalar(&func, &[date_val]).unwrap(), Value::Int64(21));
        // Year 2000 → century 20 (2000-01-01 = ~10957 days from epoch)
        let y2000 = Value::Date(Date(10957));
        assert_eq!(evaluate_scalar(&func, &[y2000]).unwrap(), Value::Int64(20));
        // Also works with timestamp
        let ts = Value::Timestamp(Timestamp(19523i64 * 86400 * 1_000_000));
        assert_eq!(evaluate_scalar(&func, &[ts]).unwrap(), Value::Int64(21));
    }

    #[test]
    fn test_epoch_ms() {
        let func = ScalarFunction::Date { op: DateOp::EpochMs };
        // 0 ms → epoch
        let result = evaluate_scalar(&func, &[Value::Int64(0)]).unwrap();
        assert_eq!(result, Value::Timestamp(Timestamp(0)));
        // 1000 ms = 1 sec → Timestamp(1_000_000 micros)
        let result = evaluate_scalar(&func, &[Value::Int64(1000)]).unwrap();
        assert_eq!(result, Value::Timestamp(Timestamp(1_000_000)));
    }

    #[test]
    fn test_to_timestamp() {
        let func = ScalarFunction::Date { op: DateOp::ToTimestamp };
        // 0 seconds → epoch
        let result = evaluate_scalar(&func, &[Value::Double(0.0)]).unwrap();
        assert_eq!(result, Value::Timestamp(Timestamp(0)));
        // 1 second → 1_000_000 micros
        let result = evaluate_scalar(&func, &[Value::Double(1.0)]).unwrap();
        assert_eq!(result, Value::Timestamp(Timestamp(1_000_000)));
        // Integer input
        let result = evaluate_scalar(&func, &[Value::Int64(0)]).unwrap();
        assert_eq!(result, Value::Timestamp(Timestamp(0)));
    }

    #[test]
    fn test_to_epoch_ms() {
        let func = ScalarFunction::Date { op: DateOp::ToEpochMs };
        // Epoch → 0 ms
        let result = evaluate_scalar(&func, &[Value::Timestamp(Timestamp(0))]).unwrap();
        assert_eq!(result, Value::Int64(0));
        // 1 ms = 1000 micros → 1 ms
        let result = evaluate_scalar(&func, &[Value::Timestamp(Timestamp(1000))]).unwrap();
        assert_eq!(result, Value::Int64(1));
    }

    // --- Interval constructor function tests ---

    #[test]
    fn test_to_years() {
        let func = ScalarFunction::Interval { op: IntervalOp::ToYears };
        let result = evaluate_scalar(&func, &[Value::Int64(3)]).unwrap();
        assert_eq!(result, Value::Interval(Interval::new(36, 0, 0)));
    }

    #[test]
    fn test_to_months() {
        let func = ScalarFunction::Interval { op: IntervalOp::ToMonths };
        let result = evaluate_scalar(&func, &[Value::Int64(5)]).unwrap();
        assert_eq!(result, Value::Interval(Interval::new(5, 0, 0)));
    }

    #[test]
    fn test_to_days() {
        let func = ScalarFunction::Interval { op: IntervalOp::ToDays };
        let result = evaluate_scalar(&func, &[Value::Int64(10)]).unwrap();
        assert_eq!(result, Value::Interval(Interval::new(0, 10, 0)));
    }

    #[test]
    fn test_to_hours() {
        let func = ScalarFunction::Interval { op: IntervalOp::ToHours };
        let result = evaluate_scalar(&func, &[Value::Int64(2)]).unwrap();
        assert_eq!(result, Value::Interval(Interval::new(0, 0, 7_200_000_000)));
    }

    #[test]
    fn test_to_minutes() {
        let func = ScalarFunction::Interval { op: IntervalOp::ToMinutes };
        let result = evaluate_scalar(&func, &[Value::Int64(30)]).unwrap();
        assert_eq!(result, Value::Interval(Interval::new(0, 0, 1_800_000_000)));
    }

    #[test]
    fn test_to_seconds() {
        let func = ScalarFunction::Interval { op: IntervalOp::ToSeconds };
        let result = evaluate_scalar(&func, &[Value::Int64(45)]).unwrap();
        assert_eq!(result, Value::Interval(Interval::new(0, 0, 45_000_000)));
    }

    #[test]
    fn test_to_milliseconds() {
        let func = ScalarFunction::Interval { op: IntervalOp::ToMilliseconds };
        let result = evaluate_scalar(&func, &[Value::Int64(500)]).unwrap();
        assert_eq!(result, Value::Interval(Interval::new(0, 0, 500_000)));
    }

    #[test]
    fn test_to_microseconds() {
        let func = ScalarFunction::Interval { op: IntervalOp::ToMicroseconds };
        let result = evaluate_scalar(&func, &[Value::Int64(999)]).unwrap();
        assert_eq!(result, Value::Interval(Interval::new(0, 0, 999)));
    }

    // --- Blob function tests ---

    #[test]
    fn test_encode() {
        let func = ScalarFunction::Blob { op: BlobOp::Encode };
        let result = evaluate_scalar(&func, &[Value::String("hello".into())]).unwrap();
        assert_eq!(result, Value::Blob(b"hello".to_vec()));
        // Empty string
        let result = evaluate_scalar(&func, &[Value::String("".into())]).unwrap();
        assert_eq!(result, Value::Blob(vec![]));
    }

    #[test]
    fn test_decode() {
        let func = ScalarFunction::Blob { op: BlobOp::Decode };
        let result = evaluate_scalar(&func, &[Value::Blob(b"hello".to_vec())]).unwrap();
        assert_eq!(result, Value::String("hello".into()));
        // Invalid UTF-8 should error
        let result = evaluate_scalar(&func, &[Value::Blob(vec![0xFF, 0xFE])]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("UTF8"));
    }

    #[test]
    fn test_octet_length() {
        let func = ScalarFunction::Blob { op: BlobOp::OctetLength };
        let result = evaluate_scalar(&func, &[Value::Blob(b"hello".to_vec())]).unwrap();
        assert_eq!(result, Value::Int64(5));
        // Empty blob
        let result = evaluate_scalar(&func, &[Value::Blob(vec![])]).unwrap();
        assert_eq!(result, Value::Int64(0));
    }

    // --- Union function tests ---

    #[test]
    fn test_union_value() {
        let func = ScalarFunction::Union { op: UnionOp::UnionValue };
        let result = evaluate_scalar(&func, &[Value::Int64(42)]).unwrap();
        // Should produce a union wrapping the value
        assert_eq!(result, Value::Struct(vec![
            ("tag".to_string(), Value::UInt16(0)),
            ("_value".to_string(), Value::Int64(42)),
        ]));
    }

    #[test]
    fn test_union_tag() {
        // Create a union with tag=1 (second variant active)
        let union_val = Value::Struct(vec![
            ("tag".to_string(), Value::UInt16(1)),
            ("a".to_string(), Value::Int64(10)),
            ("b".to_string(), Value::String("hello".into())),
        ]);
        let func = ScalarFunction::Union { op: UnionOp::UnionTag };
        let result = evaluate_scalar(&func, &[union_val]).unwrap();
        assert_eq!(result, Value::String("b".into()));
    }

    #[test]
    fn test_union_extract() {
        let union_val = Value::Struct(vec![
            ("tag".to_string(), Value::UInt16(0)),
            ("a".to_string(), Value::Int64(10)),
            ("b".to_string(), Value::String("hello".into())),
        ]);
        let func = ScalarFunction::Union { op: UnionOp::UnionExtract };
        // Extract field "a"
        let result = evaluate_scalar(&func, &[union_val.clone(), Value::String("a".into())]).unwrap();
        assert_eq!(result, Value::Int64(10));
        // Extract field "b"
        let result = evaluate_scalar(&func, &[union_val.clone(), Value::String("b".into())]).unwrap();
        assert_eq!(result, Value::String("hello".into()));
        // Non-existent key
        let result = evaluate_scalar(&func, &[union_val, Value::String("c".into())]);
        assert!(result.is_err());
    }

    // --- List function tests ---
    #[test]
    fn test_list_creation() {
        let func = ScalarFunction::List { op: ListOp::Creation };
        let result = evaluate_scalar(&func, &[Value::Int64(1), Value::Int64(2), Value::Int64(3)]).unwrap();
        match result {
            Value::List(items) => assert_eq!(items.len(), 3),
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn test_array_value() {
        // array_value is an alias for ListOp::Creation
        let func = ScalarFunction::List { op: ListOp::Creation };
        let result = evaluate_scalar(&func, &[
            Value::Int64(10),
            Value::Int64(20),
            Value::Int64(30),
            Value::Int64(40),
        ]).unwrap();
        match result {
            Value::List(items) => {
                assert_eq!(items.len(), 4);
                assert_eq!(items[0], Value::Int64(10));
                assert_eq!(items[2], Value::Int64(30));
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn test_list_concat() {
        let func = ScalarFunction::List { op: ListOp::Concat };
        let l1 = Value::List(vec![Value::Int64(1), Value::Int64(2)]);
        let l2 = Value::List(vec![Value::Int64(3), Value::Int64(4)]);
        let result = evaluate_scalar(&func, &[l1, l2]).unwrap();
        match result {
            Value::List(items) => {
                assert_eq!(items.len(), 4);
                assert_eq!(items[0], Value::Int64(1));
                assert_eq!(items[3], Value::Int64(4));
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn test_list_sort() {
        let func = ScalarFunction::List { op: ListOp::Sort };
        let list = Value::List(vec![Value::Int64(3), Value::Int64(1), Value::Int64(2)]);
        let result = evaluate_scalar(&func, &[list]).unwrap();
        match result {
            Value::List(items) => {
                assert_eq!(items[0], Value::Int64(1));
                assert_eq!(items[1], Value::Int64(2));
                assert_eq!(items[2], Value::Int64(3));
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn test_list_prepend() {
        let func = ScalarFunction::List { op: ListOp::Prepend };
        let result = evaluate_scalar(
            &func,
            &[Value::List(vec![Value::Int64(2), Value::Int64(3)]), Value::Int64(1)],
        )
        .unwrap();
        match result {
            Value::List(items) => {
                assert_eq!(items[0], Value::Int64(1));
                assert_eq!(items.len(), 3);
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn test_list_reverse() {
        let func = ScalarFunction::List { op: ListOp::Reverse };
        let result = evaluate_scalar(
            &func,
            &[Value::List(vec![Value::Int64(1), Value::Int64(2), Value::Int64(3)])],
        )
        .unwrap();
        match result {
            Value::List(items) => {
                assert_eq!(items[0], Value::Int64(3));
                assert_eq!(items[2], Value::Int64(1));
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn test_list_extract() {
        let func = ScalarFunction::List { op: ListOp::Extract };
        let list = Value::List(vec![Value::Int64(10), Value::Int64(20), Value::Int64(30)]);
        assert_eq!(
            evaluate_scalar(&func, &[list, Value::Int64(2)]).unwrap(),
            Value::Int64(20)
        );
    }

    // --- Map function tests ---
    #[test]
    fn test_map_creation() {
        let func = ScalarFunction::Map { op: MapOp::Creation };
        let result = evaluate_scalar(
            &func,
            &[
                Value::String("a".into()),
                Value::Int64(1),
                Value::String("b".into()),
                Value::Int64(2),
            ],
        )
        .unwrap();
        match result {
            Value::Struct(entries) => {
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].0, "a");
                assert_eq!(entries[1].0, "b");
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_map_extract() {
        let func = ScalarFunction::Map { op: MapOp::Extract };
        let map_val = Value::Struct(vec![
            ("x".into(), Value::Int64(42)),
            ("y".into(), Value::String("hello".into())),
        ]);
        assert_eq!(
            evaluate_scalar(&func, &[map_val.clone(), Value::String("x".into())]).unwrap(),
            Value::Int64(42)
        );
        assert_eq!(
            evaluate_scalar(&func, &[map_val, Value::String("y".into())]).unwrap(),
            Value::String("hello".into())
        );
    }

    #[test]
    fn test_map_contains() {
        let func = ScalarFunction::Map { op: MapOp::Contains };
        let map_val = Value::Struct(vec![("a".into(), Value::Int64(1))]);
        assert_eq!(
            evaluate_scalar(&func, &[map_val.clone(), Value::String("a".into())]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_scalar(&func, &[map_val, Value::String("b".into())]).unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn test_map_keys_values() {
        let keys = ScalarFunction::Map { op: MapOp::Keys };
        let values = ScalarFunction::Map { op: MapOp::Values };
        let map_val = Value::Struct(vec![("a".into(), Value::Int64(1)), ("b".into(), Value::Int64(2))]);

        let key_result = evaluate_scalar(&keys, &[map_val.clone()]).unwrap();
        match key_result {
            Value::List(items) => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0], Value::String("a".into()));
            }
            _ => panic!("Expected list"),
        }

        let val_result = evaluate_scalar(&values, &[map_val]).unwrap();
        match val_result {
            Value::List(items) => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0], Value::Int64(1));
            }
            _ => panic!("Expected list"),
        }
    }

    // --- Struct function tests ---
    #[test]
    fn test_struct_creation() {
        let func = ScalarFunction::Struct { op: StructOp::Creation };
        let result = evaluate_scalar(
            &func,
            &[
                Value::String("name".into()),
                Value::String("Alice".into()),
                Value::String("age".into()),
                Value::Int64(30),
            ],
        )
        .unwrap();
        match result {
            Value::Struct(entries) => {
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].0, "name");
                assert_eq!(entries[1].0, "age");
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_extract() {
        let func = ScalarFunction::Struct { op: StructOp::Extract };
        let s = Value::Struct(vec![("name".into(), Value::String("Bob".into()))]);
        assert_eq!(
            evaluate_scalar(&func, &[s, Value::String("name".into())]).unwrap(),
            Value::String("Bob".into())
        );
    }

    // --- Cast function tests ---
    #[test]
    fn test_cast_int32() {
        let func = ScalarFunction::Cast {
            target_type: CastTarget::Int32,
        };
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(42)]).unwrap(), Value::Int32(42));
        assert_eq!(evaluate_scalar(&func, &[Value::Double(3.14)]).unwrap(), Value::Int32(3));
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("99".into())]).unwrap(),
            Value::Int32(99)
        );
    }

    #[test]
    fn test_cast_float() {
        let func = ScalarFunction::Cast {
            target_type: CastTarget::Float,
        };
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(42)]).unwrap(), Value::Float(42.0));
        let result = evaluate_scalar(&func, &[Value::String("3.14".into())]).unwrap();
        match result {
            Value::Float(x) => assert!((x - 3.14).abs() < 0.001),
            _ => panic!("Expected float"),
        }
    }

    #[test]
    fn test_cast_bool() {
        let func = ScalarFunction::Cast {
            target_type: CastTarget::Bool,
        };
        assert_eq!(evaluate_scalar(&func, &[Value::Bool(true)]).unwrap(), Value::Bool(true));
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(1)]).unwrap(), Value::Bool(true));
        assert_eq!(evaluate_scalar(&func, &[Value::Int64(0)]).unwrap(), Value::Bool(false));
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("true".into())]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("false".into())]).unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn test_cast_date_timestamp() {
        let cast_date = ScalarFunction::Cast {
            target_type: CastTarget::Date,
        };
        let cast_ts = ScalarFunction::Cast {
            target_type: CastTarget::Timestamp,
        };

        let d = Value::Date(Date(100));
        assert_eq!(
            evaluate_scalar(&cast_date, &[d.clone()]).unwrap(),
            Value::Date(Date(100))
        );

        let ts = evaluate_scalar(&cast_ts, &[d]).unwrap();
        assert!(matches!(ts, Value::Timestamp(_)));
    }

    // --- Aggregate function tests ---
    #[test]
    fn test_aggregate_count() {
        let result = evaluate_aggregate(
            &AggregateFunction::Count,
            &[Value::Int64(1), Value::Int64(2), Value::Int64(3)],
        );
        assert_eq!(result.unwrap(), Value::Int64(3));
    }

    #[test]
    fn test_aggregate_count_star() {
        let result = evaluate_aggregate(&AggregateFunction::CountStar, &[Value::Null, Value::Int64(1)]);
        assert_eq!(result.unwrap(), Value::Int64(2));
    }

    #[test]
    fn test_aggregate_sum() {
        let result = evaluate_aggregate(
            &AggregateFunction::Sum,
            &[Value::Int64(1), Value::Int64(2), Value::Int64(3)],
        );
        assert_eq!(result.unwrap(), Value::Int64(6));
    }

    #[test]
    fn test_aggregate_avg() {
        let result = evaluate_aggregate(
            &AggregateFunction::Avg,
            &[Value::Int64(1), Value::Int64(2), Value::Int64(3)],
        );
        match result.unwrap() {
            Value::Double(x) => assert!((x - 2.0).abs() < 1e-10),
            _ => panic!("Expected double"),
        }
    }

    #[test]
    fn test_aggregate_min_max() {
        let values = &[Value::Int64(5), Value::Int64(2), Value::Int64(8), Value::Int64(1)];
        assert_eq!(
            evaluate_aggregate(&AggregateFunction::Min, values).unwrap(),
            Value::Int64(1)
        );
        assert_eq!(
            evaluate_aggregate(&AggregateFunction::Max, values).unwrap(),
            Value::Int64(8)
        );
    }

    #[test]
    fn test_aggregate_collect() {
        let result = evaluate_aggregate(&AggregateFunction::Collect, &[Value::Int64(1), Value::Int64(2)]);
        match result.unwrap() {
            Value::List(items) => assert_eq!(items.len(), 2),
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn test_aggregate_skip_null() {
        let result = evaluate_aggregate(
            &AggregateFunction::Sum,
            &[Value::Int64(1), Value::Null, Value::Int64(2)],
        );
        assert_eq!(result.unwrap(), Value::Int64(3));
    }

    #[test]
    fn test_aggregate_empty() {
        let result = evaluate_aggregate(&AggregateFunction::Count, &[]);
        assert_eq!(result.unwrap(), Value::Int64(0));
    }

    #[test]
    fn test_aggregate_double_sum() {
        let result = evaluate_aggregate(&AggregateFunction::Sum, &[Value::Double(1.5), Value::Double(2.5)]);
        match result.unwrap() {
            Value::Double(x) => assert!((x - 4.0).abs() < 1e-10),
            _ => panic!("Expected double"),
        }
    }

    // --- AggValueState tests ---
    #[test]
    fn test_agg_value_state_new() {
        let state = AggValueState::new(&AggregateFunction::Count);
        assert!(matches!(state, AggValueState::Count(0)));
        let state = AggValueState::new(&AggregateFunction::Sum);
        assert!(matches!(state, AggValueState::Sum(Value::Null)));
        let state = AggValueState::new(&AggregateFunction::Collect);
        assert!(matches!(state, AggValueState::Collect(_)));
    }

    #[test]
    fn test_agg_state_stddev() {
        let mut state = AggValueState::new(&AggregateFunction::StdDev);
        state.update(&Value::Double(2.0));
        state.update(&Value::Double(4.0));
        state.update(&Value::Double(6.0));
        let result = state.finalize();
        match result {
            Value::Double(x) => assert!((x - 1.63299).abs() < 0.001),
            _ => panic!("Expected double, got {:?}", result),
        }
    }

    #[test]
    fn test_agg_state_variance() {
        let mut state = AggValueState::new(&AggregateFunction::Variance);
        state.update(&Value::Double(2.0));
        state.update(&Value::Double(4.0));
        state.update(&Value::Double(6.0));
        let result = state.finalize();
        match result {
            Value::Double(x) => assert!((x - 2.66666).abs() < 0.001),
            _ => panic!("Expected double"),
        }
    }

    #[test]
    fn test_agg_state_percentile_disc() {
        let mut state = AggValueState::new(&AggregateFunction::PercentileDisc { percentile: 0.5 });
        state.update(&Value::Double(1.0));
        state.update(&Value::Double(3.0));
        state.update(&Value::Double(7.0));
        state.update(&Value::Double(9.0));
        // 4 values, 0.5 * 4 = 2 → ceil(2) = 2 → index 1 → 3.0
        let result = state.finalize();
        match result {
            Value::Double(x) => assert!((x - 3.0).abs() < 0.001, "Expected median 3.0, got {}", x),
            _ => panic!("Expected double"),
        }
    }

    #[test]
    fn test_agg_state_percentile_disc_90th() {
        let mut state = AggValueState::new(&AggregateFunction::PercentileDisc { percentile: 0.9 });
        for v in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0] {
            state.update(&Value::Double(v));
        }
        // 10 values, 0.9 * 10 = 9 → ceil(9) = 9 → index 8 → 9.0
        let result = state.finalize();
        match result {
            Value::Double(x) => assert!((x - 9.0).abs() < 0.001),
            _ => panic!("Expected double"),
        }
    }

    #[test]
    fn test_agg_state_percentile_skip_null() {
        let mut state = AggValueState::new(&AggregateFunction::PercentileDisc { percentile: 0.5 });
        state.update(&Value::Null);
        state.update(&Value::Double(10.0));
        state.update(&Value::Double(20.0));
        // 2 values: 10, 20. 0.5 * 2 = 1 → ceil(1) = 1 → index 0 → 10.0
        let result = state.finalize();
        match result {
            Value::Double(x) => assert!((x - 10.0).abs() < 0.001),
            _ => panic!("Expected double"),
        }
    }

    #[test]
    fn test_agg_state_percentile_empty() {
        let state = AggValueState::new(&AggregateFunction::PercentileDisc { percentile: 0.5 });
        let result = state.finalize();
        assert_eq!(result, Value::Null);
    }

    #[test]
    fn test_agg_state_percentile_cont() {
        let mut state = AggValueState::new(&AggregateFunction::PercentileCont { percentile: 0.5 });
        state.update(&Value::Double(1.0));
        state.update(&Value::Double(5.0));
        // 2 values, 0.5 * 2 = 1 → ceil(1) = 1 → index 0 → 1.0 (same as disc for small N)
        let result = state.finalize();
        match result {
            Value::Double(x) => assert!((x - 1.0).abs() < 0.001),
            _ => panic!("Expected double"),
        }
    }

    // --- AggValueState merge tests ---
    #[test]
    fn test_agg_state_merge_count() {
        let mut a = AggValueState::new(&AggregateFunction::Count);
        let b = AggValueState::Count(5);
        a.update(&Value::Int64(1));
        a.update(&Value::Int64(2));
        a.merge(&b);
        assert_eq!(a.finalize(), Value::Int64(7)); // 2 + 5
    }

    #[test]
    fn test_agg_state_merge_sum() {
        let mut a = AggValueState::new(&AggregateFunction::Sum);
        let mut b = AggValueState::new(&AggregateFunction::Sum);
        a.update(&Value::Int64(10));
        b.update(&Value::Int64(20));
        b.update(&Value::Int64(30));
        a.merge(&b);
        assert_eq!(a.finalize(), Value::Int64(60));
    }

    #[test]
    fn test_agg_state_merge_avg() {
        let mut a = AggValueState::new(&AggregateFunction::Avg);
        let mut b = AggValueState::new(&AggregateFunction::Avg);
        a.update(&Value::Double(10.0));
        b.update(&Value::Double(20.0));
        b.update(&Value::Double(30.0));
        a.merge(&b);
        match a.finalize() {
            Value::Double(x) => assert!((x - 20.0).abs() < 0.001),
            _ => panic!("Expected double"),
        }
    }

    #[test]
    fn test_agg_state_merge_percentile() {
        let mut a = AggValueState::new(&AggregateFunction::PercentileDisc { percentile: 0.5 });
        let mut b = AggValueState::new(&AggregateFunction::PercentileDisc { percentile: 0.5 });
        a.update(&Value::Double(1.0));
        a.update(&Value::Double(3.0));
        b.update(&Value::Double(7.0));
        b.update(&Value::Double(9.0));
        a.merge(&b);
        match a.finalize() {
            Value::Double(x) => assert!((x - 3.0).abs() < 0.001),
            _ => panic!("Expected double"),
        }
    }

    // --- Schema function tests ---
    #[test]
    fn test_schema_offset_internal_id() {
        let func = ScalarFunction::Schema { op: SchemaOp::Offset };
        let id = kuzu_common::types::InternalID { table_id: 1, offset: 42 };
        assert_eq!(
            evaluate_scalar(&func, &[Value::InternalID(id)]).unwrap(),
            Value::Int64(42)
        );
    }

    #[test]
    fn test_schema_offset_struct() {
        let func = ScalarFunction::Schema { op: SchemaOp::Offset };
        let id = kuzu_common::types::InternalID { table_id: 1, offset: 99 };
        let node = Value::Struct(vec![("_id".into(), Value::InternalID(id))]);
        assert_eq!(
            evaluate_scalar(&func, &[node]).unwrap(),
            Value::Int64(99)
        );
    }

    #[test]
    fn test_schema_offset_error() {
        let func = ScalarFunction::Schema { op: SchemaOp::Offset };
        assert!(evaluate_scalar(&func, &[Value::Int64(42)]).is_err());
    }

    #[test]
    fn test_schema_id_internal_id() {
        let func = ScalarFunction::Schema { op: SchemaOp::Id };
        let id = kuzu_common::types::InternalID { table_id: 5, offset: 100 };
        assert_eq!(
            evaluate_scalar(&func, &[Value::InternalID(id)]).unwrap(),
            Value::InternalID(id)
        );
    }

    #[test]
    fn test_schema_id_from_struct() {
        let func = ScalarFunction::Schema { op: SchemaOp::Id };
        let id = kuzu_common::types::InternalID { table_id: 2, offset: 77 };
        let node = Value::Struct(vec![("_id".into(), Value::InternalID(id))]);
        assert_eq!(
            evaluate_scalar(&func, &[node]).unwrap(),
            Value::InternalID(id)
        );
    }

    #[test]
    fn test_schema_start_end_node() {
        let start_func = ScalarFunction::Schema { op: SchemaOp::StartNode };
        let end_func = ScalarFunction::Schema { op: SchemaOp::EndNode };
        let src_id = kuzu_common::types::InternalID { table_id: 1, offset: 10 };
        let dst_id = kuzu_common::types::InternalID { table_id: 1, offset: 20 };
        let rel = Value::Struct(vec![
            ("_src".into(), Value::InternalID(src_id)),
            ("_dst".into(), Value::InternalID(dst_id)),
        ]);
        assert_eq!(
            evaluate_scalar(&start_func, &[rel.clone()]).unwrap(),
            Value::InternalID(src_id)
        );
        assert_eq!(
            evaluate_scalar(&end_func, &[rel]).unwrap(),
            Value::InternalID(dst_id)
        );
    }

    #[test]
    fn test_schema_label_string() {
        let func = ScalarFunction::Schema { op: SchemaOp::Label };
        assert_eq!(
            evaluate_scalar(&func, &[Value::String("Person".into())]).unwrap(),
            Value::String("Person".into())
        );
    }

    #[test]
    fn test_schema_label_struct() {
        let func = ScalarFunction::Schema { op: SchemaOp::Label };
        let node = Value::Struct(vec![("_label".into(), Value::String("Person".into()))]);
        assert_eq!(
            evaluate_scalar(&func, &[node]).unwrap(),
            Value::String("Person".into())
        );
    }

    #[test]
    fn test_schema_label_internal_id() {
        let func = ScalarFunction::Schema { op: SchemaOp::Label };
        let id = kuzu_common::types::InternalID { table_id: 3, offset: 0 };
        let result = evaluate_scalar(&func, &[Value::InternalID(id)]).unwrap();
        assert!(matches!(result, Value::String(_)));
        if let Value::String(s) = result {
            assert!(s.contains("3"));
        }
    }

    #[test]
    fn test_schema_empty_args() {
        let func = ScalarFunction::Schema { op: SchemaOp::Label };
        assert!(evaluate_scalar(&func, &[]).is_err());
    }

    #[test]
    fn test_schema_registry_contains() {
        let reg = FunctionRegistry::new();
        assert!(reg.contains("OFFSET"));
        assert!(reg.contains("ID"));
        assert!(reg.contains("START_NODE"));
        assert!(reg.contains("END_NODE"));
        assert!(reg.contains("LABEL"));
    }

    // --- Array function tests ---
    #[test]
    fn test_array_cosine_similarity() {
        let func = ScalarFunction::Array { op: ArrayOp::CosineSimilarity };
        let a = Value::List(vec![Value::Double(1.0), Value::Double(0.0)]);
        let b = Value::List(vec![Value::Double(0.0), Value::Double(1.0)]);
        let result = evaluate_scalar(&func, &[a, b]).unwrap();
        if let Value::Double(x) = result {
            assert!((x - 0.0).abs() < 1e-10);
        } else {
            panic!("Expected Double");
        }
    }

    #[test]
    fn test_array_cosine_identical() {
        let func = ScalarFunction::Array { op: ArrayOp::CosineSimilarity };
        let a = Value::List(vec![Value::Double(3.0), Value::Double(4.0)]);
        let b = Value::List(vec![Value::Double(3.0), Value::Double(4.0)]);
        let result = evaluate_scalar(&func, &[a, b]).unwrap();
        if let Value::Double(x) = result {
            assert!((x - 1.0).abs() < 1e-10);
        } else {
            panic!("Expected Double");
        }
    }

    #[test]
    fn test_array_distance() {
        let func = ScalarFunction::Array { op: ArrayOp::Distance };
        let a = Value::List(vec![Value::Double(0.0), Value::Double(0.0)]);
        let b = Value::List(vec![Value::Double(3.0), Value::Double(4.0)]);
        let result = evaluate_scalar(&func, &[a, b]).unwrap();
        if let Value::Double(x) = result {
            assert!((x - 5.0).abs() < 1e-10);
        } else {
            panic!("Expected Double");
        }
    }

    #[test]
    fn test_array_inner_product() {
        let func = ScalarFunction::Array { op: ArrayOp::InnerProduct };
        let a = Value::List(vec![Value::Double(1.0), Value::Double(2.0)]);
        let b = Value::List(vec![Value::Double(3.0), Value::Double(4.0)]);
        let result = evaluate_scalar(&func, &[a, b]).unwrap();
        assert_eq!(result, Value::Double(11.0));
    }

    #[test]
    fn test_array_cross_product() {
        let func = ScalarFunction::Array { op: ArrayOp::CrossProduct };
        let a = Value::List(vec![Value::Double(1.0), Value::Double(0.0), Value::Double(0.0)]);
        let b = Value::List(vec![Value::Double(0.0), Value::Double(1.0), Value::Double(0.0)]);
        let result = evaluate_scalar(&func, &[a, b]).unwrap();
        match result {
            Value::List(items) => {
                assert_eq!(items.len(), 3);
                if let Value::Double(z) = &items[2] {
                    assert!((z - 1.0).abs() < 1e-10);
                } else {
                    panic!("Expected Double for z-component");
                }
            }
            _ => panic!("Expected List"),
        }
    }

    #[test]
    fn test_array_cross_product_wrong_dim() {
        let func = ScalarFunction::Array { op: ArrayOp::CrossProduct };
        let a = Value::List(vec![Value::Double(1.0), Value::Double(2.0)]);
        let b = Value::List(vec![Value::Double(3.0), Value::Double(4.0)]);
        assert!(evaluate_scalar(&func, &[a, b]).is_err());
    }

    #[test]
    fn test_array_squared_distance() {
        let func = ScalarFunction::Array { op: ArrayOp::SquaredDistance };
        let a = Value::List(vec![Value::Double(0.0), Value::Double(0.0)]);
        let b = Value::List(vec![Value::Double(3.0), Value::Double(4.0)]);
        let result = evaluate_scalar(&func, &[a, b]).unwrap();
        assert_eq!(result, Value::Double(25.0));
    }

    #[test]
    fn test_array_diff_length() {
        let func = ScalarFunction::Array { op: ArrayOp::Distance };
        let a = Value::List(vec![Value::Double(1.0)]);
        let b = Value::List(vec![Value::Double(1.0), Value::Double(2.0)]);
        assert!(evaluate_scalar(&func, &[a, b]).is_err());
    }

    #[test]
    fn test_list_slice() {
        let func = ScalarFunction::List { op: ListOp::Slice };
        let list = Value::List(vec![
            Value::Int64(10), Value::Int64(20), Value::Int64(30), Value::Int64(40), Value::Int64(50),
        ]);
        let result = evaluate_scalar(&func, &[list, Value::Int64(2), Value::Int64(4)]).unwrap();
        match result {
            Value::List(items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0], Value::Int64(20));
                assert_eq!(items[2], Value::Int64(40));
            }
            _ => panic!("Expected List"),
        }
    }

    #[test]
    fn test_list_slice_single_arg() {
        let func = ScalarFunction::List { op: ListOp::Slice };
        let list = Value::List(vec![Value::Int64(10), Value::Int64(20), Value::Int64(30)]);
        let result = evaluate_scalar(&func, &[list, Value::Int64(2)]).unwrap();
        match result {
            Value::List(items) => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0], Value::Int64(20));
                assert_eq!(items[1], Value::Int64(30));
            }
            _ => panic!("Expected List"),
        }
    }

    #[test]
    fn test_array_registry_contains() {
        let reg = FunctionRegistry::new();
        assert!(reg.contains("array_cosine_similarity"));
        assert!(reg.contains("array_distance"));
        assert!(reg.contains("array_inner_product"));
        assert!(reg.contains("array_cross_product"));
        assert!(reg.contains("array_squared_distance"));
        assert!(reg.contains("list_slice"));
        assert!(reg.contains("list_prepend"));
        // Array utility aliases
        assert!(reg.contains("array_concat"), "array_concat should be registered");
        assert!(reg.contains("array_cat"), "array_cat should be registered");
        assert!(reg.contains("array_append"), "array_append should be registered");
        assert!(reg.contains("array_push_back"), "array_push_back should be registered");
        assert!(reg.contains("array_prepend"), "array_prepend should be registered");
        assert!(reg.contains("array_push_front"), "array_push_front should be registered");
        assert!(reg.contains("array_contains"), "array_contains should be registered");
        assert!(reg.contains("array_has"), "array_has should be registered");
        assert!(reg.contains("array_slice"), "array_slice should be registered");
    }

    // --- List functions (C++ port) tests ---

    #[test]
    fn test_range() {
        let func = ScalarFunction::List { op: ListOp::Range };
        // 1-arg: range(end) → [0, 1, ..., end]
        let result = evaluate_scalar(&func, &[Value::Int64(3)]).unwrap();
        assert_eq!(result, Value::List(vec![
            Value::Int64(0), Value::Int64(1), Value::Int64(2), Value::Int64(3),
        ]));
        // 2-arg: range(start, end)
        let result = evaluate_scalar(&func, &[Value::Int64(2), Value::Int64(5)]).unwrap();
        assert_eq!(result, Value::List(vec![
            Value::Int64(2), Value::Int64(3), Value::Int64(4), Value::Int64(5),
        ]));
        // 3-arg: range(start, end, step)
        let result = evaluate_scalar(&func, &[Value::Int64(0), Value::Int64(6), Value::Int64(2)]).unwrap();
        assert_eq!(result, Value::List(vec![
            Value::Int64(0), Value::Int64(2), Value::Int64(4), Value::Int64(6),
        ]));
        // Zero step → error
        assert!(evaluate_scalar(&func, &[Value::Int64(0), Value::Int64(5), Value::Int64(0)]).is_err());
    }

    #[test]
    fn test_list_distinct() {
        let func = ScalarFunction::List { op: ListOp::Distinct };
        let result = evaluate_scalar(&func, &[Value::List(vec![
            Value::Int64(1), Value::Int64(2), Value::Int64(1), Value::Int64(3),
        ])]).unwrap();
        if let Value::List(items) = result {
            assert_eq!(items.len(), 3);
            assert!(items.contains(&Value::Int64(1)));
            assert!(items.contains(&Value::Int64(2)));
            assert!(items.contains(&Value::Int64(3)));
        } else { panic!("Expected list"); }
    }

    #[test]
    fn test_list_unique() {
        let func = ScalarFunction::List { op: ListOp::Unique };
        // All unique → count = 3
        let result = evaluate_scalar(&func, &[Value::List(vec![
            Value::Int64(1), Value::Int64(2), Value::Int64(3),
        ])]).unwrap();
        assert_eq!(result, Value::Int64(3));
        // Duplicates → count = 2
        let result = evaluate_scalar(&func, &[Value::List(vec![
            Value::Int64(1), Value::Int64(2), Value::Int64(1),
        ])]).unwrap();
        assert_eq!(result, Value::Int64(2));
    }

    #[test]
    fn test_list_sum() {
        let func = ScalarFunction::List { op: ListOp::Sum };
        let result = evaluate_scalar(&func, &[Value::List(vec![
            Value::Int64(1), Value::Int64(2), Value::Int64(3),
        ])]).unwrap();
        assert_eq!(result, Value::Int64(6));
    }

    #[test]
    fn test_list_product() {
        let func = ScalarFunction::List { op: ListOp::Product };
        let result = evaluate_scalar(&func, &[Value::List(vec![
            Value::Int64(2), Value::Int64(3), Value::Int64(4),
        ])]).unwrap();
        assert_eq!(result, Value::Int64(24));
    }

    #[test]
    fn test_list_any_value() {
        let func = ScalarFunction::List { op: ListOp::AnyValue };
        let result = evaluate_scalar(&func, &[Value::List(vec![
            Value::Int64(42), Value::Int64(100),
        ])]).unwrap();
        assert_eq!(result, Value::Int64(42));
    }

    #[test]
    fn test_list_to_string() {
        let func = ScalarFunction::List { op: ListOp::ToString };
        let result = evaluate_scalar(&func, &[
            Value::String(",".into()),
            Value::List(vec![Value::Int64(1), Value::Int64(2), Value::Int64(3)]),
        ]).unwrap();
        assert_eq!(result, Value::String("Int64(1),Int64(2),Int64(3)".into()));
    }

    #[test]
    fn test_list_position() {
        let func = ScalarFunction::List { op: ListOp::Position };
        let list = Value::List(vec![
            Value::String("a".into()), Value::String("b".into()), Value::String("c".into()),
        ]);
        // Found → 1-based index
        let result = evaluate_scalar(&func, &[list.clone(), Value::String("b".into())]).unwrap();
        assert_eq!(result, Value::Int64(2));
        // Not found → 0
        let result = evaluate_scalar(&func, &[list.clone(), Value::String("z".into())]).unwrap();
        assert_eq!(result, Value::Int64(0));
    }

    #[test]
    fn test_list_has_all() {
        let func = ScalarFunction::List { op: ListOp::HasAll };
        let left = Value::List(vec![
            Value::Int64(1), Value::Int64(2), Value::Int64(3),
        ]);
        let right_yes = Value::List(vec![Value::Int64(1), Value::Int64(3)]);
        let right_no = Value::List(vec![Value::Int64(1), Value::Int64(99)]);
        assert_eq!(
            evaluate_scalar(&func, &[left.clone(), right_yes]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_scalar(&func, &[left, right_no]).unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn test_list_reverse_sort() {
        let func = ScalarFunction::List { op: ListOp::ReverseSort };
        let result = evaluate_scalar(&func, &[Value::List(vec![
            Value::Int64(3), Value::Int64(1), Value::Int64(2),
        ])]).unwrap();
        assert_eq!(result, Value::List(vec![
            Value::Int64(3), Value::Int64(2), Value::Int64(1),
        ]));
    }

    // --- List predicate function tests ---

    #[test]
    fn test_list_any() {
        let func = ScalarFunction::List { op: ListOp::Any };
        assert_eq!(
            evaluate_scalar(&func, &[Value::List(vec![
                Value::Bool(false), Value::Bool(true),
            ])]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::List(vec![
                Value::Bool(false), Value::Bool(false),
            ])]).unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn test_list_all() {
        let func = ScalarFunction::List { op: ListOp::All };
        assert_eq!(
            evaluate_scalar(&func, &[Value::List(vec![
                Value::Bool(true), Value::Int64(1),
            ])]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::List(vec![
                Value::Bool(true), Value::Int64(0),
            ])]).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::List(vec![])]).unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn test_list_none() {
        let func = ScalarFunction::List { op: ListOp::None };
        assert_eq!(
            evaluate_scalar(&func, &[Value::List(vec![
                Value::Bool(false), Value::Int64(0),
            ])]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::List(vec![
                Value::Bool(false), Value::Int64(1),
            ])]).unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn test_list_single() {
        let func = ScalarFunction::List { op: ListOp::Single };
        assert_eq!(
            evaluate_scalar(&func, &[Value::List(vec![
                Value::Bool(false), Value::Bool(true), Value::Int64(0),
            ])]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::List(vec![
                Value::Bool(false), Value::Int64(0),
            ])]).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            evaluate_scalar(&func, &[Value::List(vec![
                Value::Bool(true), Value::Int64(1),
            ])]).unwrap(),
            Value::Bool(false)
        );
    }
}
