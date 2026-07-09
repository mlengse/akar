//! CSV reader for the COPY FROM command.
//!
//! Parses CSV files and coerces string values to Kuzu `Value` types
//! based on a provided schema (column names + types from the catalog).
//!
//! Supports: delimiter, header detection, quoting, escaping,
//! null handling, and type coercion with detailed error messages.

use kuzu_catalog::CatalogColumn;
use kuzu_common::types::{Date, Interval, LogicalTypeID, Timestamp, Value};
use std::collections::HashMap;

/// Configuration for reading a CSV file.
#[derive(Debug, Clone)]
pub struct CsvReaderConfig {
    /// Field delimiter character (default: `,`).
    pub delimiter: u8,
    /// Whether the first row is a header row (default: true).
    pub has_header: bool,
    /// Quote character (default: `"`).
    pub quote: u8,
    /// Escape character (default: `\\`).
    pub escape: u8,
    /// String representation of NULL values (default: `""` — empty string).
    pub null_str: String,
}

impl Default for CsvReaderConfig {
    fn default() -> Self {
        Self {
            delimiter: b',',
            has_header: true,
            quote: b'"',
            escape: b'\\',
            null_str: String::new(),
        }
    }
}

impl CsvReaderConfig {
    /// Build a config from a `HashMap<String, String>` of COPY options.
    ///
    /// Supported keys: `HEADER`, `DELIM` (or `DELIMITER`), `QUOTE`, `ESCAPE`, `NULL`.
    pub fn from_options(options: &HashMap<String, String>) -> Self {
        let mut config = Self::default();

        if let Some(d) = options.get("HEADER").or_else(|| options.get("header")) {
            config.has_header = d.eq_ignore_ascii_case("true");
        }

        if let Some(d) = options
            .get("DELIM")
            .or_else(|| options.get("delim"))
            .or_else(|| options.get("DELIMITER"))
            && let Some(c) = d.chars().next()
        {
            config.delimiter = c as u8;
        }

        if let Some(q) = options.get("QUOTE").or_else(|| options.get("quote"))
            && let Some(c) = q.chars().next()
        {
            config.quote = c as u8;
        }

        if let Some(e) = options.get("ESCAPE").or_else(|| options.get("escape"))
            && let Some(c) = e.chars().next()
        {
            config.escape = c as u8;
        }

        if let Some(n) = options.get("NULL").or_else(|| options.get("null")) {
            config.null_str = n.clone();
        }

        config
    }
}

/// Error type for CSV reader operations.
#[derive(Debug)]
pub enum CsvReaderError {
    /// I/O error (file not found, permission denied, etc.).
    IoError(std::io::Error),
    /// CSV format error from the parser.
    CsvError(String),
    /// Type coercion failure (e.g. "abc" cannot be parsed as Int64).
    TypeCoercion {
        line: usize,
        column: usize,
        column_name: String,
        value: String,
        expected_type: String,
        message: String,
    },
    /// Row has a different number of columns than the schema expects.
    ColumnCountMismatch {
        line: usize,
        expected: usize,
        actual: usize,
    },
}

impl std::fmt::Display for CsvReaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CsvReaderError::IoError(e) => write!(f, "IO error: {e}"),
            CsvReaderError::CsvError(e) => write!(f, "CSV error: {e}"),
            CsvReaderError::TypeCoercion {
                line,
                column,
                column_name,
                value,
                expected_type,
                message,
            } => write!(
                f,
                "Type coercion error at line {line}, column {column} ('{column_name}'): \
                 cannot coerce '{value}' to {expected_type}: {message}"
            ),
            CsvReaderError::ColumnCountMismatch { line, expected, actual } => write!(
                f,
                "Column count mismatch at line {line}: expected {expected} columns, got {actual}"
            ),
        }
    }
}

impl std::error::Error for CsvReaderError {}

/// Result alias for CSV reader operations.
pub type CsvResult<T> = Result<T, CsvReaderError>;

/// Read a CSV file and coerce string values to Kuzu `Value`s matching the schema.
///
/// # Arguments
///
/// * `path` - Path to the CSV file.
/// * `columns` - Column schema (name + type) from the catalog.
/// * `config` - CSV reader configuration.
///
/// # Returns
///
/// A vector of rows, where each row is a `Vec<Value>` with length equal to
/// `columns.len()`.
///
/// # Errors
///
/// Returns `CsvReaderError` on I/O errors, CSV parse errors, column count
/// mismatches, or type coercion failures.
pub fn read_csv(
    path: &str,
    vfs: &kuzu_common::file_system::VirtualFileSystemRegistry,
    columns: &[CatalogColumn],
    config: &CsvReaderConfig,
) -> CsvResult<Vec<Vec<Value>>> {
    let file = vfs.open_read(path).map_err(CsvReaderError::IoError)?;
    let mut reader = std::io::BufReader::new(file);

    let mut raw_reader = csv::ReaderBuilder::new()
        .delimiter(config.delimiter)
        .has_headers(config.has_header)
        .quote(config.quote)
        .escape(Some(config.escape))
        .flexible(true)
        .from_reader(&mut reader);

    // Get header / column names
    let headers: Vec<String> = if config.has_header {
        raw_reader
            .headers()
            .map_err(|e| CsvReaderError::CsvError(e.to_string()))?
            .iter()
            .map(|h| h.to_string())
            .collect()
    } else {
        columns.iter().map(|c| c.name.clone()).collect()
    };

    // Validate column count
    if headers.len() != columns.len() {
        return Err(CsvReaderError::ColumnCountMismatch {
            line: 1,
            expected: columns.len(),
            actual: headers.len(),
        });
    }

    let mut results = Vec::new();
    let start_line = if config.has_header { 2 } else { 1 };

    for (line_number, result) in (start_line..).zip(raw_reader.records()) {
        let record = result.map_err(|e| CsvReaderError::CsvError(format!("Line {line_number}: {e}")))?;

        if record.len() != columns.len() {
            return Err(CsvReaderError::ColumnCountMismatch {
                line: line_number,
                expected: columns.len(),
                actual: record.len(),
            });
        }

        let mut row = Vec::with_capacity(columns.len());
        for (col_idx, field) in record.iter().enumerate() {
            let col = &columns[col_idx];
            let value = coerce_string_to_value(field, col.logical_type, line_number, col_idx, &col.name)?;
            row.push(value);
        }

        results.push(row);
    }

    Ok(results)
}

// ─── Type coercion ──────────────────────────────────────────────────────────────

/// Coerce a raw CSV string field to a `Value` of the target `LogicalTypeID`.
fn coerce_string_to_value(
    field: &str,
    target_type: LogicalTypeID,
    line: usize,
    column: usize,
    column_name: &str,
) -> CsvResult<Value> {
    let trimmed = field.trim();

    // Handle NULL / empty
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") {
        return Ok(Value::Null);
    }

    match target_type {
        LogicalTypeID::Bool => coerce_bool(trimmed, line, column, column_name),
        LogicalTypeID::Int64 | LogicalTypeID::Serial => {
            coerce_parse(trimmed, line, column, column_name, "INT64").map(Value::Int64)
        }
        LogicalTypeID::Int32 => coerce_parse(trimmed, line, column, column_name, "INT32").map(Value::Int32),
        LogicalTypeID::Int16 => coerce_parse(trimmed, line, column, column_name, "INT16").map(Value::Int16),
        LogicalTypeID::Int8 => coerce_parse(trimmed, line, column, column_name, "INT8").map(Value::Int8),
        LogicalTypeID::UInt64 => coerce_parse(trimmed, line, column, column_name, "UINT64").map(Value::UInt64),
        LogicalTypeID::UInt32 => coerce_parse(trimmed, line, column, column_name, "UINT32").map(Value::UInt32),
        LogicalTypeID::UInt16 => coerce_parse(trimmed, line, column, column_name, "UINT16").map(Value::UInt16),
        LogicalTypeID::UInt8 => coerce_parse(trimmed, line, column, column_name, "UINT8").map(Value::UInt8),
        LogicalTypeID::Double => coerce_parse::<f64>(trimmed, line, column, column_name, "DOUBLE").map(Value::Double),
        LogicalTypeID::Float => coerce_parse::<f32>(trimmed, line, column, column_name, "FLOAT").map(Value::Float),
        LogicalTypeID::String => Ok(Value::String(trimmed.to_string())),
        LogicalTypeID::Date => coerce_date(trimmed, line, column, column_name),
        LogicalTypeID::Timestamp | LogicalTypeID::TimestampMs => coerce_timestamp(trimmed, line, column, column_name),
        LogicalTypeID::TimestampSec => coerce_timestamp_sec(trimmed, line, column, column_name),
        LogicalTypeID::TimestampNs => coerce_timestamp_ns(trimmed, line, column, column_name),
        LogicalTypeID::TimestampTz => coerce_timestamp_tz(trimmed, line, column, column_name),
        LogicalTypeID::Interval => coerce_interval(trimmed, line, column, column_name),
        LogicalTypeID::Blob => Ok(Value::Blob(parse_blob(trimmed))),
        LogicalTypeID::List => Ok(Value::List(parse_list(trimmed))),
        LogicalTypeID::Map => Ok(Value::Map(parse_map(trimmed))),
        LogicalTypeID::Struct | LogicalTypeID::Node | LogicalTypeID::Rel => Ok(Value::Struct(parse_struct(trimmed))),
        // Fallback: keep as string
        _ => Ok(Value::String(trimmed.to_string())),
    }
}

/// Coerce a string to a boolean.
fn coerce_bool(s: &str, line: usize, column: usize, column_name: &str) -> CsvResult<Value> {
    match s.to_lowercase().as_str() {
        "true" | "1" | "yes" | "t" => Ok(Value::Bool(true)),
        "false" | "0" | "no" | "f" => Ok(Value::Bool(false)),
        other => Err(CsvReaderError::TypeCoercion {
            line,
            column,
            column_name: column_name.to_string(),
            value: other.to_string(),
            expected_type: "BOOL".into(),
            message: "expected true/false, 1/0, yes/no, or t/f".into(),
        }),
    }
}

/// Parse a numeric field via `str::parse`, returning a type-coercion error on failure.
fn coerce_parse<T: std::str::FromStr>(
    s: &str,
    line: usize,
    column: usize,
    column_name: &str,
    type_name: &str,
) -> CsvResult<T> {
    s.parse::<T>().map_err(|_| CsvReaderError::TypeCoercion {
        line,
        column,
        column_name: column_name.to_string(),
        value: s.to_string(),
        expected_type: type_name.to_string(),
        message: format!("cannot parse '{s}' as {type_name}"),
    })
}

/// Parse a date string in `YYYY-MM-DD` format.
fn coerce_date(s: &str, line: usize, column: usize, column_name: &str) -> CsvResult<Value> {
    // Accept formats: YYYY-MM-DD or YYYY-M-D
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        return Err(coercion_err(
            s,
            line,
            column,
            column_name,
            "DATE",
            "expected YYYY-MM-DD format",
        ));
    }
    let year: i32 = parts[0]
        .parse()
        .map_err(|_| coercion_err(s, line, column, column_name, "DATE", "invalid year"))?;
    let month: u32 = parts[1]
        .parse()
        .map_err(|_| coercion_err(s, line, column, column_name, "DATE", "invalid month"))?;
    let day: u32 = parts[2]
        .parse()
        .map_err(|_| coercion_err(s, line, column, column_name, "DATE", "invalid day"))?;

    // Simple days-since-epoch calculation (from 1970-01-01)
    let days = naive_date_to_epoch_days(year, month, day)
        .ok_or_else(|| coercion_err(s, line, column, column_name, "DATE", "invalid calendar date"))?;

    Ok(Value::Date(Date::from_days_since_epoch(days)))
}

/// Parse a timestamp string in `YYYY-MM-DD HH:MM:SS[.fraction]` format.
fn coerce_timestamp(s: &str, line: usize, column: usize, column_name: &str) -> CsvResult<Value> {
    let ts = parse_timestamp_micros(s).ok_or_else(|| {
        coercion_err(
            s,
            line,
            column,
            column_name,
            "TIMESTAMP",
            "expected YYYY-MM-DD HH:MM:SS[.ffffff] format",
        )
    })?;
    Ok(Value::Timestamp(Timestamp::from_micros_since_epoch(ts)))
}

/// Parse a timestamp in seconds resolution.
fn coerce_timestamp_sec(s: &str, line: usize, column: usize, column_name: &str) -> CsvResult<Value> {
    let micros = parse_timestamp_micros(s).ok_or_else(|| {
        coercion_err(
            s,
            line,
            column,
            column_name,
            "TIMESTAMP_SEC",
            "expected YYYY-MM-DD HH:MM:SS[.ffffff] format",
        )
    })?;
    Ok(Value::TimestampSec(Timestamp(micros / 1_000_000)))
}

/// Parse a timestamp in nanoseconds resolution.
fn coerce_timestamp_ns(s: &str, line: usize, column: usize, column_name: &str) -> CsvResult<Value> {
    let micros = parse_timestamp_micros(s).ok_or_else(|| {
        coercion_err(
            s,
            line,
            column,
            column_name,
            "TIMESTAMP_NS",
            "expected YYYY-MM-DD HH:MM:SS[.ffffff] format",
        )
    })?;
    // Convert micros to nanos (multiply by 1000)
    Ok(Value::TimestampNs(Timestamp(micros * 1000)))
}

/// Parse a timestamp with timezone.
fn coerce_timestamp_tz(s: &str, line: usize, column: usize, column_name: &str) -> CsvResult<Value> {
    let micros = parse_timestamp_micros(s).ok_or_else(|| {
        coercion_err(
            s,
            line,
            column,
            column_name,
            "TIMESTAMP_TZ",
            "expected YYYY-MM-DD HH:MM:SS[.ffffff] format",
        )
    })?;
    Ok(Value::TimestampTz(kuzu_common::types::TimestampTZ(micros)))
}

/// Parse an interval string like "1 year 2 months 3 days 4 hours 5 minutes 6 seconds".
fn coerce_interval(s: &str, line: usize, column: usize, column_name: &str) -> CsvResult<Value> {
    match parse_interval_str(s) {
        Some(iv) => Ok(Value::Interval(iv)),
        None => Err(coercion_err(
            s,
            line,
            column,
            column_name,
            "INTERVAL",
            "expected duration format (e.g. '1 year 2 months 3 days 4 hours 5 minutes 6 seconds')",
        )),
    }
}

// ─── Helper: coercion error builder ─────────────────────────────────────────────

fn coercion_err(
    value: &str,
    line: usize,
    column: usize,
    column_name: &str,
    expected_type: &str,
    message: &str,
) -> CsvReaderError {
    CsvReaderError::TypeCoercion {
        line,
        column,
        column_name: column_name.to_string(),
        value: value.to_string(),
        expected_type: expected_type.to_string(),
        message: message.to_string(),
    }
}

// ─── Date helper ────────────────────────────────────────────────────────────────

/// Convert a calendar date to days since Unix epoch (1970-01-01).
/// Returns `None` for invalid dates.
fn naive_date_to_epoch_days(year: i32, month: u32, day: u32) -> Option<i32> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    // Days from 1970-01-01 = days from 0000-03-01 to year-month-day - days from 0000-03-01 to 1970-01-01
    // Using the algorithm from C++ chrono / Howard Hinnant's date library
    let (y, m) = if month <= 2 {
        (year as i64 - 1, month as i64 + 12)
    } else {
        (year as i64, month as i64)
    };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m - 3) + 2) / 5 + day as i64;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;

    // Days from 0000-03-01 to 1970-01-01 (the Unix epoch in March-based calendar).
    // Computed as: 4 * 146097 + (369*365 + 369/4 - 369/100 + 307) = 719469
    const EPOCH_OFFSET: i64 = 719469;

    let days = era * 146097 + doe - EPOCH_OFFSET;
    Some(days as i32)
}

// ─── Timestamp helper ───────────────────────────────────────────────────────────

/// Parse `YYYY-MM-DD HH:MM:SS[.fraction]` string to microseconds since epoch.
fn parse_timestamp_micros(s: &str) -> Option<i64> {
    let s = s.trim();
    // Split date and time parts
    let (date_part, time_part) = if let Some(space_idx) = s.find(' ') {
        (&s[..space_idx], &s[space_idx + 1..])
    } else {
        // Date only — treat as start of day
        (s, "00:00:00")
    };

    // Parse date
    let date_parts: Vec<&str> = date_part.split('-').collect();
    if date_parts.len() != 3 {
        return None;
    }
    let year: i32 = date_parts[0].parse().ok()?;
    let month: u32 = date_parts[1].parse().ok()?;
    let day: u32 = date_parts[2].parse().ok()?;
    let epoch_days = naive_date_to_epoch_days(year, month, day)?;

    // Parse time
    let time_parts: Vec<&str> = time_part.split(':').collect();
    if time_parts.len() < 2 || time_parts.len() > 3 {
        return None;
    }
    let hour: u32 = time_parts[0].parse().ok()?;
    let minute: u32 = time_parts[1].parse().ok()?;

    let (second, micros) = if time_parts.len() == 3 {
        let sec_str = time_parts[2];
        if let Some(dot_idx) = sec_str.find('.') {
            let sec: u32 = sec_str[..dot_idx].parse().ok()?;
            let frac_str = &sec_str[dot_idx + 1..];
            // Pad/truncate to 6 digits (microseconds)
            let mut frac = [0u8; 6];
            for (i, ch) in frac_str.chars().enumerate() {
                if i >= 6 {
                    break;
                }
                if ch.is_ascii_digit() {
                    frac[i] = ch as u8 - b'0';
                } else {
                    return None;
                }
            }
            let micros = frac.iter().fold(0u64, |acc, &d| acc * 10 + d as u64);
            (sec, micros)
        } else {
            (sec_str.parse().ok()?, 0)
        }
    } else {
        (0, 0)
    };

    let total_micros = epoch_days as i64 * 86_400_000_000i64
        + hour as i64 * 3_600_000_000i64
        + minute as i64 * 60_000_000i64
        + second as i64 * 1_000_000i64
        + micros as i64;

    Some(total_micros)
}

// ─── Interval helper ────────────────────────────────────────────────────────────

/// Parse a human-readable interval string.
///
/// Supports: `X years`, `X months`, `X days`, `X hours`, `X minutes`,
/// `X seconds`, `X milliseconds`, `X microseconds`, `X us`.
/// Components are space-separated (e.g. "1 year 2 months 3 days").
fn parse_interval_str(s: &str) -> Option<Interval> {
    let mut months: i32 = 0;
    let mut days: i32 = 0;
    let mut micros: i64 = 0;

    let s = s.trim().to_lowercase();
    // Split on whitespace, process pairs
    let tokens: Vec<&str> = s.split_whitespace().collect();
    let mut i = 0;
    while i + 1 < tokens.len() {
        let value: i64 = tokens[i].parse().ok()?;
        let unit = tokens[i + 1];
        match unit {
            u if u.starts_with("year") => months += value as i32 * 12,
            u if u.starts_with("month") => months += value as i32,
            u if u.starts_with("day") => days += value as i32,
            u if u.starts_with("hour") => micros += value * 3_600_000_000,
            u if u.starts_with("minute") => micros += value * 60_000_000,
            u if u.starts_with("second") && !unit.contains("milli") && !unit.contains("micro") => {
                micros += value * 1_000_000;
            }
            u if u.starts_with("millisecond") => micros += value * 1_000,
            u if u.starts_with("microsecond") || u == "us" => micros += value,
            _ => {
                // Unknown unit — skip
            }
        }
        i += 2;
    }

    Some(Interval::new(months, days, micros))
}

// ─── Blob helper ────────────────────────────────────────────────────────────────

/// Parse a blob from hex format.
///
/// Kuzu blob format: `\xHH\xHH...` where HH is a hex byte,
/// or just a plain ASCII string if no hex escapes are present.
fn parse_blob(s: &str) -> Vec<u8> {
    let s = s.trim();
    if s.is_empty() {
        return Vec::new();
    }

    // Check if this is a hex-encoded blob (contains \x)
    let mut result = Vec::new();
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek() == Some(&'x') {
            // Hex escape: \xHH
            chars.next(); // consume 'x'
            let hex_str: String = chars.by_ref().take(2).collect();
            if hex_str.len() == 2 {
                if let Ok(byte) = u8::from_str_radix(&hex_str, 16) {
                    result.push(byte);
                } else {
                    // Invalid hex — push literal
                    result.push(b'\\');
                    result.push(b'x');
                    result.extend_from_slice(hex_str.as_bytes());
                }
            } else {
                result.push(b'\\');
                result.push(b'x');
                result.extend_from_slice(hex_str.as_bytes());
            }
        } else {
            // Plain ASCII character
            let mut buf = [0u8; 4];
            let encoded = ch.encode_utf8(&mut buf);
            result.extend_from_slice(encoded.as_bytes());
        }
    }

    result
}

// ─── List helper ────────────────────────────────────────────────────────────────

/// Parse a list in `[item1, item2, ...]` format.
///
/// Items are coerced to `Value::String` for now (no recursive type inference).
fn parse_list(s: &str) -> Vec<Value> {
    let s = s.trim();
    if s.is_empty() || s == "[]" {
        return Vec::new();
    }

    // Strip surrounding brackets
    let inner = if s.starts_with('[') && s.ends_with(']') {
        &s[1..s.len() - 1]
    } else {
        s
    };

    if inner.trim().is_empty() {
        return Vec::new();
    }

    // Split by comma, respecting quoted strings
    split_csv_respecting_quotes(inner, ',')
        .into_iter()
        .map(|item| {
            let trimmed = item.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") {
                Value::Null
            } else {
                Value::String(trimmed.to_string())
            }
        })
        .collect()
}

// ─── Struct helper ──────────────────────────────────────────────────────────────

/// Parse a struct in `{key1: value1, key2: value2, ...}` format.
///
/// Values are coerced to `Value::String` for now.
fn parse_struct(s: &str) -> Vec<(String, Value)> {
    let s = s.trim();
    if s.is_empty() || s == "{}" {
        return Vec::new();
    }

    // Strip surrounding braces
    let inner = if s.starts_with('{') && s.ends_with('}') {
        &s[1..s.len() - 1]
    } else {
        s
    };

    if inner.trim().is_empty() {
        return Vec::new();
    }

    // Split top-level fields by comma, respecting nested braces/quotes
    split_top_level(inner, ',')
        .into_iter()
        .filter_map(|field| {
            let trimmed = field.trim();
            if trimmed.is_empty() {
                return None;
            }
            // Split on first ':'
            if let Some(colon_idx) = trimmed.find(':') {
                let key = trimmed[..colon_idx].trim().to_string();
                let val_str = trimmed[colon_idx + 1..].trim();
                let value = if val_str.is_empty() || val_str.eq_ignore_ascii_case("null") {
                    Value::Null
                } else {
                    Value::String(val_str.to_string())
                };
                Some((key, value))
            } else {
                Some((trimmed.to_string(), Value::Null))
            }
        })
        .collect()
}

// ─── Map helper ─────────────────────────────────────────────────────────────────

/// Parse a map in `{key1=value1, key2=value2, ...}` format.
///
/// Kuzu uses `=` as key-value separator for maps (vs `:` for structs).
fn parse_map(s: &str) -> Vec<(Value, Value)> {
    let s = s.trim();
    if s.is_empty() || s == "{}" {
        return Vec::new();
    }

    // Strip surrounding braces
    let inner = if s.starts_with('{') && s.ends_with('}') {
        &s[1..s.len() - 1]
    } else {
        s
    };

    if inner.trim().is_empty() {
        return Vec::new();
    }

    split_top_level(inner, ',')
        .into_iter()
        .filter_map(|field| {
            let trimmed = field.trim();
            if trimmed.is_empty() {
                return None;
            }
            // Split on first '='
            if let Some(eq_idx) = trimmed.find('=') {
                let key_str = trimmed[..eq_idx].trim();
                let val_str = trimmed[eq_idx + 1..].trim();
                let key = if key_str.is_empty() || key_str.eq_ignore_ascii_case("null") {
                    Value::Null
                } else {
                    Value::String(key_str.to_string())
                };
                let value = if val_str.is_empty() || val_str.eq_ignore_ascii_case("null") {
                    Value::Null
                } else {
                    Value::String(val_str.to_string())
                };
                Some((key, value))
            } else {
                Some((Value::String(trimmed.to_string()), Value::Null))
            }
        })
        .collect()
}

// ─── Splitting helpers ──────────────────────────────────────────────────────────

/// Split a CSV line by `delimiter`, respecting double-quoted strings.
fn split_csv_respecting_quotes(s: &str, delimiter: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in s.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            c if c == delimiter && !in_quotes => {
                parts.push(current.trim().to_string());
                current = String::new();
            }
            c => current.push(c),
        }
    }
    parts.push(current.trim().to_string());
    parts
}

/// Split top-level fields by `delimiter`, respecting balanced braces, brackets,
/// parentheses, and quotes.
fn split_top_level(s: &str, delimiter: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth_brace = 0i32;
    let mut depth_bracket = 0i32;
    let mut depth_paren = 0i32;
    let mut in_quotes = false;

    for ch in s.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            '{' if !in_quotes => {
                depth_brace += 1;
                current.push(ch);
            }
            '}' if !in_quotes => {
                depth_brace -= 1;
                current.push(ch);
            }
            '[' if !in_quotes => {
                depth_bracket += 1;
                current.push(ch);
            }
            ']' if !in_quotes => {
                depth_bracket -= 1;
                current.push(ch);
            }
            '(' if !in_quotes => {
                depth_paren += 1;
                current.push(ch);
            }
            ')' if !in_quotes => {
                depth_paren -= 1;
                current.push(ch);
            }
            c if c == delimiter && !in_quotes && depth_brace == 0 && depth_bracket == 0 && depth_paren == 0 => {
                parts.push(current.trim().to_string());
                current = String::new();
            }
            c => current.push(c),
        }
    }
    parts.push(current.trim().to_string());
    parts
}

// ─── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_schema() -> Vec<CatalogColumn> {
        vec![
            CatalogColumn { compression: kuzu_common::enums::CompressionType::Uncompressed,
                name: "name".into(),
                logical_type: LogicalTypeID::String,
                is_primary_key: true,
                default_value: None,
            },
            CatalogColumn { compression: kuzu_common::enums::CompressionType::Uncompressed,
                name: "age".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: false,
                default_value: None,
            },
            CatalogColumn { compression: kuzu_common::enums::CompressionType::Uncompressed,
                name: "score".into(),
                logical_type: LogicalTypeID::Double,
                is_primary_key: false,
                default_value: None,
            },
            CatalogColumn { compression: kuzu_common::enums::CompressionType::Uncompressed,
                name: "active".into(),
                logical_type: LogicalTypeID::Bool,
                is_primary_key: false,
                default_value: None,
            },
        ]
    }

    #[test]
    fn test_read_csv_basic() {
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("test.csv");
        std::fs::write(
            &csv_path,
            "name,age,score,active\nAlice,30,95.5,true\nBob,25,87.3,false\n",
        )
        .unwrap();

        let config = CsvReaderConfig::default();
        let rows = read_csv(
            csv_path.to_str().unwrap(),
            &kuzu_common::file_system::VirtualFileSystemRegistry::new(),
            &test_schema(),
            &config,
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0], Value::String("Alice".into()));
        assert_eq!(rows[0][1], Value::Int64(30));
        assert_eq!(rows[0][2], Value::Double(95.5));
        assert_eq!(rows[0][3], Value::Bool(true));
    }

    #[test]
    fn test_read_csv_no_header() {
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("noheader.csv");
        std::fs::write(&csv_path, "Charlie,40,91.2,true\nDiana,22,88.1,false\n").unwrap();

        let config = CsvReaderConfig {
            has_header: false,
            ..Default::default()
        };
        let rows = read_csv(
            csv_path.to_str().unwrap(),
            &kuzu_common::file_system::VirtualFileSystemRegistry::new(),
            &test_schema(),
            &config,
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1][0], Value::String("Diana".into()));
        assert_eq!(rows[1][1], Value::Int64(22));
    }

    #[test]
    fn test_read_csv_custom_delimiter() {
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("pipes.csv");
        std::fs::write(&csv_path, "name|age|score|active\nEve|35|77.5|true\n").unwrap();

        let config = CsvReaderConfig {
            delimiter: b'|',
            ..Default::default()
        };
        let rows = read_csv(
            csv_path.to_str().unwrap(),
            &kuzu_common::file_system::VirtualFileSystemRegistry::new(),
            &test_schema(),
            &config,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0], Value::String("Eve".into()));
        assert_eq!(rows[0][1], Value::Int64(35));
    }

    #[test]
    fn test_read_csv_quoted_fields() {
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("quoted.csv");
        std::fs::write(&csv_path, "name,age,score,active\n\"Frank, Jr.\",28,99.9,true\n").unwrap();

        let config = CsvReaderConfig::default();
        let rows = read_csv(
            csv_path.to_str().unwrap(),
            &kuzu_common::file_system::VirtualFileSystemRegistry::new(),
            &test_schema(),
            &config,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0], Value::String("Frank, Jr.".into()));
    }

    #[test]
    fn test_read_csv_null_values() {
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("nulls.csv");
        std::fs::write(&csv_path, "name,age,score,active\nGrace,,,\n").unwrap();

        let config = CsvReaderConfig::default();
        let rows = read_csv(
            csv_path.to_str().unwrap(),
            &kuzu_common::file_system::VirtualFileSystemRegistry::new(),
            &test_schema(),
            &config,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0], Value::String("Grace".into()));
        assert_eq!(rows[0][1], Value::Null);
        assert_eq!(rows[0][2], Value::Null);
        assert_eq!(rows[0][3], Value::Null);
    }

    #[test]
    fn test_read_csv_column_count_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("bad_cols.csv");
        std::fs::write(&csv_path, "name,age,score,active\nAlice,30\n").unwrap();

        let config = CsvReaderConfig::default();
        let result = read_csv(
            csv_path.to_str().unwrap(),
            &kuzu_common::file_system::VirtualFileSystemRegistry::new(),
            &test_schema(),
            &config,
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            CsvReaderError::ColumnCountMismatch { line, expected, actual } => {
                assert_eq!(line, 2);
                assert_eq!(expected, 4);
                assert_eq!(actual, 2);
            }
            _ => panic!("Expected ColumnCountMismatch"),
        }
    }

    #[test]
    fn test_read_csv_type_coercion_error() {
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("bad_type.csv");
        std::fs::write(&csv_path, "name,age,score,active\nAlice,not_a_number,95.5,true\n").unwrap();

        let config = CsvReaderConfig::default();
        let result = read_csv(
            csv_path.to_str().unwrap(),
            &kuzu_common::file_system::VirtualFileSystemRegistry::new(),
            &test_schema(),
            &config,
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            CsvReaderError::TypeCoercion { line, column, .. } => {
                assert_eq!(line, 2);
                assert_eq!(column, 1);
            }
            _ => panic!("Expected TypeCoercion"),
        }
    }

    #[test]
    fn test_read_csv_file_not_found() {
        let config = CsvReaderConfig::default();
        let result = read_csv(
            "nonexistent.csv",
            &kuzu_common::file_system::VirtualFileSystemRegistry::new(),
            &test_schema(),
            &config,
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            CsvReaderError::IoError(_) => {} // expected
            _ => panic!("Expected IoError"),
        }
    }

    #[test]
    fn test_read_csv_dates_and_timestamps() {
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("dates.csv");
        std::fs::write(
            &csv_path,
            "name,birth,updated\nAlice,1990-05-15,2024-01-20 14:30:00.123456\n",
        )
        .unwrap();

        let schema = vec![
            CatalogColumn { compression: kuzu_common::enums::CompressionType::Uncompressed,
                name: "name".into(),
                logical_type: LogicalTypeID::String,
                is_primary_key: false,
                default_value: None,
            },
            CatalogColumn { compression: kuzu_common::enums::CompressionType::Uncompressed,
                name: "birth".into(),
                logical_type: LogicalTypeID::Date,
                is_primary_key: false,
                default_value: None,
            },
            CatalogColumn { compression: kuzu_common::enums::CompressionType::Uncompressed,
                name: "updated".into(),
                logical_type: LogicalTypeID::Timestamp,
                is_primary_key: false,
                default_value: None,
            },
        ];

        let config = CsvReaderConfig::default();
        let rows = read_csv(
            csv_path.to_str().unwrap(),
            &kuzu_common::file_system::VirtualFileSystemRegistry::new(),
            &schema,
            &config,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0], Value::String("Alice".into()));
        // Date: 1990-05-15 → compute days since epoch
        if let Value::Date(d) = &rows[0][1] {
            assert_eq!(d.days_since_epoch(), 7439); // 1990-05-15
        } else {
            panic!("Expected Date");
        }
        if let Value::Timestamp(ts) = &rows[0][2] {
            // 2024-01-20 14:30:00.123456 → compute micros
            assert!(ts.micros_since_epoch() > 0);
        } else {
            panic!("Expected Timestamp");
        }
    }

    #[test]
    fn test_read_csv_interval() {
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("intervals.csv");
        std::fs::write(
            &csv_path,
            "name,duration\nTask1,1 year 2 months 3 days 4 hours 5 minutes 6 seconds\n",
        )
        .unwrap();

        let schema = vec![
            CatalogColumn { compression: kuzu_common::enums::CompressionType::Uncompressed,
                name: "name".into(),
                logical_type: LogicalTypeID::String,
                is_primary_key: false,
                default_value: None,
            },
            CatalogColumn { compression: kuzu_common::enums::CompressionType::Uncompressed,
                name: "duration".into(),
                logical_type: LogicalTypeID::Interval,
                is_primary_key: false,
                default_value: None,
            },
        ];

        let config = CsvReaderConfig::default();
        let rows = read_csv(
            csv_path.to_str().unwrap(),
            &kuzu_common::file_system::VirtualFileSystemRegistry::new(),
            &schema,
            &config,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        if let Value::Interval(iv) = &rows[0][1] {
            // 1 year = 12 months, + 2 months = 14 months
            assert_eq!(iv.months, 14);
            assert_eq!(iv.days, 3);
            // 4h 5m 6s = 4*3600 + 5*60 + 6 = 14706 seconds = 14706000000 micros
            assert_eq!(iv.micros, 14_706_000_000);
        } else {
            panic!("Expected Interval");
        }
    }

    #[test]
    fn test_read_csv_list_and_struct() {
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("complex.csv");
        std::fs::write(
            &csv_path,
            "name,tags,metadata\nItem1,\"[a,b,c]\",\"{key1: val1, key2: val2}\"\n",
        )
        .unwrap();

        let schema = vec![
            CatalogColumn { compression: kuzu_common::enums::CompressionType::Uncompressed,
                name: "name".into(),
                logical_type: LogicalTypeID::String,
                is_primary_key: false,
                default_value: None,
            },
            CatalogColumn { compression: kuzu_common::enums::CompressionType::Uncompressed,
                name: "tags".into(),
                logical_type: LogicalTypeID::List,
                is_primary_key: false,
                default_value: None,
            },
            CatalogColumn { compression: kuzu_common::enums::CompressionType::Uncompressed,
                name: "metadata".into(),
                logical_type: LogicalTypeID::Struct,
                is_primary_key: false,
                default_value: None,
            },
        ];

        let config = CsvReaderConfig::default();
        let rows = read_csv(
            csv_path.to_str().unwrap(),
            &kuzu_common::file_system::VirtualFileSystemRegistry::new(),
            &schema,
            &config,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0], Value::String("Item1".into()));
        if let Value::List(items) = &rows[0][1] {
            assert_eq!(items.len(), 3);
        } else {
            panic!("Expected List");
        }
        if let Value::Struct(fields) = &rows[0][2] {
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].0, "key1");
        } else {
            panic!("Expected Struct");
        }
    }

    #[test]
    fn test_read_csv_blob() {
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("blobs.csv");
        std::fs::write(&csv_path, "name,data\nBlob1,\\xAA\\xBB\\xCC\\xDD\n").unwrap();

        let schema = vec![
            CatalogColumn { compression: kuzu_common::enums::CompressionType::Uncompressed,
                name: "name".into(),
                logical_type: LogicalTypeID::String,
                is_primary_key: false,
                default_value: None,
            },
            CatalogColumn { compression: kuzu_common::enums::CompressionType::Uncompressed,
                name: "data".into(),
                logical_type: LogicalTypeID::Blob,
                is_primary_key: false,
                default_value: None,
            },
        ];

        let config = CsvReaderConfig::default();
        let rows = read_csv(
            csv_path.to_str().unwrap(),
            &kuzu_common::file_system::VirtualFileSystemRegistry::new(),
            &schema,
            &config,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        if let Value::Blob(bytes) = &rows[0][1] {
            assert_eq!(bytes, &[0xAA, 0xBB, 0xCC, 0xDD]);
        } else {
            panic!("Expected Blob");
        }
    }

    #[test]
    fn test_read_csv_uint_types() {
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("uints.csv");
        std::fs::write(&csv_path, "small,medium,large\n100,1000,100000\n").unwrap();

        let schema = vec![
            CatalogColumn { compression: kuzu_common::enums::CompressionType::Uncompressed,
                name: "small".into(),
                logical_type: LogicalTypeID::UInt8,
                is_primary_key: false,
                default_value: None,
            },
            CatalogColumn { compression: kuzu_common::enums::CompressionType::Uncompressed,
                name: "medium".into(),
                logical_type: LogicalTypeID::UInt32,
                is_primary_key: false,
                default_value: None,
            },
            CatalogColumn { compression: kuzu_common::enums::CompressionType::Uncompressed,
                name: "large".into(),
                logical_type: LogicalTypeID::UInt64,
                is_primary_key: false,
                default_value: None,
            },
        ];

        let config = CsvReaderConfig::default();
        let rows = read_csv(
            csv_path.to_str().unwrap(),
            &kuzu_common::file_system::VirtualFileSystemRegistry::new(),
            &schema,
            &config,
        )
        .unwrap();
        assert_eq!(rows[0][0], Value::UInt8(100));
        assert_eq!(rows[0][1], Value::UInt32(1000));
        assert_eq!(rows[0][2], Value::UInt64(100000));
    }

    #[test]
    fn test_config_from_options() {
        let mut opts = HashMap::new();
        opts.insert("HEADER".into(), "false".into());
        opts.insert("DELIM".into(), "|".into());
        opts.insert("QUOTE".into(), "'".into());
        opts.insert("ESCAPE".into(), "`".into());

        let config = CsvReaderConfig::from_options(&opts);
        assert!(!config.has_header);
        assert_eq!(config.delimiter, b'|');
        assert_eq!(config.quote, b'\'');
        assert_eq!(config.escape, b'`');
    }

    #[test]
    fn test_parse_blob_mixed() {
        let blob = parse_blob("Hello\\x20World");
        assert_eq!(blob, b"Hello World");
    }

    #[test]
    fn test_split_top_level_nested() {
        let result = split_top_level("{a: 1, b: {c: 2}}, {d: 3}", ',');
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "{a: 1, b: {c: 2}}");
        assert_eq!(result[1], "{d: 3}");
    }

    #[test]
    fn test_naive_date_to_epoch_days_known() {
        // 1970-01-01 = 0
        assert_eq!(naive_date_to_epoch_days(1970, 1, 1), Some(0));
        // 2024-01-01 = 19723 days after epoch
        assert_eq!(naive_date_to_epoch_days(2024, 1, 1), Some(19723));
        // 1990-05-15
        assert_eq!(naive_date_to_epoch_days(1990, 5, 15), Some(7439));
    }
}
