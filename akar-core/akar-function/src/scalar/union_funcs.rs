use super::get_string;
use crate::registry::*;
use akar_common::types::Value;

// ==================== Union functions ====================

/// Evaluate a union function.
pub(crate) fn evaluate_union(op: UnionOp, args: &[Value]) -> Result<Value, String> {
    match op {
        UnionOp::UnionValue => {
            // UNION_VALUE(val) → create a union wrapping the value as a single variant
            let val = args[0].clone();
            Ok(Value::Struct(vec![
                ("tag".to_string(), Value::UInt16(0)),
                ("_value".to_string(), val),
            ]))
        }
        UnionOp::UnionTag => {
            // UNION_TAG(union) → return the active tag name as a string
            let entries = match &args[0] {
                Value::Struct(entries) => entries,
                _ => return Err("UNION_TAG requires a union argument".into()),
            };
            // Find the tag field (should be first entry)
            let tag_val = entries
                .iter()
                .find(|(k, _)| k == "tag")
                .ok_or("Union has no tag field".to_string())?;
            let tag_idx = match &tag_val.1 {
                Value::UInt16(x) => *x as usize,
                _ => return Err("Invalid tag field type".into()),
            };
            // The active variant name is at entries[tag_idx + 1]
            let field_idx = tag_idx + 1;
            if field_idx >= entries.len() {
                return Err(format!("Union tag index {} out of range", tag_idx));
            }
            Ok(Value::String(entries[field_idx].0.clone()))
        }
        UnionOp::UnionExtract => {
            // UNION_EXTRACT(union, key) → same as struct_extract
            let struct_val = &args[0];
            let key = get_string(&args[1])?;
            match struct_val {
                Value::Struct(entries) => {
                    for (k, v) in entries {
                        if *k == key {
                            return Ok(v.clone());
                        }
                    }
                    Err(format!("Key '{}' not found in union", key))
                }
                _ => Err("UNION_EXTRACT requires a union argument".into()),
            }
        }
    }
}
