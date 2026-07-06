use crate::registry::*;
use kuzu_common::types::Value;
use super::{get_string};


// ==================== Blob functions ====================

/// Evaluate a blob function.
pub(crate) fn evaluate_blob(op: BlobOp, args: &[Value]) -> Result<Value, String> {
    match op {
        BlobOp::Encode => {
            let s = get_string(&args[0])?;
            Ok(Value::Blob(s.into_bytes()))
        }
        BlobOp::Decode => {
            let bytes = match &args[0] {
                Value::Blob(b) => b.clone(),
                _ => return Err("DECODE requires a blob argument".into()),
            };
            let s = String::from_utf8(bytes).map_err(|_| {
                "Failure in decode: could not convert blob to UTF8 string, the blob contained invalid UTF8 characters".to_string()
            })?;
            Ok(Value::String(s))
        }
        BlobOp::OctetLength => {
            let len = match &args[0] {
                Value::Blob(b) => b.len() as i64,
                _ => return Err("OCTET_LENGTH requires a blob argument".into()),
            };
            Ok(Value::Int64(len))
        }
    }
}
