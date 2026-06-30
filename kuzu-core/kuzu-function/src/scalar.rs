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
use kuzu_common::types::{Date, Timestamp, Value};
use time::{Date as TimeDate, Month, OffsetDateTime, Time as TimeTime};

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
        ScalarFunction::CustomScalar { execute, .. } => (execute)(args),
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
}
