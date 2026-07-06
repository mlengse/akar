use crate::registry::*;
use kuzu_common::types::Value;

// ==================== Path ====================

pub(crate) fn evaluate_path(op: PathOp, args: &[Value]) -> Result<Value, String> {
    match op {
        PathOp::Nodes => {
            let path = &args[0];
            match path {
                Value::Struct(fields) => {
                    // Look for "_nodes" field in struct
                    if let Some((_, nodes_val)) = fields.iter().find(|(k, _)| k == "_nodes") {
                        Ok(nodes_val.clone())
                    } else if let Some((_, first)) = fields.first() {
                        // Fallback: return first field (usually nodes list)
                        Ok(first.clone())
                    } else {
                        Ok(Value::Null)
                    }
                }
                Value::List(_) => Ok(path.clone()),
                _ => Err(format!("NODES() requires a path/recursive rel, got {:?}", path)),
            }
        }
        PathOp::Rels => {
            let path = &args[0];
            match path {
                Value::Struct(fields) => {
                    if let Some((_, rels_val)) = fields.iter().find(|(k, _)| k == "_rels") {
                        Ok(rels_val.clone())
                    } else if fields.len() >= 2 {
                        Ok(fields[1].1.clone())
                    } else {
                        Ok(Value::Null)
                    }
                }
                _ => Err(format!("RELS() requires a path/recursive rel, got {:?}", path)),
            }
        }
        PathOp::Length => {
            let path = &args[0];
            match path {
                Value::List(items) => Ok(Value::Int64(items.len() as i64)),
                Value::Struct(fields) => {
                    // Count entries in _rels or _nodes minus 1
                    if let Some((_, Value::List(rels))) = fields.iter().find(|(k, _)| k == "_rels") {
                        Ok(Value::Int64(rels.len() as i64))
                    } else {
                        Ok(Value::Int64(0))
                    }
                }
                _ => Err(format!("LENGTH() requires a path/recursive rel, got {:?}", path)),
            }
        }
    }
}

/// Generate a random UUID v4 string.
pub(crate) fn evaluate_uuid(_args: &[Value]) -> Result<Value, String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    // Simple UUID v4 generation without external crate dependency
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let mut seed = now.as_nanos() as u64;
    // Simple PRNG (xorshift64*)
    seed ^= seed >> 12;
    seed ^= seed << 25;
    seed ^= seed >> 27;
    let r1 = seed.wrapping_mul(0x2545F4914F6CDD1Du64);
    seed ^= seed >> 12;
    seed ^= seed << 25;
    seed ^= seed >> 27;
    let r2 = seed.wrapping_mul(0x2545F4914F6CDD1Du64);

    // Format as UUID v4: 8-4-4-4-12 hex digits
    let time_low = (r1 & 0xFFFFFFFF) as u32;
    let time_mid = ((r1 >> 32) & 0xFFFF) as u16;
    let time_hi_and_version = (((r1 >> 48) & 0x0FFF) | 0x4000) as u16; // version 4
    let clock_seq = ((r2 & 0x3FFF) | 0x8000) as u16; // variant 1
    let node_low = ((r2 >> 14) & 0xFFFFFFFF) as u32;
    let node_hi = ((r2 >> 46) & 0xFFFF) as u16;

    Ok(Value::String(format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:04x}{:08x}",
        time_low, time_mid, time_hi_and_version, clock_seq, node_hi, node_low
    )))
}
