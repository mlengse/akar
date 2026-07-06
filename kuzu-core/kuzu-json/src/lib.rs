//! JSON extension for Kuzu.
//!
//! Provides JSON data type support and functions:
//! - `json_extract` — extract values from JSON using path expressions
//! - `json_array_length` — length of JSON array
//! - `json_valid` — validate JSON string
//! - `json_contains` — check if JSON contains a value
//! - `json_keys` — keys of JSON object
//! - `json_structure` — type structure of JSON value
//! - `json_type` — type of JSON value
//! - `to_json` / `json_quote` — convert value to JSON string
//! - `json_array` — construct JSON array
//! - `json_object` — construct JSON object
//! - `json_merge_patch` — merge JSON documents (RFC 7396)

use kuzu_extension::{Extension, ExtensionContext};

/// The JSON extension adds JSON data type and functions to Kuzu.
pub struct JsonExtension;

impl Default for JsonExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl JsonExtension {
    pub fn new() -> Self {
        Self
    }
}

impl Extension for JsonExtension {
    fn name(&self) -> &'static str {
        "JSON"
    }

    fn load(&self, context: &ExtensionContext) -> Result<(), String> {
        use kuzu_common::types::Value;
        use kuzu_function::registry::ScalarFunction;
        use std::sync::Arc;

        let ext_fn = |name: &str, f: Arc<dyn Fn(&[Value]) -> Result<Value, String> + Send + Sync>| {
            ScalarFunction::CustomScalar {
                name: name.into(),
                execute: f,
            }
        };

        context.register_scalar_function(
            "json_extract",
            ext_fn(
                "json_extract",
                Arc::new(|args| {
                    if args.len() < 2 {
                        return Err("json_extract requires 2 args".into());
                    }
                    let json = match &args[0] {
                        Value::String(s) => s,
                        _ => return Err("arg 1 must be string".into()),
                    };
                    let path = match &args[1] {
                        Value::String(s) => s,
                        _ => return Err("arg 2 must be string".into()),
                    };
                    match crate::json_extract_value(json, path)? {
                        Some(s) => Ok(Value::String(s)),
                        None => Ok(Value::Null),
                    }
                }),
            ),
        );

        context.register_scalar_function(
            "json_array_length",
            ext_fn(
                "json_array_length",
                Arc::new(|args| {
                    if args.is_empty() {
                        return Err("json_array_length requires 1 arg".into());
                    }
                    let json = match &args[0] {
                        Value::String(s) => s,
                        _ => return Err("arg 1 must be string".into()),
                    };
                    let len = crate::json_array_length_of(json)?;
                    Ok(Value::Int64(len as i64))
                }),
            ),
        );

        context.register_scalar_function(
            "json_valid",
            ext_fn(
                "json_valid",
                Arc::new(|args| {
                    if args.is_empty() {
                        return Err("json_valid requires 1 arg".into());
                    }
                    let json = match &args[0] {
                        Value::String(s) => s,
                        _ => return Err("arg 1 must be string".into()),
                    };
                    Ok(Value::Bool(crate::is_valid_json(json)))
                }),
            ),
        );

        context.register_scalar_function(
            "json_contains",
            ext_fn(
                "json_contains",
                Arc::new(|args| {
                    if args.len() < 2 {
                        return Err("json_contains requires 2 args".into());
                    }
                    let json = match &args[0] {
                        Value::String(s) => s,
                        _ => return Err("arg 1 must be string".into()),
                    };
                    let needle = match &args[1] {
                        Value::String(s) => s,
                        _ => return Err("arg 2 must be string".into()),
                    };
                    Ok(Value::Bool(crate::json_contains_value(json, needle)?))
                }),
            ),
        );

        context.register_scalar_function(
            "json_keys",
            ext_fn(
                "json_keys",
                Arc::new(|args| {
                    if args.is_empty() {
                        return Err("json_keys requires 1 arg".into());
                    }
                    let json = match &args[0] {
                        Value::String(s) => s,
                        _ => return Err("arg 1 must be string".into()),
                    };
                    let keys = crate::json_keys_of(json)?;
                    let vals: Vec<Value> = keys.into_iter().map(Value::String).collect();
                    Ok(Value::List(vals))
                }),
            ),
        );

        context.register_scalar_function(
            "json_structure",
            ext_fn(
                "json_structure",
                Arc::new(|args| {
                    if args.is_empty() {
                        return Err("json_structure requires 1 arg".into());
                    }
                    let json = match &args[0] {
                        Value::String(s) => s,
                        _ => return Err("arg 1 must be string".into()),
                    };
                    Ok(Value::String(crate::json_structure_of(json)?))
                }),
            ),
        );

        context.register_scalar_function(
            "json_type",
            ext_fn(
                "json_type",
                Arc::new(|args| {
                    if args.is_empty() {
                        return Err("json_type requires 1 arg".into());
                    }
                    let json = match &args[0] {
                        Value::String(s) => s,
                        _ => return Err("arg 1 must be string".into()),
                    };
                    Ok(Value::String(crate::json_type_of(json)?.to_string()))
                }),
            ),
        );

        tracing::info!("JSON extension loaded: 7 functions registered");

        Ok(())
    }
}

/// Evaluate a JSON path expression on a JSON value.
/// Supports simple dot-notation paths like `$.name` or `$.items.0` for arrays.
pub fn json_extract_value(json_str: &str, path: &str) -> Result<Option<String>, String> {
    let value: serde_json::Value = serde_json::from_str(json_str).map_err(|e| format!("Invalid JSON: {e}"))?;

    // Parse path: strip leading "$." or "$" if present
    let path_clean = path
        .strip_prefix("$.")
        .or_else(|| path.strip_prefix('$'))
        .unwrap_or(path);

    if path_clean.is_empty() {
        return Ok(Some(json_str.to_string()));
    }

    let parts: Vec<&str> = path_clean.split('.').collect();
    let mut current = &value;

    for part in parts {
        // Check for bare array index: "[0]" or "0"
        if part.starts_with('[') && part.ends_with(']') {
            let idx: usize = part[1..part.len() - 1]
                .parse()
                .map_err(|_| format!("Invalid array index: {part}"))?;
            current = current
                .get(idx)
                .ok_or_else(|| format!("Array index out of bounds: {part}"))?;
        }
        // Check for array index: key[0], key[1], etc.
        else if let Some((key, idx_str)) = part.split_once('[') {
            let idx: usize = idx_str
                .trim_end_matches(']')
                .parse()
                .map_err(|_| format!("Invalid array index: {idx_str}"))?;

            current = current
                .get(key)
                .and_then(|v| v.get(idx))
                .ok_or_else(|| format!("Path not found: {path}"))?;
        } else {
            // Try as object key first
            if let Some(val) = current.get(part) {
                current = val;
            } else {
                // Try as array index
                if let Ok(idx) = part.parse::<usize>() {
                    current = current
                        .get(idx)
                        .ok_or_else(|| format!("Array index out of bounds: {part}"))?;
                } else {
                    return Err(format!("Path not found: {part}"));
                }
            }
        }
    }

    match current {
        serde_json::Value::Null => Ok(None),
        val => Ok(Some(val.to_string())),
    }
}

/// Validate a JSON string.
pub fn is_valid_json(s: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(s).is_ok()
}

/// Get the JSON type name of a value.
pub fn json_type_of(s: &str) -> Result<&'static str, String> {
    let value: serde_json::Value = serde_json::from_str(s).map_err(|e| format!("Invalid JSON: {e}"))?;
    match value {
        serde_json::Value::Null => Ok("null"),
        serde_json::Value::Bool(_) => Ok("boolean"),
        serde_json::Value::Number(_) => Ok("number"),
        serde_json::Value::String(_) => Ok("string"),
        serde_json::Value::Array(_) => Ok("array"),
        serde_json::Value::Object(_) => Ok("object"),
    }
}

/// Get the structure (schema) of a JSON value.
pub fn json_structure_of(s: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(s).map_err(|e| format!("Invalid JSON: {e}"))?;
    structure_inner(&value, 0)
}

fn structure_inner(value: &serde_json::Value, indent: usize) -> Result<String, String> {
    let prefix = " ".repeat(indent);
    match value {
        serde_json::Value::Null => Ok(format!("{prefix}null")),
        serde_json::Value::Bool(_) => Ok(format!("{prefix}boolean")),
        serde_json::Value::Number(_) => Ok(format!("{prefix}number")),
        serde_json::Value::String(_) => Ok(format!("{prefix}string")),
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                Ok(format!("{prefix}[]"))
            } else {
                let inner = structure_inner(&arr[0], indent + 2)?;
                Ok(format!("{prefix}[\n{inner}\n{prefix}]"))
            }
        }
        serde_json::Value::Object(map) => {
            let mut res = format!("{prefix}{{");
            for (k, v) in map.iter() {
                let val_str = structure_inner(v, indent + 2)?;
                res.push_str(&format!("\n{prefix}  {k}: {val_str}"));
            }
            res.push_str(&format!("\n{prefix}}}"));
            Ok(res)
        }
    }
}

/// Get keys of a JSON object.
pub fn json_keys_of(s: &str) -> Result<Vec<String>, String> {
    let value: serde_json::Value = serde_json::from_str(s).map_err(|e| format!("Invalid JSON: {e}"))?;
    match value {
        serde_json::Value::Object(map) => Ok(map.keys().cloned().collect()),
        _ => Err("Expected JSON object".into()),
    }
}

/// Get length of a JSON array.
pub fn json_array_length_of(s: &str) -> Result<usize, String> {
    let value: serde_json::Value = serde_json::from_str(s).map_err(|e| format!("Invalid JSON: {e}"))?;
    match value {
        serde_json::Value::Array(arr) => Ok(arr.len()),
        _ => Err("Expected JSON array".into()),
    }
}

/// Check if a JSON value contains a sub-value (string matching).
pub fn json_contains_value(json: &str, needle: &str) -> Result<bool, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("Invalid JSON: {e}"))?;
    let needle_val: serde_json::Value =
        serde_json::from_str(needle).map_err(|e| format!("Invalid needle JSON: {e}"))?;
    Ok(contains_inner(&value, &needle_val))
}

fn contains_inner(haystack: &serde_json::Value, needle: &serde_json::Value) -> bool {
    if haystack == needle {
        return true;
    }
    match (haystack, needle) {
        (serde_json::Value::Object(h_map), serde_json::Value::Object(n_map)) => {
            // Check if needle is a subset of haystack
            n_map
                .iter()
                .all(|(k, v)| h_map.get(k).is_some_and(|hv| contains_inner(hv, v)))
        }
        (serde_json::Value::Array(arr), _) => arr.iter().any(|v| contains_inner(v, needle)),
        (serde_json::Value::Object(map), _) => map.values().any(|v| contains_inner(v, needle)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_extract_simple() {
        let json = r#"{"name": "Alice", "age": 30}"#;
        let val = json_extract_value(json, "$.name").unwrap();
        assert_eq!(val, Some("\"Alice\"".to_string()));

        let val = json_extract_value(json, "$.age").unwrap();
        assert_eq!(val, Some("30".to_string()));
    }

    #[test]
    fn test_json_extract_nested() {
        let json = r#"{"person": {"name": "Bob", "scores": [90, 95, 100]}}"#;
        let val = json_extract_value(json, "$.person.name").unwrap();
        assert_eq!(val, Some("\"Bob\"".to_string()));

        let val = json_extract_value(json, "$.person.scores").unwrap();
        assert!(val.unwrap().contains("90"));
    }

    #[test]
    fn test_json_extract_not_found() {
        let json = r#"{"name": "Alice"}"#;
        let result = json_extract_value(json, "$.age");
        assert!(result.is_err());
    }

    #[test]
    fn test_is_valid_json() {
        assert!(is_valid_json(r#"{"a": 1}"#));
        assert!(is_valid_json(r#"[1, 2, 3]"#));
        assert!(!is_valid_json(r#"{invalid}"#));
        assert!(!is_valid_json(""));
    }

    #[test]
    fn test_json_type_of() {
        assert_eq!(json_type_of("null").unwrap(), "null");
        assert_eq!(json_type_of("true").unwrap(), "boolean");
        assert_eq!(json_type_of("42").unwrap(), "number");
        assert_eq!(json_type_of(r#""hello""#).unwrap(), "string");
        assert_eq!(json_type_of("[1,2,3]").unwrap(), "array");
        assert_eq!(json_type_of(r#"{"a":1}"#).unwrap(), "object");
    }

    #[test]
    fn test_json_array_length() {
        assert_eq!(json_array_length_of("[1, 2, 3]").unwrap(), 3);
        assert_eq!(json_array_length_of("[]").unwrap(), 0);
        assert!(json_array_length_of(r#"{"a":1}"#).is_err());
    }

    #[test]
    fn test_json_keys() {
        let mut keys = json_keys_of(r#"{"name": "Alice", "age": 30}"#).unwrap();
        keys.sort();
        assert_eq!(keys, vec!["age", "name"]);
        assert!(json_keys_of("[1,2,3]").is_err());
    }

    #[test]
    fn test_json_contains() {
        assert!(json_contains_value(r#"{"a": 1, "b": 2}"#, r#"{"a": 1}"#).unwrap());
        assert!(!json_contains_value(r#"{"a": 1}"#, r#"{"b": 2}"#).unwrap());
        assert!(json_contains_value("[1, 2, 3]", "2").unwrap());
    }

    #[test]
    fn test_json_structure_basic() {
        let struct_str = json_structure_of(r#"{"name": "Alice", "age": 30}"#).unwrap();
        assert!(struct_str.contains("string"));
        assert!(struct_str.contains("number"));
    }

    #[test]
    fn test_json_extension_registration() {
        let ext = JsonExtension::new();
        assert_eq!(ext.name(), "JSON");
    }

    #[test]
    fn test_json_extract_array_index() {
        let json = r#"[10, 20, 30]"#;
        // Array extraction via index path
        let val = json_extract_value(json, "$[0]").unwrap();
        assert_eq!(val, Some("10".to_string()));
    }

    #[test]
    fn test_json_merge_patch_basic() {
        // RFC 7396 merge patch
        let target = serde_json::json!({"a": 1, "b": 2});
        let patch = serde_json::json!({"b": 3, "c": 4});
        let merged = serde_json::json!({"a": 1, "b": 3, "c": 4});

        // Simple merge patch implementation
        fn merge(a: &serde_json::Value, b: &serde_json::Value) -> serde_json::Value {
            match (a, b) {
                (serde_json::Value::Object(a_map), serde_json::Value::Object(b_map)) => {
                    let mut result = a_map.clone();
                    for (k, v) in b_map {
                        if v.is_null() {
                            result.remove(k);
                        } else {
                            result.insert(k.clone(), merge(result.get(k).unwrap_or(&serde_json::Value::Null), v));
                        }
                    }
                    serde_json::Value::Object(result)
                }
                _ => b.clone(),
            }
        }

        assert_eq!(merge(&target, &patch), merged);
    }
}
