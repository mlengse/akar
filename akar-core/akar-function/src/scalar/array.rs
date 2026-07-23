use crate::registry::*;
use akar_common::types::Value;

// ==================== Array Math Functions ====================

/// Evaluate an array math function: cosine_similarity, distance, inner_product,
/// cross_product, squared_distance.
pub(crate) fn evaluate_array(op: ArrayOp, args: &[Value]) -> Result<Value, String> {
    if args.len() < 2 {
        return Err(format!("Array function {:?} requires 2 arguments", op));
    }

    /// Extract a Vec<f64> from a Value::List or return an error.
    fn extract_f64s(v: &Value) -> Result<Vec<f64>, String> {
        match v {
            Value::List(items) => items
                .iter()
                .map(|item| match item {
                    Value::Int64(i) => Ok(*i as f64),
                    Value::Double(f) => Ok(*f),
                    Value::Float(f) => Ok(*f as f64),
                    _ => Err(format!(
                        "Expected numeric value in array, got {:?}",
                        item.logical_type()
                    )),
                })
                .collect(),
            _ => Err("Expected list/array".into()),
        }
    }

    let a = extract_f64s(&args[0])?;
    let b = extract_f64s(&args[1])?;

    if a.len() != b.len() {
        return Err("Arrays must have the same length".into());
    }

    match op {
        ArrayOp::CosineSimilarity => {
            let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
            let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
            let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
            if norm_a == 0.0 || norm_b == 0.0 {
                return Ok(Value::Double(1.0));
            }
            Ok(Value::Double(dot / (norm_a * norm_b)))
        }
        ArrayOp::Distance => {
            let sum_sq: f64 = a.iter().zip(b.iter()).map(|(x, y)| (x - y) * (x - y)).sum();
            Ok(Value::Double(sum_sq.sqrt()))
        }
        ArrayOp::InnerProduct | ArrayOp::DotProduct => {
            let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
            Ok(Value::Double(dot))
        }
        ArrayOp::CrossProduct => {
            if a.len() != 3 || b.len() != 3 {
                return Err("Cross product requires 3D arrays".into());
            }
            let result = vec![
                Value::Double(a[1] * b[2] - a[2] * b[1]),
                Value::Double(a[2] * b[0] - a[0] * b[2]),
                Value::Double(a[0] * b[1] - a[1] * b[0]),
            ];
            Ok(Value::List(result))
        }
        ArrayOp::SquaredDistance => {
            let sum_sq: f64 = a.iter().zip(b.iter()).map(|(x, y)| (x - y) * (x - y)).sum();
            Ok(Value::Double(sum_sq))
        }
        ArrayOp::Intersect => {
            // Intersect two numeric arrays, preserving only common elements.
            let mut set_b = std::collections::HashSet::new();
            for item in b {
                set_b.insert(item.to_bits());
            }
            let mut result = Vec::new();
            for item in a {
                if set_b.contains(&item.to_bits()) {
                    result.push(Value::Double(item));
                }
            }
            Ok(Value::List(result))
        }
    }
}
