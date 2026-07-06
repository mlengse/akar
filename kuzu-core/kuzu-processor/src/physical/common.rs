//! Common utility functions used across physical operators.

use kuzu_common::types::{PhysicalTypeID, Value};
use kuzu_common::vector::ValueVector;

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

pub(crate) fn value_hash(val: &Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match val {
        Value::Null => 0u8.hash(&mut hasher),
        Value::Bool(b) => b.hash(&mut hasher),
        Value::Int64(i) => i.hash(&mut hasher),
        Value::Int32(i) => i.hash(&mut hasher),
        Value::Int16(i) => i.hash(&mut hasher),
        Value::Int8(i) => i.hash(&mut hasher),
        Value::UInt64(i) => i.hash(&mut hasher),
        Value::Double(f) => f.to_bits().hash(&mut hasher),
        Value::Float(f) => f.to_bits().hash(&mut hasher),
        Value::String(s) => s.hash(&mut hasher),
        Value::Date(d) => d.0.hash(&mut hasher),
        Value::Timestamp(t) => t.0.hash(&mut hasher),
        Value::InternalID(id) => id.offset.hash(&mut hasher),
        _ => std::mem::discriminant(val).hash(&mut hasher),
    }
    hasher.finish()
}
