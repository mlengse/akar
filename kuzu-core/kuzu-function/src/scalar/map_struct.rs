use crate::registry::*;
use kuzu_common::types::Value;
use super::{get_string};


// ==================== Map & Struct ====================

pub(crate) fn evaluate_map(op: MapOp, args: &[Value]) -> Result<Value, String> {
    match op {
        MapOp::Creation => {
            // map_creation(args): args are alternating key, value, key, value, ...
            if args.len() % 2 != 0 {
                return Err("Map creation requires an even number of arguments (key-value pairs)".into());
            }
            let mut entries = Vec::new();
            let mut i = 0;
            while i < args.len() {
                let key = match &args[i] {
                    Value::String(s) => s.clone(),
                    other => return Err(format!("Map key must be a string, got {:?}", other.logical_type())),
                };
                let val = args[i + 1].clone();
                entries.push((key, val));
                i += 2;
            }
            Ok(Value::Struct(entries))
        }
        MapOp::MapFromEntries => {
            match &args[0] {
                Value::List(items) => {
                    let mut result_entries = Vec::new();
                    for item in items {
                        match item {
                            Value::Struct(s) => {
                                let mut k = None;
                                let mut v = None;
                                for (fname, fval) in s {
                                    if fname == "key" || fname == "k" {
                                        k = Some(fval);
                                    } else if fname == "value" || fname == "v" {
                                        v = Some(fval);
                                    }
                                }
                                if let (Some(Value::String(k_str)), Some(val)) = (k, v) {
                                    result_entries.push((k_str.clone(), val.clone()));
                                } else {
                                    // fallback: use first and second field
                                    if s.len() >= 2 {
                                        if let Value::String(k_str) = &s[0].1 {
                                            result_entries.push((k_str.clone(), s[1].1.clone()));
                                        }
                                    }
                                }
                            }
                            _ => return Err("MapFromEntries requires a list of structs".into()),
                        }
                    }
                    Ok(Value::Struct(result_entries))
                }
                _ => Err("MapFromEntries requires a list of structs".into()),
            }
        }
        MapOp::Extract => {
            let map_val = &args[0];
            let key = get_string(&args[1])?;
            match map_val {
                Value::Struct(entries) => {
                    for (k, v) in entries {
                        if *k == key {
                            return Ok(v.clone());
                        }
                    }
                    Err(format!("Key '{}' not found in map", key))
                }
                _ => Err("Expected map/struct".into()),
            }
        }
        MapOp::Keys => match &args[0] {
            Value::Struct(entries) => Ok(Value::List(
                entries.iter().map(|(k, _)| Value::String(k.clone())).collect(),
            )),
            _ => Err("Expected map/struct".into()),
        },
        MapOp::Values => match &args[0] {
            Value::Struct(entries) => Ok(Value::List(entries.iter().map(|(_, v)| v.clone()).collect())),
            _ => Err("Expected map/struct".into()),
        },
        MapOp::Contains => match &args[0] {
            Value::Struct(entries) => {
                let key = get_string(&args[1])?;
                Ok(Value::Bool(entries.iter().any(|(k, _)| *k == key)))
            }
            _ => Err("Expected map/struct".into()),
        },
    }
}

pub(crate) fn evaluate_struct(op: StructOp, args: &[Value]) -> Result<Value, String> {
    match op {
        StructOp::Creation => {
            // struct_creation(args): args are alternating field_name (string), value, ...
            if args.len() % 2 != 0 {
                return Err("Struct creation requires an even number of arguments (field-value pairs)".into());
            }
            let mut entries = Vec::new();
            let mut i = 0;
            while i < args.len() {
                let field_name = match &args[i] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "Struct field name must be a string, got {:?}",
                            other.logical_type()
                        ));
                    }
                };
                let val = args[i + 1].clone();
                entries.push((field_name, val));
                i += 2;
            }
            Ok(Value::Struct(entries))
        }
        StructOp::Extract => {
            let struct_val = &args[0];
            let key = get_string(&args[1])?;
            match struct_val {
                Value::Struct(entries) => {
                    for (k, v) in entries {
                        if *k == key {
                            return Ok(v.clone());
                        }
                    }
                    Err(format!("Key '{}' not found in struct", key))
                }
                _ => Err("Expected struct".into()),
            }
        }
    }
}
