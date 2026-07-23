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
}
