use super::get_string;
use crate::registry::*;
use akar_common::types::Value;
use md5::{Digest, Md5};
use sha2::Sha256;

// ==================== Hash functions ====================

/// Simple non-cryptographic hash for any Value (matching C++ murmurhash64 semantics).
fn hash_value(v: &Value) -> u64 {
    match v {
        Value::Null => u64::MAX,
        Value::Bool(b) => murmur64(*b as u64),
        Value::Int64(x) => murmur64(*x as u64),
        Value::Int32(x) => murmur64(*x as u64),
        Value::Double(x) => {
            if *x == 0.0 {
                murmur64(0)
            } else {
                murmur64(x.to_bits())
            }
        }
        Value::String(s) => hash_string(s),
        Value::List(items) => {
            let mut h: u64 = 0;
            for item in items {
                h = combine_hash(h, hash_value(item));
            }
            h
        }
        _ => {
            let s = format!("{:?}", v);
            hash_string(&s)
        }
    }
}

fn murmur64(mut x: u64) -> u64 {
    x ^= x >> 32;
    x = x.wrapping_mul(0xd6e8feb86659fd93);
    x ^= x >> 32;
    x = x.wrapping_mul(0xd6e8feb86659fd93);
    x ^= x >> 32;
    x
}

fn combine_hash(a: u64, b: u64) -> u64 {
    a.wrapping_mul(0xbf58476d1ce4e5b9) ^ b
}

fn hash_string(s: &str) -> u64 {
    let bytes = s.as_bytes();
    let mut h: u64 = 0;
    for chunk in bytes.chunks(8) {
        let mut val: u64 = 0;
        for (i, &b) in chunk.iter().enumerate() {
            val |= (b as u64) << (i * 8);
        }
        h = combine_hash(h, murmur64(val));
    }
    h
}

pub(crate) fn evaluate_hash(op: HashOp, args: &[Value]) -> Result<Value, String> {
    match op {
        HashOp::Md5 => {
            let s = get_string(&args[0])?;
            let mut hasher = Md5::new();
            hasher.update(s.as_bytes());
            let result = hasher.finalize();
            Ok(Value::String(result.iter().map(|b| format!("{:02x}", b)).collect()))
        }
        HashOp::Sha256 => {
            let s = get_string(&args[0])?;
            let mut hasher = Sha256::new();
            hasher.update(s.as_bytes());
            let result = hasher.finalize();
            Ok(Value::String(result.iter().map(|b| format!("{:02x}", b)).collect()))
        }
        HashOp::Hash => {
            if args.is_empty() {
                return Err("hash requires at least one argument".into());
            }
            let h = hash_value(&args[0]);
            Ok(Value::Int64(h as i64))
        }
    }
}
