use crate::registry::*;
use kuzu_common::types::Value;

// ==================== Boolean ====================

pub(crate) fn evaluate_boolean(op: BooleanOp, args: &[Value]) -> Result<Value, String> {
    if args.len() < 2 && !matches!(op, BooleanOp::Not) {
        return Err("Boolean op requires 2 arguments".into());
    }
    let a = match &args[0] {
        Value::Bool(b) => *b,
        _ => return Err("Expected boolean".into()),
    };
    match op {
        BooleanOp::And => {
            let b = match &args[1] {
                Value::Bool(x) => *x,
                _ => return Err("Expected boolean".into()),
            };
            Ok(Value::Bool(a && b))
        }
        BooleanOp::Or => {
            let b = match &args[1] {
                Value::Bool(x) => *x,
                _ => return Err("Expected boolean".into()),
            };
            Ok(Value::Bool(a || b))
        }
        BooleanOp::Xor => {
            let b = match &args[1] {
                Value::Bool(x) => *x,
                _ => return Err("Expected boolean".into()),
            };
            Ok(Value::Bool(a ^ b))
        }
        BooleanOp::Not => Ok(Value::Bool(!a)),
    }
}
