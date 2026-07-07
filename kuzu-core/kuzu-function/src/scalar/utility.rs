use crate::registry::*;
use kuzu_common::types::Value;

// ==================== Utility ====================

pub(crate) fn evaluate_utility(op: UtilityOp, args: &[Value]) -> Result<Value, String> {
    match op {
        UtilityOp::Coalesce | UtilityOp::IfNull => {
            for arg in args {
                if !matches!(arg, Value::Null) {
                    return Ok(arg.clone());
                }
            }
            Ok(Value::Null)
        }
        UtilityOp::TypeOf => {
            if args.is_empty() {
                return Ok(Value::String("NULL".into()));
            }
            Ok(Value::String(format!("{:?}", args[0].logical_type())))
        }
        UtilityOp::NullIf => {
            if args.len() < 2 {
                return Err("NULLIF requires 2 arguments".into());
            }
            if args[0] == args[1] {
                Ok(Value::Null)
            } else {
                Ok(args[0].clone())
            }
        }
        UtilityOp::Size => {
            if args.is_empty() {
                return Err("SIZE requires 1 argument".into());
            }
            match &args[0] {
                Value::List(v) => Ok(Value::Int64(v.len() as i64)),
                Value::String(s) => Ok(Value::Int64(s.len() as i64)),
                Value::Map(v) => Ok(Value::Int64(v.len() as i64)),
                Value::Null => Ok(Value::Null),
                _ => Err(format!("SIZE does not support type {:?}", args[0].logical_type())),
            }
        }
    }
}
