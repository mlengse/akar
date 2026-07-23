use super::get_string;
use crate::registry::*;
use akar_common::types::{Date, Timestamp, Value};
use time::{Date as TimeDate, Month, OffsetDateTime, Time as TimeTime};

// ==================== Date ====================

/// Helper: convert Date (days since epoch) to time::Date.
pub(crate) fn epoch_days_to_date(days: i32) -> Result<TimeDate, String> {
    TimeDate::from_calendar_date(1970, Month::January, 1)
        .map_err(|e| format!("Date error: {e}"))?
        .checked_add(time::Duration::days(days as i64))
        .ok_or_else(|| "Date overflow".into())
}

/// Helper: convert Timestamp (micros since epoch) to OffsetDateTime.
pub(crate) fn epoch_micros_to_datetime(micros: i64) -> Result<OffsetDateTime, String> {
    let secs = micros.div_euclid(1_000_000);
    let nanos = (micros.rem_euclid(1_000_000) * 1000) as u32;
    OffsetDateTime::from_unix_timestamp(secs)
        .map_err(|e| format!("Timestamp error: {e}"))?
        .replace_nanosecond(nanos)
        .map_err(|e| format!("Timestamp nanos error: {e}"))
}

/// Helper: get a numeric value from args (i64 or f64) for date math.
pub(crate) fn extract_numeric_value(v: &Value) -> Result<i64, String> {
    match v {
        Value::Int64(x) => Ok(*x),
        Value::Int32(x) => Ok(*x as i64),
        _ => Err("Expected numeric value for date operation".into()),
    }
}

pub(crate) fn evaluate_date(op: DateOp, args: &[Value]) -> Result<Value, String> {
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
            )
            .map_err(|e| format!("Date error: {e}"))?;
            let last_day = next_month_first
                .previous_day()
                .ok_or("Could not compute previous day")?;
            let epoch = TimeDate::from_calendar_date(1970, Month::January, 1).unwrap();
            let days = (last_day - epoch).whole_days() as i32;
            Ok(Value::Date(Date(days)))
        }
        DateOp::MakeDate => {
            if args.len() < 3 {
                return Err("make_date requires 3 arguments (year, month, day)".into());
            }
            let year = match &args[0] {
                Value::Int64(x) => *x as i32,
                _ => return Err("make_date year must be integer".into()),
            };
            let month_val = match &args[1] {
                Value::Int64(x) => *x as u8,
                _ => return Err("make_date month must be integer".into()),
            };
            let day = match &args[2] {
                Value::Int64(x) => *x as u8,
                _ => return Err("make_date day must be integer".into()),
            };
            let month_enum = Month::try_from(month_val).map_err(|_| format!("Invalid month: {month_val}"))?;
            let d = TimeDate::from_calendar_date(year, month_enum, day).map_err(|e| format!("Invalid date: {e}"))?;
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
pub(crate) fn extract_date_or_timestamp(v: &Value) -> Result<(TimeDate, TimeTime), String> {
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

pub(crate) fn date_part_value(part: &str, date: &TimeDate, time: &TimeTime) -> Result<Value, String> {
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

pub(crate) fn date_trunc_value(part: &str, date: &TimeDate) -> Result<Value, String> {
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

pub(crate) fn date_diff_value(part: &str, d1: &TimeDate, d2: &TimeDate) -> Result<Value, String> {
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

pub(crate) fn date_add_value(part: &str, count: i64, date: &TimeDate) -> Result<Value, String> {
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

pub(crate) fn days_in_month(year: i32, month: Month) -> u8 {
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
