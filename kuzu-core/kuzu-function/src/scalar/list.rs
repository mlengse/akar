use crate::registry::*;
use kuzu_common::types::Value;
use super::{get_string};


// ==================== List ====================

pub(crate) fn evaluate_list(op: ListOp, args: &[Value]) -> Result<Value, String> {
    match op {
        ListOp::Creation => {
            // list_creation just collects all args into a list
            Ok(Value::List(args.to_vec()))
        }
        ListOp::Len => match &args[0] {
            Value::List(items) => Ok(Value::Int64(items.len() as i64)),
            _ => Err("Expected list".into()),
        },
        ListOp::Extract => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let idx = match &args[1] {
                Value::Int64(i) => {
                    // Cypher uses 1-based indexing
                    if *i < 1 {
                        return Err("List index must be >= 1".into());
                    }
                    (*i - 1) as usize
                }
                _ => return Err("Index must be integer".into()),
            };
            list.get(idx)
                .cloned()
                .ok_or_else(|| format!("Index {idx} out of bounds"))
        }
        ListOp::Concat => {
            let mut result = Vec::new();
            for arg in args {
                match arg {
                    Value::List(items) => result.extend(items.clone()),
                    _ => result.push(arg.clone()),
                }
            }
            Ok(Value::List(result))
        }
        ListOp::Sort => {
            let mut list = match args[0].clone() {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            // Sort using Value's PartialOrd implementation (lexicographic)
            list.sort_by(|a, b| {
                match compare_values_for_sort(a, b) {
                    Ok(ord) => ord,
                    Err(_) => std::cmp::Ordering::Equal, // fallback for incomparable types
                }
            });
            Ok(Value::List(list))
        }
        ListOp::Contains => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            Ok(Value::Bool(list.contains(&args[1])))
        }
        ListOp::Append => {
            let mut list = match args[0].clone() {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            list.push(args[1].clone());
            Ok(Value::List(list))
        }
        ListOp::Prepend => {
            let mut list = match args[0].clone() {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            list.insert(0, args[1].clone());
            Ok(Value::List(list))
        }
        ListOp::Reverse => {
            let mut list = match args[0].clone() {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            list.reverse();
            Ok(Value::List(list))
        }
        ListOp::Slice => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let start = match &args[1] {
                Value::Int64(i) => {
                    if *i < 1 {
                        return Err("Slice start index must be >= 1".into());
                    }
                    (*i - 1) as usize
                }
                _ => return Err("Slice start must be integer".into()),
            };
            if start >= list.len() {
                return Err("Slice start index out of bounds".into());
            }
            if args.len() >= 3 {
                // Explicit end (1-based inclusive)
                let end = match &args[2] {
                    Value::Int64(i) => {
                        if *i < 1 {
                            return Err("Slice end index must be >= 1".into());
                        }
                        (*i - 1) as usize
                    }
                    _ => return Err("Slice end must be integer".into()),
                };
                if end >= list.len() || end < start {
                    return Err("Slice end index out of bounds".into());
                }
                Ok(Value::List(list[start..=end].to_vec()))
            } else {
                // No end specified — slice to the end of the list
                Ok(Value::List(list[start..].to_vec()))
            }
        }
        // --- List functions (C++ port) ---
        ListOp::Range => {
            let step = if args.len() >= 3 {
                match &args[2] {
                    Value::Int64(s) => *s,
                    _ => 1i64,
                }
            } else {
                1i64
            };
            let (start, end) = if args.len() >= 2 {
                match (&args[0], &args[1]) {
                    (Value::Int64(s), Value::Int64(e)) => (*s, *e),
                    _ => return Err("RANGE requires integer arguments".into()),
                }
            } else {
                match &args[0] {
                    Value::Int64(e) => (0i64, *e),
                    _ => return Err("RANGE requires integer arguments".into()),
                }
            };
            if step == 0 {
                return Err("Step of range cannot be 0".into());
            }
            if (end - start).signum() != step.signum() && end != start {
                Ok(Value::List(vec![]))
            } else {
                let size = ((end - start).unsigned_abs() / step.unsigned_abs()) + 1;
                let items: Vec<Value> = (0..size).map(|i| Value::Int64(start + step * i as i64)).collect();
                Ok(Value::List(items))
            }
        }
        ListOp::Distinct => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let mut seen = hashbrown::HashSet::new();
            let mut result = Vec::new();
            for item in list {
                if !matches!(item, Value::Null) && seen.insert(format!("{:?}", item)) {
                    result.push(item.clone());
                }
            }
            Ok(Value::List(result))
        }
        ListOp::Unique => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let mut seen = hashbrown::HashSet::new();
            for item in list {
                if !matches!(item, Value::Null) {
                    seen.insert(format!("{:?}", item));
                }
            }
            Ok(Value::Int64(seen.len() as i64))
        }
        ListOp::Sum => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let mut sum: f64 = 0.0;
            let mut is_int = true;
            for item in list {
                match item {
                    Value::Null => continue,
                    Value::Int64(x) => sum += *x as f64,
                    Value::Double(x) => {
                        sum += x;
                        is_int = false;
                    }
                    _ => return Err("LIST_SUM requires numeric list".into()),
                }
            }
            if is_int {
                Ok(Value::Int64(sum as i64))
            } else {
                Ok(Value::Double(sum))
            }
        }
        ListOp::Product => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let mut prod: f64 = 1.0;
            let mut is_int = true;
            for item in list {
                match item {
                    Value::Null => continue,
                    Value::Int64(x) => prod *= *x as f64,
                    Value::Double(x) => {
                        prod *= x;
                        is_int = false;
                    }
                    _ => return Err("LIST_PRODUCT requires numeric list".into()),
                }
            }
            if is_int {
                Ok(Value::Int64(prod as i64))
            } else {
                Ok(Value::Double(prod))
            }
        }
        ListOp::AnyValue => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            // Return first non-null element
            match list.iter().find(|v| !matches!(v, Value::Null)) {
                Some(v) => Ok(v.clone()),
                None => Ok(Value::Null),
            }
        }
        ListOp::ToString => {
            // Parameters: (delimiter: STRING, list: LIST)
            if args.len() < 2 {
                return Err("list_to_string requires delimiter and list arguments".into());
            }
            let delim = get_string(&args[0])?;
            let list = match &args[1] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let mut result = String::new();
            let mut first = true;
            for item in list {
                if matches!(item, Value::Null) {
                    continue;
                }
                if !first {
                    result.push_str(&delim);
                }
                match item {
                    Value::String(s) => result.push_str(s),
                    other => result.push_str(&format!("{:?}", other)),
                }
                first = false;
            }
            Ok(Value::String(result))
        }
        ListOp::Position => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let target = &args[1];
            // 1-based index, returns 0 if not found
            for (i, item) in list.iter().enumerate() {
                if item == target {
                    return Ok(Value::Int64((i + 1) as i64));
                }
            }
            Ok(Value::Int64(0))
        }
        ListOp::HasAll => {
            let left = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let right = match &args[1] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            for target in right {
                if matches!(target, Value::Null) {
                    continue;
                }
                if !left.contains(target) {
                    return Ok(Value::Bool(false));
                }
            }
            Ok(Value::Bool(true))
        }
        ListOp::ReverseSort => {
            let mut list = match args[0].clone() {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            list.sort_by(|a, b| match compare_values_for_sort(a, b) {
                Ok(ord) => ord.reverse(),
                Err(_) => std::cmp::Ordering::Equal,
            });
            Ok(Value::List(list))
        }
        // --- List predicate functions (non-lambda) ---
        ListOp::Any => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            Ok(Value::Bool(list.iter().any(is_truthy)))
        }
        ListOp::All => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            Ok(Value::Bool(!list.is_empty() && list.iter().all(is_truthy)))
        }
        ListOp::None => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            Ok(Value::Bool(list.iter().all(|v| !is_truthy(v))))
        }
        ListOp::Single => {
            let list = match &args[0] {
                Value::List(items) => items,
                _ => return Err("Expected list".into()),
            };
            let count = list.iter().filter(|v| is_truthy(v)).count();
            Ok(Value::Bool(count == 1))
        }
    }
}

/// Check if a Value is "truthy": Bool(true) or non-zero Int64/Double.
fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Int64(x) => *x != 0,
        Value::Double(x) => *x != 0.0,
        _ => false,
    }
}

/// Compare two Values for sorting purposes. Supports numeric, string, bool, date, timestamp.
pub(crate) fn compare_values_for_sort(a: &Value, b: &Value) -> Result<std::cmp::Ordering, String> {
    match (a, b) {
        (Value::Null, Value::Null) => Ok(std::cmp::Ordering::Equal),
        (Value::Null, _) => Ok(std::cmp::Ordering::Less),
        (_, Value::Null) => Ok(std::cmp::Ordering::Greater),
        (Value::Int64(x), Value::Int64(y)) => Ok(x.cmp(y)),
        (Value::Int32(x), Value::Int32(y)) => Ok(x.cmp(y)),
        (Value::Int16(x), Value::Int16(y)) => Ok(x.cmp(y)),
        (Value::Int8(x), Value::Int8(y)) => Ok(x.cmp(y)),
        (Value::UInt64(x), Value::UInt64(y)) => Ok(x.cmp(y)),
        (Value::UInt32(x), Value::UInt32(y)) => Ok(x.cmp(y)),
        (Value::UInt16(x), Value::UInt16(y)) => Ok(x.cmp(y)),
        (Value::UInt8(x), Value::UInt8(y)) => Ok(x.cmp(y)),
        (Value::Double(x), Value::Double(y)) => Ok(x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)),
        (Value::Float(x), Value::Float(y)) => Ok(x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)),
        (Value::String(x), Value::String(y)) => Ok(x.cmp(y)),
        (Value::Bool(x), Value::Bool(y)) => Ok(x.cmp(y)),
        (Value::Date(x), Value::Date(y)) => Ok(x.cmp(y)),
        (Value::Timestamp(x), Value::Timestamp(y)) => Ok(x.cmp(y)),
        // Cross-type numeric promotion
        (Value::Int64(x), Value::Double(y)) => Ok(x
            .partial_cmp(&(*y as i64))
            .map(|o| o.reverse())
            .unwrap_or(std::cmp::Ordering::Equal)),
        (Value::Double(x), Value::Int64(y)) => Ok(x.partial_cmp(&(*y as f64)).unwrap_or(std::cmp::Ordering::Equal)),
        _ => Err("Cannot compare types for sort".into()),
    }
}
