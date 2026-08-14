//! Parameter interpolation (P53.7).
//!
//! Kairos passes query parameters as Python dicts (workaround P51.31). The
//! native prepared-statement path cannot substitute `LIMIT $n` / `ORDER BY` /
//! property-pattern parameters, so the binding interpolates `$name` markers
//! into Cypher literals on the *translated* query before execution.
//!
//! Literal forms follow Akar's grammar (`cypher.pest`):
//! - strings   → `'...'` with `\`-escaping (doubling is NOT valid in Akar)
//! - floats    → always keep a `.0` (grammar float requires a dot)
//! - booleans  → `TRUE` / `FALSE`
//! - lists     → `[...]`   (elements are expressions)
//! - dicts     → `{k: v}`  (keys must be identifiers, backticked if needed)
//! - null      → `NULL`
//!
//! `$` markers inside string literals are left untouched.

use std::collections::HashMap;

use akar_common::types::Value;

/// Replace every `$name` marker (outside string literals) with its literal
/// value. Errors on missing parameters and on values that cannot be expressed
/// as a Cypher literal.
pub fn interpolate(query: &str, params: &HashMap<String, Value>) -> Result<String, String> {
    if params.is_empty() {
        return Ok(query.to_string());
    }

    let mut out = String::with_capacity(query.len() + 16);
    let bytes = query.as_bytes();
    let mut i = 0usize;
    let mut in_single = false;
    let mut in_double = false;

    while i < bytes.len() {
        let ch = bytes[i] as char;
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '$' if !in_single && !in_double => {
                let next = bytes.get(i + 1).copied();
                if let Some(b) = next {
                    if b.is_ascii_alphabetic() || b == b'_' {
                        // read `$` + identifier
                        let start = i + 1;
                        let mut end = start;
                        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                            end += 1;
                        }
                        let name = &query[start..end];
                        let value = lookup(params, name)
                            .ok_or_else(|| format!("Missing parameter ${name} — provide it via params= dict"))?;
                        out.push_str(&escape_value(value)?);
                        i = end;
                        continue;
                    }
                }
                out.push('$');
            }
            _ => {}
        }
        out.push(ch);
        i += 1;
    }
    Ok(out)
}

fn lookup<'a>(params: &'a HashMap<String, Value>, name: &str) -> Option<&'a Value> {
    if let Some(v) = params.get(name) {
        return Some(v);
    }
    // Case-insensitive fallback: Kairos is consistent, but the DB layer is
    // not always.
    params
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v)
}

/// Render a [`Value`] as a Cypher literal usable in an expression position.
pub fn escape_value(value: &Value) -> Result<String, String> {
    match value {
        Value::Null => Ok("NULL".to_string()),
        Value::Bool(b) => Ok(if *b { "TRUE" } else { "FALSE" }.to_string()),
        Value::Int64(v) => Ok(v.to_string()),
        Value::Int32(v) => Ok(v.to_string()),
        Value::Int16(v) => Ok(v.to_string()),
        Value::Int8(v) => Ok(v.to_string()),
        Value::UInt64(v) => Ok(v.to_string()),
        Value::UInt32(v) => Ok(v.to_string()),
        Value::UInt16(v) => Ok(v.to_string()),
        Value::UInt8(v) => Ok(v.to_string()),
        Value::Int128(v) => Ok(v.to_string()),
        Value::UInt128(v) => Ok(v.to_string()),
        Value::Double(d) => Ok(float_literal(*d)),
        Value::Float(f) => Ok(float_literal(*f as f64)),
        Value::String(s) => Ok(escape_string(s)),
        Value::List(items) => {
            let mut parts = Vec::with_capacity(items.len());
            for item in items {
                parts.push(escape_value(item)?);
            }
            Ok(format!("[{}]", parts.join(", ")))
        }
        Value::Struct(fields) => escape_map(fields.iter().map(|(k, v)| (k.as_str(), v))),
        Value::Map(kvs) => {
            let pairs = kvs.iter().map(|(k, v)| match k {
                Value::String(s) => Ok((s.as_str(), v)),
                other => Err(format!("Cannot interpolate non-string map key: {other:?}")),
            });
            let pairs: Result<Vec<(&str, &Value)>, String> = pairs.collect();
            escape_map(pairs?.into_iter())
        }
        other => Err(format!(
            "Cannot interpolate parameter of type {:?} into a Cypher literal",
            variant_name(other)
        )),
    }
}

fn escape_map<'a, I>(pairs: I) -> Result<String, String>
where
    I: IntoIterator<Item = (&'a str, &'a Value)>,
{
    let mut parts = Vec::new();
    for (k, v) in pairs {
        parts.push(format!("{}: {}", escape_key(k), escape_value(v)?));
    }
    Ok(format!("{{{}}}", parts.join(", ")))
}

/// Identifiers only accept `[A-Za-z_][A-Za-z0-9_]*`; anything else (e.g.
/// dotted dict keys) is backtick-quoted.
fn escape_key(k: &str) -> String {
    let valid = {
        let mut chars = k.chars();
        matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    if valid {
        k.to_string()
    } else {
        format!("`{}`", k.replace('`', "``"))
    }
}

fn escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        match ch {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            _ => out.push(ch),
        }
    }
    out.push('\'');
    out
}

/// Akar's `float` grammar requires a decimal point.
fn float_literal(f: f64) -> String {
    let s = format!("{f}");
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

fn variant_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Int64(_) => "int64",
        Value::Int32(_) => "int32",
        Value::Int16(_) => "int16",
        Value::Int8(_) => "int8",
        Value::UInt64(_) => "uint64",
        Value::UInt32(_) => "uint32",
        Value::UInt16(_) => "uint16",
        Value::UInt8(_) => "uint8",
        Value::Int128(_) => "int128",
        Value::UInt128(_) => "uint128",
        Value::Double(_) => "double",
        Value::Float(_) => "float",
        Value::String(_) => "string",
        Value::Blob(_) => "blob",
        Value::Date(_) => "date",
        Value::Timestamp(_)
        | Value::TimestampTz(_)
        | Value::TimestampNs(_)
        | Value::TimestampMs(_)
        | Value::TimestampSec(_) => "timestamp",
        Value::Interval(_) => "interval",
        Value::InternalID(_) => "internal_id",
        Value::Json(_) => "json",
        Value::DTime(_) => "dtime",
        Value::Union(_, _) => "union",
        Value::List(_) => "list",
        Value::Map(_) => "map",
        Value::Struct(_) => "struct",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn no_params_returns_unchanged() {
        let q = "MATCH (m) WHERE m.id = $id RETURN m";
        assert_eq!(interpolate(q, &HashMap::new()).unwrap(), q);
    }

    #[test]
    fn interpolates_scalars() {
        let p = params(&[
            ("n", Value::Int64(3)),
            ("f", Value::Double(2.5)),
            ("b", Value::Bool(true)),
            ("s", Value::String("it's a \\test".into())),
            ("z", Value::Null),
        ]);
        let q = "MATCH (m) WHERE m.x = $n AND m.y > $f AND m.ok = $b AND m.name = $s AND m.z = $z RETURN m";
        let out = interpolate(q, &p).unwrap();
        assert_eq!(
            out,
            "MATCH (m) WHERE m.x = 3 AND m.y > 2.5 AND m.ok = TRUE AND m.name = 'it\\'s a \\\\test' AND m.z = NULL RETURN m"
        );
    }

    #[test]
    fn integral_floats_get_decimal_point() {
        let p = params(&[("f", Value::Double(1.0)), ("g", Value::Float(2.0))]);
        let out = interpolate("RETURN $f + $g", &p).unwrap();
        assert_eq!(out, "RETURN 1.0 + 2.0");
    }

    #[test]
    fn interpolates_list_and_dict() {
        let p = params(&[
            ("v", Value::List(vec![Value::Double(0.1), Value::Double(0.9)])),
            (
                "m",
                Value::Struct(vec![
                    ("foo".into(), Value::Int64(1)),
                    ("bar".into(), Value::String("x".into())),
                ]),
            ),
        ]);
        let out = interpolate("MATCH (n) WHERE n.vec = $v AND n.meta = $m RETURN n", &p).unwrap();
        assert_eq!(
            out,
            "MATCH (n) WHERE n.vec = [0.1, 0.9] AND n.meta = {foo: 1, bar: 'x'} RETURN n"
        );
    }

    #[test]
    fn dollar_inside_string_is_untouched() {
        let p = params(&[("n", Value::Int64(1))]);
        let q = "MATCH (m) WHERE m.p = '$n' AND m.q = $n RETURN m";
        let out = interpolate(q, &p).unwrap();
        assert_eq!(out, "MATCH (m) WHERE m.p = '$n' AND m.q = 1 RETURN m");
    }

    #[test]
    fn missing_param_errors() {
        let p = params(&[("a", Value::Int64(1))]);
        let err = interpolate("RETURN $b", &p).unwrap_err();
        assert!(err.contains("$b"), "{err}");
    }

    #[test]
    fn unsupported_type_errors() {
        let p = params(&[(
            "x",
            Value::InternalID(akar_common::types::InternalID { table_id: 1, offset: 2 }),
        )]);
        assert!(interpolate("RETURN $x", &p).is_err());
    }

    #[test]
    fn escape_value_matches_grammar() {
        assert_eq!(escape_value(&Value::Bool(false)).unwrap(), "FALSE");
        assert_eq!(escape_value(&Value::String("a".into())).unwrap(), "'a'");
        assert_eq!(escape_value(&Value::List(vec![])).unwrap(), "[]");
        assert_eq!(escape_value(&Value::Null).unwrap(), "NULL");
    }
}
