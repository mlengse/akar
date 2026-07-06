use crate::registry::*;
use kuzu_common::types::{Date, Interval, Timestamp, Value};


// ==================== Cast ====================

pub(crate) fn evaluate_cast(target: CastTarget, args: &[Value]) -> Result<Value, String> {
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
                Ok(Value::Interval(Interval {
                    months: 0,
                    days: 0,
                    micros: *x,
                }))
            }
            _ => Err("Cannot cast to Interval".into()),
        },
    }
}
