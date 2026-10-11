//! Core type system: LogicalType, PhysicalType, Value, InternalID, date/time types.

use serde::{Deserialize, Serialize};

/// Logical type identifiers for Akar's type system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum LogicalTypeID {
    Any = 0,
    Node = 10,
    Rel = 11,
    RecursiveRel = 12,
    Serial = 13,
    Bool = 22,
    Int64 = 23,
    Int32 = 24,
    Int16 = 25,
    Int8 = 26,
    UInt64 = 27,
    UInt32 = 28,
    UInt16 = 29,
    UInt8 = 30,
    Int128 = 31,
    Double = 32,
    Float = 33,
    Date = 34,
    Timestamp = 35,
    TimestampSec = 36,
    TimestampMs = 37,
    TimestampNs = 38,
    TimestampTz = 39,
    Interval = 40,
    Decimal = 41,
    InternalID = 42,
    UInt128 = 43,
    Json = 44,
    Time = 45,
    String = 50,
    Blob = 51,
    List = 52,
    Array = 53,
    Struct = 54,
    Map = 55,
    Union = 56,
    Uuid = 59,
}

/// Physical type identifiers for in-memory representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum PhysicalTypeID {
    Any = 0,
    Bool = 1,
    Int64 = 2,
    Int32 = 3,
    Int16 = 4,
    Int8 = 5,
    UInt64 = 6,
    UInt32 = 7,
    UInt16 = 8,
    UInt8 = 9,
    Int128 = 10,
    Double = 11,
    Float = 12,
    Interval = 13,
    String = 14,
    Struct = 15,
    List = 16,
    Array = 17,
    Blob = 20,
}

/// A 4-byte aligned, 8-byte internal node/rel identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InternalID {
    pub table_id: u64,
    pub offset: u64,
}

/// Date representation (days since epoch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Date(pub i32);

/// Timestamp representation (microseconds since epoch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Timestamp(pub i64);

/// Timestamp with timezone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimestampTZ(pub i64);

/// Interval (duration).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interval {
    pub months: i32,
    pub days: i32,
    pub micros: i64,
}

/// A Akar value — the runtime representation of any data type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Null,
    Bool(bool),
    Int64(i64),
    Int32(i32),
    Int16(i16),
    Int8(i8),
    UInt64(u64),
    UInt32(u32),
    UInt16(u16),
    UInt8(u8),
    Int128(i128),
    Double(f64),
    Float(f32),
    String(String),
    Blob(Vec<u8>),
    Date(Date),
    Timestamp(Timestamp),
    TimestampTz(TimestampTZ),
    TimestampNs(Timestamp),
    TimestampMs(Timestamp),
    TimestampSec(Timestamp),
    Interval(Interval),
    InternalID(InternalID),
    UInt128(u128),
    Json(serde_json::Value),
    DTime(i64),
    Union(String, Box<Value>),
    List(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Struct(Vec<(String, Value)>),
}

// --- Value rendering -------------------------------------------------------
//
// Single home for display formatting. The CLI (`fmt_val`), `QueryResult`'s
// `Display` impl, and the regression tests all route through `format_value`,
// so a value variant cannot render one way in one surface and another way in
// the other. A variant added to `Value` must be added here too — the
// exhaustive match has no catch-all arm for exactly that reason.

/// Render a [`Value`] as a human-readable string.
///
/// Total function: every variant renders and the function never panics.
/// Composites (`List`/`Map`/`Struct`/`Union`) render recursively.
pub fn format_value(v: &Value) -> String {
    match v {
        Value::Null => "NULL".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int64(i) => i.to_string(),
        Value::Int32(i) => i.to_string(),
        Value::Int16(i) => i.to_string(),
        Value::Int8(i) => i.to_string(),
        Value::UInt64(i) => i.to_string(),
        Value::UInt32(i) => i.to_string(),
        Value::UInt16(i) => i.to_string(),
        Value::UInt8(i) => i.to_string(),
        Value::Int128(i) => i.to_string(),
        Value::UInt128(i) => i.to_string(),
        Value::Double(d) => d.to_string(),
        Value::Float(f) => f.to_string(),
        Value::DTime(t) => time_of_day_to_iso(*t),
        Value::String(s) => s.clone(),
        Value::Blob(b) => {
            let mut hex = String::with_capacity(b.len() * 2);
            for byte in b {
                hex.push_str(&format!("{byte:02x}"));
            }
            format!("x'{hex}'")
        }
        Value::Date(d) => days_to_iso(d.0 as i64),
        Value::Timestamp(t) => micros_to_iso(t.0),
        Value::TimestampTz(t) => format!("{}+00:00", micros_to_iso(t.0)),
        Value::TimestampNs(t) => micros_to_iso(t.0),
        Value::TimestampMs(t) => micros_to_iso(t.0),
        Value::TimestampSec(t) => micros_to_iso(t.0),
        Value::Interval(iv) => format!("{} months {} days {} micros", iv.months, iv.days, iv.micros),
        Value::InternalID(id) => format!("{}:{}", id.table_id, id.offset),
        Value::Json(j) => serde_json::to_string(j).unwrap_or_else(|_| "<json>".to_string()),
        Value::Union(tag, inner) => format!("{}({})", tag, format_value(inner)),
        Value::List(items) => {
            let rendered: Vec<String> = items.iter().map(format_value).collect();
            format!("[{}]", rendered.join(", "))
        }
        Value::Map(pairs) => {
            let rendered: Vec<String> = pairs
                .iter()
                .map(|(k, v)| format!("{}: {}", format_value(k), format_value(v)))
                .collect();
            format!("{{{}}}", rendered.join(", "))
        }
        Value::Struct(fields) => {
            let rendered: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("{k}: {}", format_value(v)))
                .collect();
            format!("{{{}}}", rendered.join(", "))
        }
    }
}

/// Days since the Unix epoch → ISO 8601 date (`YYYY-MM-DD`).
///
/// Proleptic Gregorian, Howard Hinnant's `civil_from_days` algorithm.
fn days_to_iso(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Microseconds since midnight → `HH:MM:SS.ffffff`.
fn time_of_day_to_iso(micros: i64) -> String {
    let micros = micros.rem_euclid(86_400_000_000);
    let (seconds, us) = (micros / 1_000_000, micros % 1_000_000);
    let (minutes, s) = (seconds / 60, seconds % 60);
    let (hours, min) = (minutes / 60, minutes % 60);
    if us == 0 {
        format!("{hours:02}:{min:02}:{s:02}")
    } else {
        format!("{hours:02}:{min:02}:{s:02}.{us:06}")
    }
}

/// Microseconds since the Unix epoch → ISO 8601 UTC datetime
/// (`YYYY-MM-DDTHH:MM:SS[.ffffff]`).
fn micros_to_iso(micros: i64) -> String {
    let days = micros.div_euclid(86_400_000_000);
    let tod = micros.rem_euclid(86_400_000_000);
    format!("{}T{}", days_to_iso(days), time_of_day_to_iso(tod))
}

/// Civil (proleptic Gregorian) date from days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// --- From implementations for Value ---

impl From<bool> for Value {
    #[inline(always)]
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}
impl From<i64> for Value {
    #[inline(always)]
    fn from(v: i64) -> Self {
        Value::Int64(v)
    }
}
impl From<i32> for Value {
    #[inline(always)]
    fn from(v: i32) -> Self {
        Value::Int32(v)
    }
}
impl From<i16> for Value {
    #[inline(always)]
    fn from(v: i16) -> Self {
        Value::Int16(v)
    }
}
impl From<i8> for Value {
    #[inline(always)]
    fn from(v: i8) -> Self {
        Value::Int8(v)
    }
}
impl From<u64> for Value {
    #[inline(always)]
    fn from(v: u64) -> Self {
        Value::UInt64(v)
    }
}
impl From<u32> for Value {
    #[inline(always)]
    fn from(v: u32) -> Self {
        Value::UInt32(v)
    }
}
impl From<u16> for Value {
    #[inline(always)]
    fn from(v: u16) -> Self {
        Value::UInt16(v)
    }
}
impl From<u8> for Value {
    #[inline(always)]
    fn from(v: u8) -> Self {
        Value::UInt8(v)
    }
}
impl From<f64> for Value {
    #[inline(always)]
    fn from(v: f64) -> Self {
        Value::Double(v)
    }
}
impl From<f32> for Value {
    #[inline(always)]
    fn from(v: f32) -> Self {
        Value::Float(v)
    }
}
impl From<String> for Value {
    #[inline(always)]
    fn from(v: String) -> Self {
        Value::String(v)
    }
}
impl From<&str> for Value {
    #[inline(always)]
    fn from(v: &str) -> Self {
        Value::String(v.to_string())
    }
}
impl From<Date> for Value {
    #[inline(always)]
    fn from(v: Date) -> Self {
        Value::Date(v)
    }
}
impl From<Timestamp> for Value {
    #[inline(always)]
    fn from(v: Timestamp) -> Self {
        Value::Timestamp(v)
    }
}
impl From<Interval> for Value {
    #[inline(always)]
    fn from(v: Interval) -> Self {
        Value::Interval(v)
    }
}
impl From<InternalID> for Value {
    #[inline(always)]
    fn from(v: InternalID) -> Self {
        Value::InternalID(v)
    }
}
impl From<u128> for Value {
    #[inline(always)]
    fn from(v: u128) -> Self {
        Value::UInt128(v)
    }
}
impl From<serde_json::Value> for Value {
    #[inline(always)]
    fn from(v: serde_json::Value) -> Self {
        Value::Json(v)
    }
}

impl Value {
    /// Get the LogicalTypeID corresponding to this Value.
    pub fn logical_type(&self) -> LogicalTypeID {
        match self {
            Value::Null => LogicalTypeID::Any,
            Value::Bool(_) => LogicalTypeID::Bool,
            Value::Int64(_) => LogicalTypeID::Int64,
            Value::Int32(_) => LogicalTypeID::Int32,
            Value::Int16(_) => LogicalTypeID::Int16,
            Value::Int8(_) => LogicalTypeID::Int8,
            Value::UInt64(_) => LogicalTypeID::UInt64,
            Value::UInt32(_) => LogicalTypeID::UInt32,
            Value::UInt16(_) => LogicalTypeID::UInt16,
            Value::UInt8(_) => LogicalTypeID::UInt8,
            Value::Double(_) => LogicalTypeID::Double,
            Value::Float(_) => LogicalTypeID::Float,
            Value::String(_) => LogicalTypeID::String,
            Value::Blob(_) => LogicalTypeID::Blob,
            Value::Date(_) => LogicalTypeID::Date,
            Value::Timestamp(_) => LogicalTypeID::Timestamp,
            Value::Interval(_) => LogicalTypeID::Interval,
            Value::InternalID(_) => LogicalTypeID::InternalID,
            Value::UInt128(_) => LogicalTypeID::UInt128,
            Value::Json(_) => LogicalTypeID::Json,
            Value::DTime(_) => LogicalTypeID::Time,
            Value::Union(_, _) => LogicalTypeID::Union,
            Value::List(_) => LogicalTypeID::List,
            Value::Map(_) => LogicalTypeID::Map,
            Value::Struct(_) => LogicalTypeID::Struct,
            Value::Int128(_) => LogicalTypeID::Int128,
            Value::TimestampTz(_) => LogicalTypeID::TimestampTz,
            Value::TimestampNs(_) => LogicalTypeID::TimestampNs,
            Value::TimestampMs(_) => LogicalTypeID::TimestampMs,
            Value::TimestampSec(_) => LogicalTypeID::TimestampSec,
        }
    }

    /// Get the PhysicalTypeID for this Value's logical type.
    #[inline(always)]
    pub fn physical_type(&self) -> PhysicalTypeID {
        physical_type_from_logical(self.logical_type())
    }
}

/// Map a LogicalTypeID to its corresponding PhysicalTypeID.
#[inline]
pub const fn physical_type_from_logical(logical: LogicalTypeID) -> PhysicalTypeID {
    match logical {
        LogicalTypeID::Any => PhysicalTypeID::Any,
        LogicalTypeID::Bool => PhysicalTypeID::Bool,
        LogicalTypeID::Int64 | LogicalTypeID::Serial => PhysicalTypeID::Int64,
        LogicalTypeID::Int32 => PhysicalTypeID::Int32,
        LogicalTypeID::Int16 => PhysicalTypeID::Int16,
        LogicalTypeID::Int8 => PhysicalTypeID::Int8,
        LogicalTypeID::UInt64 => PhysicalTypeID::UInt64,
        LogicalTypeID::UInt32 => PhysicalTypeID::UInt32,
        LogicalTypeID::UInt16 => PhysicalTypeID::UInt16,
        LogicalTypeID::UInt8 => PhysicalTypeID::UInt8,
        LogicalTypeID::Double => PhysicalTypeID::Double,
        LogicalTypeID::Float => PhysicalTypeID::Float,
        LogicalTypeID::Int128 | LogicalTypeID::Decimal | LogicalTypeID::UInt128 => PhysicalTypeID::Int128,
        LogicalTypeID::Date
        | LogicalTypeID::Timestamp
        | LogicalTypeID::TimestampSec
        | LogicalTypeID::TimestampMs
        | LogicalTypeID::TimestampNs
        | LogicalTypeID::TimestampTz
        | LogicalTypeID::Time => PhysicalTypeID::Int64,
        LogicalTypeID::Interval => PhysicalTypeID::Interval,
        LogicalTypeID::String | LogicalTypeID::Blob | LogicalTypeID::Uuid | LogicalTypeID::Json => {
            PhysicalTypeID::String
        }
        LogicalTypeID::InternalID => PhysicalTypeID::Struct,
        LogicalTypeID::List | LogicalTypeID::Array => PhysicalTypeID::List,
        LogicalTypeID::Map | LogicalTypeID::Struct | LogicalTypeID::Union => PhysicalTypeID::Struct,
        LogicalTypeID::Node | LogicalTypeID::Rel | LogicalTypeID::RecursiveRel => PhysicalTypeID::Struct,
    }
}

impl Date {
    /// Create a Date from epoch days.
    #[inline(always)]
    pub fn from_days_since_epoch(days: i32) -> Self {
        Date(days)
    }

    /// Get the days since epoch.
    #[inline(always)]
    pub fn days_since_epoch(&self) -> i32 {
        self.0
    }
}

impl Timestamp {
    /// Create a Timestamp from epoch microseconds.
    #[inline(always)]
    pub fn from_micros_since_epoch(micros: i64) -> Self {
        Timestamp(micros)
    }

    /// Get the microseconds since epoch.
    #[inline(always)]
    pub fn micros_since_epoch(&self) -> i64 {
        self.0
    }
}

impl Interval {
    pub fn new(months: i32, days: i32, micros: i64) -> Self {
        Interval { months, days, micros }
    }
}

/// Extract a `Vec<f64>` from a `Value` (expects `Value::List` of numbers).
///
/// Numeric list items (Double/Int64/Int32/Float) are coerced to `f64`; any
/// other item or a non-List value produces an error. Shared by the vector
/// extension and the vector-index write path (DRY, P51.41).
pub fn extract_f64_list(val: &Value) -> Result<Vec<f64>, String> {
    match val {
        Value::List(items) => {
            let mut result = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Value::Double(d) => result.push(*d),
                    Value::Int64(i) => result.push(*i as f64),
                    Value::Int32(i) => result.push(*i as f64),
                    Value::Float(f) => result.push(*f as f64),
                    other => {
                        return Err(format!("Expected numeric value in vector list, got {:?}", other));
                    }
                }
            }
            Ok(result)
        }
        other => Err(format!("Expected List value for vector, got {:?}", other)),
    }
}

/// Render a [`Value`] as the canonical string key used by hash indexes on
/// primary keys.
///
/// Single source of truth shared by `akar-storage` (index build, lookup,
/// delete) and `akar-main` (DDL/DML lookup paths) so every layer produces
/// byte-identical keys. Non-scalar values fall back to their `Debug` text.
pub fn pk_value_to_string(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int64(i) => i.to_string(),
        Value::Int32(i) => i.to_string(),
        Value::Int16(i) => i.to_string(),
        Value::Int8(i) => i.to_string(),
        Value::UInt64(u) => u.to_string(),
        Value::UInt32(u) => u.to_string(),
        Value::UInt16(u) => u.to_string(),
        Value::UInt8(u) => u.to_string(),
        Value::Double(f) => f.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => s.clone(),
        Value::Date(d) => format!("Date({})", d.0),
        Value::Timestamp(ts) => format!("Timestamp({})", ts.0),
        other => format!("{other:?}"),
    }
}

/// Render a [`Value`] as a CSV cell: `Null` becomes an empty field, anything
/// else uses the canonical scalar rendering ([`pk_value_to_string`]).
pub fn value_to_csv_string(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        other => pk_value_to_string(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_from_primitives() {
        assert_eq!(Value::from(true), Value::Bool(true));
        assert_eq!(Value::from(42i64), Value::Int64(42));
        assert_eq!(Value::from(42i32), Value::Int32(42));
        assert_eq!(Value::from(std::f64::consts::PI), Value::Double(std::f64::consts::PI));
        assert_eq!(Value::from("hello"), Value::String("hello".into()));
        assert_eq!(Value::from(Date(100)), Value::Date(Date(100)));
    }

    #[test]
    fn test_value_logical_type() {
        assert_eq!(Value::Null.logical_type(), LogicalTypeID::Any);
        assert_eq!(Value::Bool(true).logical_type(), LogicalTypeID::Bool);
        assert_eq!(Value::Int64(1).logical_type(), LogicalTypeID::Int64);
        assert_eq!(Value::String("a".into()).logical_type(), LogicalTypeID::String);
        assert_eq!(Value::List(vec![]).logical_type(), LogicalTypeID::List);
    }

    #[test]
    fn test_extract_f64_list() {
        assert_eq!(
            extract_f64_list(&Value::List(vec![
                Value::Double(1.5),
                Value::Int64(2),
                Value::Int32(3),
                Value::Float(4.0),
            ]))
            .unwrap(),
            vec![1.5, 2.0, 3.0, 4.0]
        );
        assert!(extract_f64_list(&Value::String("x".into())).is_err());
        assert!(extract_f64_list(&Value::List(vec![Value::Bool(true)])).is_err());
        assert_eq!(extract_f64_list(&Value::List(vec![])).unwrap(), Vec::<f64>::new());
    }

    #[test]
    fn test_physical_type_from_logical() {
        assert_eq!(physical_type_from_logical(LogicalTypeID::Bool), PhysicalTypeID::Bool);
        assert_eq!(physical_type_from_logical(LogicalTypeID::Int64), PhysicalTypeID::Int64);
        assert_eq!(
            physical_type_from_logical(LogicalTypeID::String),
            PhysicalTypeID::String
        );
        assert_eq!(physical_type_from_logical(LogicalTypeID::Date), PhysicalTypeID::Int64);
        assert_eq!(physical_type_from_logical(LogicalTypeID::List), PhysicalTypeID::List);
    }

    #[test]
    fn test_date_roundtrip() {
        let d = Date::from_days_since_epoch(20000);
        assert_eq!(d.days_since_epoch(), 20000);
    }

    #[test]
    fn test_timestamp_roundtrip() {
        let ts = Timestamp::from_micros_since_epoch(1_700_000_000_000_000);
        assert_eq!(ts.micros_since_epoch(), 1_700_000_000_000_000);
    }

    #[test]
    fn test_internal_id() {
        let id = InternalID {
            table_id: 5,
            offset: 100,
        };
        assert_eq!(id.table_id, 5);
        assert_eq!(id.offset, 100);
    }

    #[test]
    fn test_value_from_list() {
        let list = Value::List(vec![Value::Int64(1), Value::Int64(2)]);
        assert_eq!(list.logical_type(), LogicalTypeID::List);
    }

    #[test]
    fn test_value_physical_type() {
        let v: Value = 42i64.into();
        assert_eq!(v.physical_type(), PhysicalTypeID::Int64);
        let v: Value = std::f64::consts::PI.into();
        assert_eq!(v.physical_type(), PhysicalTypeID::Double);
        let v: Value = "test".into();
        assert_eq!(v.physical_type(), PhysicalTypeID::String);
    }

    #[test]
    fn test_pk_value_to_string_canonical_keys() {
        assert_eq!(pk_value_to_string(&Value::Null), "null");
        assert_eq!(pk_value_to_string(&Value::Bool(true)), "true");
        assert_eq!(pk_value_to_string(&Value::Int64(-7)), "-7");
        assert_eq!(pk_value_to_string(&Value::Int32(9)), "9");
        // Narrow integer/float widths must render as bare numerics (not
        // Debug text) so keys built here match lookups in every layer.
        assert_eq!(pk_value_to_string(&Value::Int16(5)), "5");
        assert_eq!(pk_value_to_string(&Value::Int8(3)), "3");
        assert_eq!(pk_value_to_string(&Value::UInt64(u64::MAX)), u64::MAX.to_string());
        assert_eq!(pk_value_to_string(&Value::UInt32(11)), "11");
        assert_eq!(pk_value_to_string(&Value::UInt16(12)), "12");
        assert_eq!(pk_value_to_string(&Value::UInt8(13)), "13");
        assert_eq!(pk_value_to_string(&Value::Double(1.5)), "1.5");
        assert_eq!(pk_value_to_string(&Value::Float(0.25)), "0.25");
        assert_eq!(pk_value_to_string(&Value::String("k".into())), "k");
        assert_eq!(
            pk_value_to_string(&Value::Date(Date(20000))),
            format!("Date({})", 20000)
        );
        assert_eq!(
            pk_value_to_string(&Value::Timestamp(Timestamp(1_700_000_000_000_000))),
            "Timestamp(1700000000000000)"
        );
        // Non-scalars fall back to Debug text.
        assert_eq!(
            pk_value_to_string(&Value::List(vec![Value::Int64(1)])),
            "List([Int64(1)])"
        );
    }

    #[test]
    fn test_value_to_csv_string_null_is_empty_cell() {
        assert_eq!(value_to_csv_string(&Value::Null), "");
        assert_eq!(value_to_csv_string(&Value::Bool(false)), "false");
        assert_eq!(value_to_csv_string(&Value::Int64(42)), "42");
        assert_eq!(value_to_csv_string(&Value::String("a,b".into())), "a,b");
        assert_eq!(value_to_csv_string(&Value::UInt8(7)), "7");
    }

    // --- format_value (P138: every Value variant renders) -----------------

    #[test]
    fn test_format_value_every_variant_renders() {
        let cases: Vec<Value> = vec![
            Value::Null,
            Value::Bool(true),
            Value::Int64(-7),
            Value::Int32(9),
            Value::Int16(5),
            Value::Int8(3),
            Value::UInt64(u64::MAX),
            Value::UInt32(11),
            Value::UInt16(12),
            Value::UInt8(13),
            Value::Int128(i128::MIN),
            Value::UInt128(u128::MAX),
            Value::Double(1.5),
            Value::Float(0.25),
            Value::DTime(3_661_000_000),
            Value::String("x".into()),
            Value::Blob(vec![0xde, 0xad, 0xbe, 0xef]),
            Value::Date(Date(0)),
            Value::Timestamp(Timestamp(0)),
            Value::TimestampTz(TimestampTZ(0)),
            Value::TimestampNs(Timestamp(0)),
            Value::TimestampMs(Timestamp(0)),
            Value::TimestampSec(Timestamp(0)),
            Value::Interval(Interval {
                months: 2,
                days: 3,
                micros: 4000,
            }),
            Value::InternalID(InternalID { table_id: 1, offset: 2 }),
            Value::Json(serde_json::json!({"a": 1})),
            Value::Union("u".into(), Box::new(Value::Int64(4))),
            Value::List(vec![Value::Int64(1), Value::String("a".into())]),
            Value::Map(vec![(Value::String("k".into()), Value::Int64(2))]),
            Value::Struct(vec![("f".to_string(), Value::Bool(true))]),
        ];
        for v in &cases {
            let rendered = format_value(v);
            assert!(!rendered.is_empty(), "variant {v:?} rendered an empty string");
            assert!(rendered != "<val>", "variant {v:?} still renders the placeholder");
        }
    }

    #[test]
    fn test_format_value_exact_rendering() {
        assert_eq!(format_value(&Value::Null), "NULL");
        assert_eq!(format_value(&Value::Bool(true)), "true");
        assert_eq!(format_value(&Value::Int64(-7)), "-7");
        assert_eq!(format_value(&Value::UInt64(42)), "42");
        assert_eq!(format_value(&Value::String("hi".into())), "hi");
        // Blob renders as tagged lowercase hex.
        assert_eq!(format_value(&Value::Blob(vec![0xde, 0xad])), "x'dead'");
        // Date/timestamps render ISO 8601 from epoch-relative counts.
        assert_eq!(format_value(&Value::Date(Date(0))), "1970-01-01");
        assert_eq!(format_value(&Value::Date(Date(20_000))), "2024-10-04");
        assert_eq!(format_value(&Value::Timestamp(Timestamp(0))), "1970-01-01T00:00:00");
        assert_eq!(
            format_value(&Value::Timestamp(Timestamp(1_700_000_000_000_000))),
            "2023-11-14T22:13:20"
        );
        assert_eq!(
            format_value(&Value::TimestampTz(TimestampTZ(0))),
            "1970-01-01T00:00:00+00:00"
        );
        assert_eq!(format_value(&Value::DTime(3_661_000_000)), "01:01:01");
        assert_eq!(
            format_value(&Value::Interval(Interval {
                months: 2,
                days: 3,
                micros: 4000
            })),
            "2 months 3 days 4000 micros"
        );
        assert_eq!(
            format_value(&Value::InternalID(InternalID { table_id: 1, offset: 2 })),
            "1:2"
        );
        assert_eq!(format_value(&Value::Json(serde_json::json!({"a": 1}))), "{\"a\":1}");
        assert_eq!(
            format_value(&Value::Union("u".into(), Box::new(Value::Int64(4)))),
            "u(4)"
        );
        assert_eq!(
            format_value(&Value::List(vec![Value::Int64(1), Value::String("a".into())])),
            "[1, a]"
        );
        assert_eq!(
            format_value(&Value::Map(vec![(Value::String("k".into()), Value::Int64(2))])),
            "{k: 2}"
        );
        assert_eq!(
            format_value(&Value::Struct(vec![("f".to_string(), Value::Bool(true))])),
            "{f: true}"
        );
    }

    #[test]
    fn test_days_to_iso_known_dates() {
        assert_eq!(days_to_iso(0), "1970-01-01");
        assert_eq!(days_to_iso(-1), "1969-12-31");
        assert_eq!(days_to_iso(19_000), "2022-01-08");
        // Leap day: 2024-02-29 is day 19782, its neighbours bound it.
        assert_eq!(days_to_iso(19_781), "2024-02-28");
        assert_eq!(days_to_iso(19_782), "2024-02-29");
        assert_eq!(days_to_iso(19_783), "2024-03-01");
    }
}
