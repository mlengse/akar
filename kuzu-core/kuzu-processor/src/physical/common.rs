//! Common utility functions used across physical operators.

use kuzu_common::types::{PhysicalTypeID, Value};
use kuzu_common::vector::ValueVector;

#[inline]
pub(crate) fn store_value_in_vector(v: &mut ValueVector, row: usize, val: &Value) {
    match val {
        Value::Null => { v.set_null(row, true); }
        Value::Bool(x) => {
            if v.physical_type() == PhysicalTypeID::Bool {
                v.data_mut()[row] = if *x { 1 } else { 0 };
                v.set_null(row, false);
            }
        }
        Value::Int64(x) => {
            let offset = row * 8;
            if offset + 8 <= v.data().len() {
                v.data_mut()[offset..offset + 8].copy_from_slice(&x.to_le_bytes());
                v.set_null(row, false);
            }
        }
        Value::Int32(x) => {
            let offset = row * 4;
            if offset + 4 <= v.data().len() {
                v.data_mut()[offset..offset + 4].copy_from_slice(&x.to_le_bytes());
                v.set_null(row, false);
            }
        }
        Value::Double(x) => {
            let offset = row * 8;
            if offset + 8 <= v.data().len() {
                v.data_mut()[offset..offset + 8].copy_from_slice(&x.to_le_bytes());
                v.set_null(row, false);
            }
        }
        Value::Float(x) => {
            let offset = row * 4;
            if offset + 4 <= v.data().len() {
                v.data_mut()[offset..offset + 4].copy_from_slice(&x.to_le_bytes());
                v.set_null(row, false);
            }
        }
        Value::String(s) => {
            let offset = row * 256;
            let bytes = s.as_bytes();
            let len = bytes.len().min(255) as u8;
            if offset < v.data().len() {
                v.data_mut()[offset] = len;
                let copy_len = bytes.len().min(255);
                if offset + 1 + copy_len <= v.data().len() {
                    v.data_mut()[offset + 1..offset + 1 + copy_len].copy_from_slice(&bytes[..copy_len]);
                }
                v.set_null(row, false);
            }
        }
        _ => { v.set_null(row, true); }
    }
}

#[inline(always)]
pub(crate) fn value_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => std::cmp::Ordering::Equal,
        (Value::Null, _) => std::cmp::Ordering::Greater,
        (_, Value::Null) => std::cmp::Ordering::Less,
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Int64(x), Value::Int64(y)) => x.cmp(y),
        (Value::Int32(x), Value::Int32(y)) => x.cmp(y),
        (Value::Int16(x), Value::Int16(y)) => (*x as i64).cmp(&(*y as i64)),
        (Value::Int8(x), Value::Int8(y)) => (*x as i64).cmp(&(*y as i64)),
        (Value::UInt64(x), Value::UInt64(y)) => x.cmp(y),
        (Value::UInt32(x), Value::UInt32(y)) => (*x as u64).cmp(&(*y as u64)),
        (Value::Double(x), Value::Double(y)) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Date(x), Value::Date(y)) => x.0.cmp(&y.0),
        (Value::Timestamp(x), Value::Timestamp(y)) => x.0.cmp(&y.0),
        _ => std::cmp::Ordering::Equal,
    }
}

#[inline]
pub(crate) fn value_hash(val: &Value) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hash_value_into(val, &mut hasher);
    hasher.finish()
}

/// Write a Value's hash into an arbitrary Hasher.
#[inline]
pub(crate) fn hash_value_into(val: &Value, hasher: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    match val {
        Value::Null => 0u8.hash(hasher),
        Value::Bool(b) => b.hash(hasher),
        Value::Int64(i) => i.hash(hasher),
        Value::Int32(i) => i.hash(hasher),
        Value::Int16(i) => i.hash(hasher),
        Value::Int8(i) => i.hash(hasher),
        Value::UInt64(i) => i.hash(hasher),
        Value::UInt32(i) => i.hash(hasher),
        Value::UInt16(i) => (*i as u64).hash(hasher),
        Value::UInt8(i) => (*i as u64).hash(hasher),
        Value::Int128(i) => i.hash(hasher),
        Value::Double(f) => f.to_bits().hash(hasher),
        Value::Float(f) => f.to_bits().hash(hasher),
        Value::String(s) => s.hash(hasher),
        Value::Blob(b) => b.hash(hasher),
        Value::Date(d) => d.0.hash(hasher),
        Value::Timestamp(t) => t.0.hash(hasher),
        Value::TimestampTz(t) => t.0.hash(hasher),
        Value::TimestampNs(t) => t.0.hash(hasher),
        Value::TimestampMs(t) => t.0.hash(hasher),
        Value::TimestampSec(t) => t.0.hash(hasher),
        Value::Interval(i) => (i.months, i.days, i.micros).hash(hasher),
        Value::InternalID(id) => id.offset.hash(hasher),
        Value::UInt128(i) => i.hash(hasher),
        Value::List(vals) => {
            for v in vals {
                hash_value_into(v, hasher);
            }
        }
        Value::Map(kvs) => {
            for (k, v) in kvs {
                hash_value_into(k, hasher);
                hash_value_into(v, hasher);
            }
        }
        Value::Union(_, v) => hash_value_into(v, hasher),
        _ => std::mem::discriminant(val).hash(hasher),
    }
}

/// Fast hash using ahash — ~3-5× faster than SipHash for integer keys.
/// Used in the hot path of JoinHashTable build/probe.
#[inline(always)]
pub(crate) fn value_hash_fast(val: &Value) -> u64 {
    use std::hash::Hasher;
    let mut hasher = ahash::AHasher::default();
    hash_value_into(val, &mut hasher);
    hasher.finish()
}

/// Hash raw i64 key bytes directly — avoids Value boxing entirely.
#[inline]
#[allow(dead_code)]
pub(crate) fn raw_key_hash_i64(val: i64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = ahash::AHasher::default();
    val.hash(&mut hasher);
    hasher.finish()
}

/// Hash raw bytes for string keys.
#[inline]
#[allow(dead_code)]
pub(crate) fn raw_key_hash_bytes(val: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = ahash::AHasher::default();
    val.hash(&mut hasher);
    hasher.finish()
}

