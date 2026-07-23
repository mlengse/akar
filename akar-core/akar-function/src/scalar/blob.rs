use super::get_string;
use crate::registry::*;
use akar_common::types::Value;

// ==================== Blob functions ====================

use base64::prelude::*;

fn encode_base64(data: &[u8]) -> String {
    BASE64_STANDARD.encode(data)
}

fn decode_base64(s: &str) -> Result<Vec<u8>, String> {
    BASE64_STANDARD.decode(s).map_err(|e| format!("Invalid base64: {}", e))
}

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
        BlobOp::BlobFromBytes => match &args[0] {
            Value::Blob(b) => Ok(Value::Blob(b.clone())),
            Value::String(s) => Ok(Value::Blob(s.clone().into_bytes())),
            _ => Err("blob_from_bytes requires string or blob".into()),
        },
        BlobOp::ToBase64 => {
            let bytes = match &args[0] {
                Value::Blob(b) => b,
                _ => return Err("to_base64 requires a blob argument".into()),
            };
            Ok(Value::String(encode_base64(bytes)))
        }
        BlobOp::FromBase64 => {
            let s = match &args[0] {
                Value::String(s) => s,
                _ => return Err("from_base64 requires a string argument".into()),
            };
            let decoded = decode_base64(s)?;
            Ok(Value::Blob(decoded))
        }
    }
}
