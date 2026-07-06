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
    }
}
