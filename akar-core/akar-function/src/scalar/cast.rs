use crate::registry::*;
use akar_common::types::{Date, Interval, Timestamp, Value};
use time::{Date as TimeDate, Month, Time as TimeTime};

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
            Value::String(s) => parse_date_string(s).map(Value::Date),
            _ => Err("Cannot cast to Date".into()),
        },
        CastTarget::Timestamp => match v {
            Value::Timestamp(x) => Ok(Value::Timestamp(*x)),
            Value::Date(d) => Ok(Value::Timestamp(Timestamp(d.0 as i64 * 86400 * 1_000_000))),
            Value::String(s) => parse_timestamp_string(s).map(Value::Timestamp),
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

/// Parse `YYYY-MM-DD` (or `YYYY-M-D`) into days since epoch.
fn parse_date_string(s: &str) -> Result<Date, String> {
    let s = s.trim();
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        return Err(format!("Cannot parse date '{}': expected YYYY-MM-DD", s));
    }
    let year: i32 = parts[0].parse().map_err(|_| format!("Invalid year in '{}'", s))?;
    let month: u32 = parts[1].parse().map_err(|_| format!("Invalid month in '{}'", s))?;
    let day: u8 = parts[2].parse().map_err(|_| format!("Invalid day in '{}'", s))?;
    let month_enum = Month::try_from(month as u8).map_err(|_| format!("Invalid month in '{}'", s))?;
    let date = TimeDate::from_calendar_date(year, month_enum, day).map_err(|e| format!("Invalid date '{}': {e}", s))?;
    let epoch = TimeDate::from_calendar_date(1970, Month::January, 1).map_err(|e| format!("Date error: {e}"))?;
    let days = (date - epoch).whole_days() as i32;
    Ok(Date(days))
}

/// Parse `YYYY-MM-DD[ HH:MM:SS[.fraction]]` into microseconds since epoch.
fn parse_timestamp_string(s: &str) -> Result<Timestamp, String> {
    let s = s.trim();
    let (date_part, time_part) = match s.find(' ') {
        Some(idx) => (&s[..idx], &s[idx + 1..]),
        None => (s, "00:00:00"),
    };
    let date = parse_date_string(date_part)?;
    let mut time_parts: Vec<&str> = time_part.split(':').collect();
    if time_parts.len() != 3 {
        return Err(format!("Cannot parse time '{}': expected HH:MM:SS", time_part));
    }
    let sec_str = time_parts.pop().unwrap_or("0");
    let (sec_str, frac_micros) = match sec_str.find('.') {
        Some(idx) => {
            let frac = &sec_str[idx + 1..];
            let micros = if frac.is_empty() {
                0
            } else {
                let padded = format!("{:<6}", frac);
                padded.chars().take(6).collect::<String>().parse::<i64>().unwrap_or(0)
            };
            (&sec_str[..idx], micros)
        }
        None => (sec_str, 0),
    };
    let hour: u8 = time_parts[0]
        .parse()
        .map_err(|_| format!("Invalid hour in '{}'", time_part))?;
    let minute: u8 = time_parts[1]
        .parse()
        .map_err(|_| format!("Invalid minute in '{}'", time_part))?;
    let second: u8 = sec_str
        .parse()
        .map_err(|_| format!("Invalid second in '{}'", time_part))?;
    let time = TimeTime::from_hms(hour, minute, second).map_err(|e| format!("Invalid time '{}': {e}", time_part))?;
    let seconds = time.as_hms().0 as i64 * 3600 + time.as_hms().1 as i64 * 60 + time.as_hms().2 as i64;
    let micros = (date.0 as i64 * 86400) + seconds * 1_000_000 + frac_micros;
    Ok(Timestamp(micros))
}
