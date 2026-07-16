use crate::registry::*;
use kuzu_common::types::Value;
use super::{get_string};


// ==================== Blob functions ====================

const BASE64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode_base64(data: &[u8]) -> String {
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        
        result.push(BASE64_CHARS[(n >> 18) as usize & 63] as char);
        result.push(BASE64_CHARS[(n >> 12) as usize & 63] as char);
        result.push(if chunk.len() > 1 { BASE64_CHARS[(n >> 6) as usize & 63] as char } else { '=' });
        result.push(if chunk.len() > 2 { BASE64_CHARS[n as usize & 63] as char } else { '=' });
    }
    result
}

fn decode_base64(s: &str) -> Result<Vec<u8>, String> {
    let mut result = Vec::with_capacity(s.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;
    
    for c in s.chars() {
        if c == '=' { break; }
        if c.is_whitespace() { continue; }
        
        let val = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            _ => return Err(format!("Invalid base64 character: {}", c)),
        };
        
        buffer = (buffer << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            result.push((buffer >> bits) as u8);
        }
    }
    Ok(result)
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
        BlobOp::BlobFromBytes => {
            match &args[0] {
                Value::Blob(b) => Ok(Value::Blob(b.clone())),
                Value::String(s) => Ok(Value::Blob(s.clone().into_bytes())),
                _ => Err("blob_from_bytes requires string or blob".into()),
            }
        }
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
