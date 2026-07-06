use crate::registry::*;
use kuzu_common::types::Value;
use super::{rng_next, gamma_func, log_gamma, set_rng_seed};


// ==================== Arithmetic ====================

pub(crate) fn evaluate_arithmetic(op: ArithmeticOp, args: &[Value]) -> Result<Value, String> {
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
            Ok(Value::Int64(if v > 0.0 {
                1
            } else if v < 0.0 {
                -1
            } else {
                0
            }))
        }
        ArithmeticOp::Pi => Ok(Value::Double(std::f64::consts::PI)),
        ArithmeticOp::Rand => Ok(Value::Double(rng_next())),
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

pub(crate) fn numeric_to_f64(v: &Value) -> Result<f64, String> {
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
