//! Minimal Avro object-container reader for Iceberg manifest files.
//!
//! Iceberg manifest lists (`*manifest-list.avro`) and manifests
//! (`*.avro`) are Avro object-container files. Rather than pulling in a full
//! Avro library, this module implements the small subset needed to decode the
//! records a snapshot references:
//!
//! - the object-container framing (magic, metadata map, sync markers), with
//!   `null` and `deflate` codecs (what Iceberg writes), and
//! - a generic schema-driven binary decoder, so both the v1 and v2 manifest
//!   schemas (and any partition spec) are handled from the schema embedded in
//!   the file header.
//!
//! Consumers extract the fields they need by name (`manifest_path`, `status`,
//! `data_file`, `file_path`); unknown or extra fields are decoded and ignored.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;

/// A parsed Avro schema (the subset used by Iceberg manifests).
#[derive(Debug, Clone)]
enum Schema {
    Null,
    Boolean,
    Int,
    Long,
    Float,
    Double,
    Bytes,
    String,
    Record { fields: Vec<(String, Schema)> },
    Enum,
    Array(Box<Schema>),
    Map(Box<Schema>),
    Union(Vec<Schema>),
    Fixed { size: usize },
}

/// A decoded Avro value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Boolean(bool),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Bytes(Vec<u8>),
    String(String),
    Record(Vec<(String, Value)>),
    Array(Vec<Value>),
    Map(Vec<(String, Value)>),
}

impl Value {
    /// Look up a named field on a record value.
    pub fn field<'a>(&'a self, name: &str) -> Option<&'a Value> {
        match self {
            Value::Record(fields) => fields.iter().find(|(k, _)| k == name).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i as i64),
            Value::Long(l) => Some(*l),
            _ => None,
        }
    }
}

/// Read and decode all records from an Avro object-container file.
pub fn read_avro_file(path: &Path) -> Result<Vec<Value>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Failed to read Avro file {}: {e}", path.display()))?;
    read_avro_bytes(&bytes)
}

/// Read and decode all records from an in-memory Avro object container.
fn read_avro_bytes(bytes: &[u8]) -> Result<Vec<Value>, String> {
    let mut cur = Cursor::new(bytes);

    let mut magic = [0u8; 4];
    cur.read_exact(&mut magic)
        .map_err(|e| format!("Truncated Avro header: {e}"))?;
    if &magic != b"Obj\x01" {
        return Err("Not an Avro object container file (bad magic)".into());
    }

    let metadata = read_map(&mut cur)?;
    let schema_json = metadata
        .get("avro.schema")
        .and_then(|b| std::str::from_utf8(b).ok())
        .ok_or_else(|| "Avro file missing avro.schema metadata".to_string())?;
    let codec = metadata
        .get("avro.codec")
        .and_then(|b| std::str::from_utf8(b).ok())
        .unwrap_or("null")
        .to_string();

    let schema = parse_schema(schema_json)?;

    let mut sync = [0u8; 16];
    cur.read_exact(&mut sync)
        .map_err(|e| format!("Truncated Avro header (sync marker): {e}"))?;

    let mut records = Vec::new();
    loop {
        let count = match read_long(&mut cur) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("Failed to read Avro block count: {e}")),
        };
        let size = read_long(&mut cur).map_err(|e| format!("Failed to read Avro block size: {e}"))?;
        let mut block = vec![0u8; size as usize];
        cur.read_exact(&mut block)
            .map_err(|e| format!("Failed to read Avro block body: {e}"))?;

        let mut block_sync = [0u8; 16];
        cur.read_exact(&mut block_sync)
            .map_err(|e| format!("Failed to read Avro block sync marker: {e}"))?;
        if block_sync != sync {
            return Err("Avro sync marker mismatch (corrupt or concatenated file)".into());
        }

        let data = match codec.as_str() {
            "null" => block,
            "deflate" => {
                let mut decoder = flate2::read::DeflateDecoder::new(&block[..]);
                let mut out = Vec::new();
                decoder
                    .read_to_end(&mut out)
                    .map_err(|e| format!("Failed to inflate Avro deflate block: {e}"))?;
                out
            }
            other => return Err(format!("Unsupported Avro codec: {other}")),
        };

        let mut r = Cursor::new(data);
        for _ in 0..count {
            let value = decode(&mut r, &schema).map_err(|e| format!("Failed to decode Avro record: {e}"))?;
            records.push(value);
        }
    }

    Ok(records)
}

// ---------------------------------------------------------------------------
// Schema parsing
// ---------------------------------------------------------------------------

fn parse_schema(json: &str) -> Result<Schema, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("Invalid Avro schema JSON: {e}"))?;
    parse_schema_value(&value)
}

fn parse_schema_value(v: &serde_json::Value) -> Result<Schema, String> {
    if let Some(prim) = v.as_str() {
        return match prim {
            "null" => Ok(Schema::Null),
            "boolean" => Ok(Schema::Boolean),
            "int" => Ok(Schema::Int),
            "long" => Ok(Schema::Long),
            "float" => Ok(Schema::Float),
            "double" => Ok(Schema::Double),
            "bytes" => Ok(Schema::Bytes),
            "string" => Ok(Schema::String),
            other => Err(format!("Unsupported Avro named type: {other}")),
        };
    }
    if let Some(arr) = v.as_array() {
        // Union: [null, "string", ...]
        let variants = arr.iter().map(parse_schema_value).collect::<Result<Vec<_>, _>>()?;
        return Ok(Schema::Union(variants));
    }
    let obj = v.as_object().ok_or_else(|| "Invalid Avro schema".to_string())?;
    match obj.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "record" => {
            let mut fields = Vec::new();
            if let Some(field_arr) = obj.get("fields").and_then(|f| f.as_array()) {
                for f in field_arr {
                    let name = f
                        .get("name")
                        .and_then(|n| n.as_str())
                        .ok_or_else(|| "Avro record field missing name".to_string())?;
                    let field_type = f
                        .get("type")
                        .ok_or_else(|| format!("Avro record field {name} missing type"))?;
                    fields.push((name.to_string(), parse_schema_value(field_type)?));
                }
            }
            Ok(Schema::Record { fields })
        }
        "array" => {
            let items = obj.get("items").ok_or_else(|| "Avro array missing items".to_string())?;
            Ok(Schema::Array(Box::new(parse_schema_value(items)?)))
        }
        "map" => {
            let values = obj.get("values").ok_or_else(|| "Avro map missing values".to_string())?;
            Ok(Schema::Map(Box::new(parse_schema_value(values)?)))
        }
        "enum" => Ok(Schema::Enum),
        "fixed" => {
            let size = obj
                .get("size")
                .and_then(|s| s.as_i64())
                .ok_or_else(|| "Avro fixed missing size".to_string())?;
            Ok(Schema::Fixed { size: size as usize })
        }
        other => Err(format!("Unsupported Avro schema type: {other}")),
    }
}

// ---------------------------------------------------------------------------
// Binary decoding
// ---------------------------------------------------------------------------

fn read_long<R: Read>(r: &mut R) -> std::io::Result<i64> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    loop {
        let mut b = [0u8; 1];
        r.read_exact(&mut b)?;
        let byte = b[0];
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift > 63 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Avro varint too long",
            ));
        }
    }
    // Zig-zag decode.
    Ok(((result >> 1) as i64) ^ -((result & 1) as i64))
}

fn read_string<R: Read>(r: &mut R) -> std::io::Result<String> {
    let len = read_long(r)?;
    if len < 0 || len > 64 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Avro string length out of range",
        ));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    String::from_utf8(buf)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "Avro string is not valid UTF-8"))
}

fn read_bytes<R: Read>(r: &mut R) -> std::io::Result<Vec<u8>> {
    let len = read_long(r)?;
    if len < 0 || len > 256 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Avro bytes length out of range",
        ));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

/// Read a block of array items or map entries (with the optional byte-size
/// header Avro allows on negative block counts).
fn read_block<R: Read>(r: &mut R) -> std::io::Result<i64> {
    let mut count = read_long(r)?;
    if count < 0 {
        // Negative count: absolute value is the count; a byte size follows.
        let _size = read_long(r)?;
        count = -count;
    }
    Ok(count)
}

fn decode<R: Read>(r: &mut R, schema: &Schema) -> Result<Value, String> {
    match schema {
        Schema::Null => Ok(Value::Null),
        Schema::Boolean => {
            let mut b = [0u8; 1];
            r.read_exact(&mut b).map_err(|e| e.to_string())?;
            Ok(Value::Boolean(b[0] != 0))
        }
        Schema::Int => {
            let v = read_long(r).map_err(|e| e.to_string())?;
            Ok(Value::Int(v as i32))
        }
        Schema::Long => {
            let v = read_long(r).map_err(|e| e.to_string())?;
            Ok(Value::Long(v))
        }
        Schema::Float => {
            let mut b = [0u8; 4];
            r.read_exact(&mut b).map_err(|e| e.to_string())?;
            Ok(Value::Float(f32::from_le_bytes(b)))
        }
        Schema::Double => {
            let mut b = [0u8; 8];
            r.read_exact(&mut b).map_err(|e| e.to_string())?;
            Ok(Value::Double(f64::from_le_bytes(b)))
        }
        Schema::Bytes => read_bytes(r).map(Value::Bytes).map_err(|e| e.to_string()),
        Schema::String => read_string(r).map(Value::String).map_err(|e| e.to_string()),
        Schema::Record { fields } => {
            let mut out = Vec::with_capacity(fields.len());
            for (name, field_schema) in fields {
                let v = decode(r, field_schema)?;
                out.push((name.clone(), v));
            }
            Ok(Value::Record(out))
        }
        Schema::Enum => {
            let idx = read_long(r).map_err(|e| e.to_string())?;
            Ok(Value::Int(idx as i32))
        }
        Schema::Array(items) => {
            let mut out = Vec::new();
            loop {
                let count = read_block(r).map_err(|e| e.to_string())?;
                if count == 0 {
                    break;
                }
                for _ in 0..count {
                    out.push(decode(r, items)?);
                }
            }
            Ok(Value::Array(out))
        }
        Schema::Map(values) => {
            let mut out = Vec::new();
            loop {
                let count = read_block(r).map_err(|e| e.to_string())?;
                if count == 0 {
                    break;
                }
                for _ in 0..count {
                    let key = read_string(r).map_err(|e| e.to_string())?;
                    let v = decode(r, values)?;
                    out.push((key, v));
                }
            }
            Ok(Value::Map(out))
        }
        Schema::Union(variants) => {
            let idx = read_long(r).map_err(|e| e.to_string())?;
            let variant = variants
                .get(idx as usize)
                .ok_or_else(|| format!("Avro union index {idx} out of range"))?;
            decode(r, variant)
        }
        Schema::Fixed { size } => {
            let mut buf = vec![0u8; *size];
            r.read_exact(&mut buf).map_err(|e| e.to_string())?;
            Ok(Value::Bytes(buf))
        }
    }
}

/// Read the file-metadata map at the head of an Avro container.
fn read_map<R: Read>(r: &mut R) -> Result<HashMap<String, Vec<u8>>, String> {
    let mut out = HashMap::new();
    loop {
        let count = read_block(r).map_err(|e| format!("Failed to read Avro metadata map: {e}"))?;
        if count == 0 {
            break;
        }
        for _ in 0..count {
            let key = read_string(r).map_err(|e| e.to_string())?;
            let value = read_bytes(r).map_err(|e| e.to_string())?;
            out.insert(key, value);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;

    fn zigzag(i: i64) -> Vec<u8> {
        let mut v = ((i << 1) ^ (i >> 63)) as u64;
        let mut out = Vec::new();
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            out.push(if v != 0 { b | 0x80 } else { b });
            if v == 0 {
                break;
            }
        }
        out
    }

    fn w_string(out: &mut Vec<u8>, s: &str) {
        out.extend(zigzag(s.len() as i64));
        out.extend_from_slice(s.as_bytes());
    }

    fn w_bytes(out: &mut Vec<u8>, b: &[u8]) {
        out.extend(zigzag(b.len() as i64));
        out.extend_from_slice(b);
    }

    fn encode_value(out: &mut Vec<u8>, v: &Value, schema: &Schema) {
        match (schema, v) {
            (Schema::Null, _) => {}
            (Schema::Boolean, Value::Boolean(b)) => out.push(if *b { 1 } else { 0 }),
            (Schema::Int, Value::Int(i)) => out.extend(zigzag(*i as i64)),
            (Schema::Long, Value::Long(l)) => out.extend(zigzag(*l)),
            (Schema::Float, Value::Float(f)) => out.extend(f.to_le_bytes()),
            (Schema::Double, Value::Double(d)) => out.extend(d.to_le_bytes()),
            (Schema::Bytes, Value::Bytes(b)) => w_bytes(out, b),
            (Schema::String, Value::String(s)) => w_string(out, s),
            (Schema::Record { fields }, Value::Record(vals)) => {
                for (name, field_schema) in fields {
                    let fv = vals.iter().find(|(k, _)| k == name).unwrap().1;
                    encode_value(out, fv, field_schema);
                }
            }
            (Schema::Array(items), Value::Array(vals)) => {
                out.extend(zigzag(vals.len() as i64));
                for item in vals {
                    encode_value(out, item, items);
                }
                out.push(0);
            }
            (Schema::Map(values), Value::Map(entries)) => {
                out.extend(zigzag(entries.len() as i64));
                for (key, val) in entries {
                    w_string(out, key);
                    encode_value(out, val, values);
                }
                out.push(0);
            }
            (Schema::Union(variants), v) => {
                let idx = variants
                    .iter()
                    .position(|s| {
                        matches!(
                            (s, v),
                            (Schema::Null, Value::Null)
                                | (Schema::Boolean, Value::Boolean(_))
                                | (Schema::Int, Value::Int(_))
                                | (Schema::Long, Value::Long(_))
                                | (Schema::Float, Value::Float(_))
                                | (Schema::Double, Value::Double(_))
                                | (Schema::Bytes, Value::Bytes(_))
                                | (Schema::String, Value::String(_))
                        )
                    })
                    .expect("no matching union variant for test value");
                out.extend(zigzag(idx as i64));
                if !matches!(variants[idx], Schema::Null) {
                    encode_value(out, v, &variants[idx]);
                }
            }
            (Schema::Enum, Value::Int(i)) => out.extend(zigzag(*i as i64)),
            (Schema::Fixed { size }, Value::Bytes(b)) => {
                assert_eq!(b.len(), *size, "fixed value length mismatch");
                out.extend_from_slice(b);
            }
            _ => panic!("encode_value: unsupported (schema, value) pair"),
        }
    }

    fn write_container(path: &Path, schema_json: &str, codec: &str, records: &[(Schema, Value)]) {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"Obj\x01");
        // File metadata map: avro.schema + avro.codec.
        buf.extend(zigzag(2));
        w_string(&mut buf, "avro.schema");
        w_bytes(&mut buf, schema_json.as_bytes());
        w_string(&mut buf, "avro.codec");
        w_bytes(&mut buf, codec.as_bytes());
        buf.push(0);

        let sync = [0u8; 16];
        buf.extend_from_slice(&sync);

        let mut body = Vec::new();
        for (schema, rec) in records {
            encode_value(&mut body, rec, schema);
        }
        let block: Vec<u8> = match codec {
            "null" => body,
            "deflate" => {
                let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                enc.write_all(&body).unwrap();
                enc.finish().unwrap()
            }
            _ => panic!("test writer supports only null/deflate codecs"),
        };

        buf.extend(zigzag(records.len() as i64));
        buf.extend(zigzag(block.len() as i64));
        buf.extend_from_slice(&block);
        buf.extend_from_slice(&sync);
        fs::write(path, buf).unwrap();
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_read_avro_roundtrip_null_codec() {
        let dir = temp_dir("avro_roundtrip_null");
        let path = dir.join("sample.avro");
        let schema_json = r#"{
            "type": "record", "name": "sample", "fields": [
                {"name": "name", "type": "string"},
                {"name": "age", "type": ["null", "long"]},
                {"name": "active", "type": "boolean"}
            ]
        }"#;
        let schema = parse_schema(schema_json).unwrap();
        let records = vec![
            (
                schema.clone(),
                Value::Record(vec![
                    ("name".into(), Value::String("alice".into())),
                    ("age".into(), Value::Long(30)),
                    ("active".into(), Value::Boolean(true)),
                ]),
            ),
            (
                schema.clone(),
                Value::Record(vec![
                    ("name".into(), Value::String("bob".into())),
                    ("age".into(), Value::Null),
                    ("active".into(), Value::Boolean(false)),
                ]),
            ),
        ];
        write_container(&path, schema_json, "null", &records);

        let decoded = read_avro_file(&path).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].field("name").unwrap().as_str(), Some("alice"));
        assert_eq!(decoded[0].field("age").unwrap().as_i64(), Some(30));
        assert_eq!(decoded[0].field("active").unwrap(), &Value::Boolean(true));
        assert_eq!(decoded[1].field("age").unwrap(), &Value::Null);
    }

    #[test]
    fn test_read_avro_roundtrip_deflate_codec() {
        let dir = temp_dir("avro_roundtrip_deflate");
        let path = dir.join("sample.avro");
        let schema_json = r#"{"type":"record","name":"s","fields":[{"name":"v","type":"string"}]}"#;
        let schema = parse_schema(schema_json).unwrap();
        let records = vec![(
            schema,
            Value::Record(vec![("v".into(), Value::String("hello world".into()))]),
        )];
        write_container(&path, schema_json, "deflate", &records);

        let decoded = read_avro_file(&path).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].field("v").unwrap().as_str(), Some("hello world"));
    }

    #[test]
    fn test_read_avro_rejects_bad_magic() {
        let dir = temp_dir("avro_bad_magic");
        let path = dir.join("bad.avro");
        fs::write(&path, b"NOTAVROFILE").unwrap();
        assert!(read_avro_file(&path).is_err());
    }

    #[test]
    fn test_read_avro_array_and_map() {
        let dir = temp_dir("avro_array_map");
        let path = dir.join("a.avro");
        let schema_json = r#"{"type":"record","name":"s","fields":[
            {"name":"tags","type":{"type":"array","items":"string"}},
            {"name":"meta","type":{"type":"map","values":"long"}}
        ]}"#;
        let schema = parse_schema(schema_json).unwrap();
        let records = vec![(
            schema,
            Value::Record(vec![
                (
                    "tags".into(),
                    Value::Array(vec![Value::String("a".into()), Value::String("b".into())]),
                ),
                ("meta".into(), Value::Map(vec![("k".into(), Value::Long(1))])),
            ]),
        )];
        write_container(&path, schema_json, "null", &records);

        let decoded = read_avro_file(&path).unwrap();
        assert_eq!(
            decoded[0].field("tags").unwrap(),
            &Value::Array(vec![Value::String("a".into()), Value::String("b".into())])
        );
        assert_eq!(
            decoded[0].field("meta").unwrap(),
            &Value::Map(vec![("k".into(), Value::Long(1))])
        );
    }

    #[test]
    fn test_iceberg_active_data_files_via_manifests() {
        let table = temp_dir("iceberg_manifest_filter");
        let meta_dir = table.join("metadata");
        fs::create_dir_all(&meta_dir).unwrap();

        let manifest_list_schema = r#"{
            "type": "record", "name": "manifest_file", "fields": [
                {"name": "manifest_path", "type": "string"},
                {"name": "manifest_length", "type": "long"},
                {"name": "partition_spec_id", "type": "int"},
                {"name": "content", "type": "int"},
                {"name": "sequence_number", "type": "long"},
                {"name": "min_sequence_number", "type": "long"},
                {"name": "added_snapshot_id", "type": "long"},
                {"name": "added_files_count", "type": "int"},
                {"name": "existing_files_count", "type": "int"},
                {"name": "deleted_files_count", "type": "int"},
                {"name": "added_rows_count", "type": "long"},
                {"name": "existing_rows_count", "type": "long"},
                {"name": "deleted_rows_count", "type": "long"},
                {"name": "partitions", "type": ["null", {"type": "array", "items": ["null", {"type": "record", "name": "r508", "fields": [{"name": "contains_null", "type": "boolean"}, {"name": "lower_bound", "type": ["null", "bytes"]}, {"name": "upper_bound", "type": ["null", "bytes"]}]}]}]},
                {"name": "key_metadata", "type": ["null", "bytes"]}
            ]
        }"#;

        let manifest_schema = r#"{
            "type": "record", "name": "manifest_entry", "fields": [
                {"name": "status", "type": "int"},
                {"name": "snapshot_id", "type": ["null", "long"]},
                {"name": "sequence_number", "type": ["null", "long"]},
                {"name": "file_sequence_number", "type": ["null", "long"]},
                {"name": "data_file", "type": {"type": "record", "name": "data_file", "fields": [
                    {"name": "content", "type": "int"},
                    {"name": "file_path", "type": "string"},
                    {"name": "file_format", "type": "string"},
                    {"name": "partition", "type": {"type": "record", "name": "partition_data", "fields": []}},
                    {"name": "record_count", "type": "long"},
                    {"name": "file_size_in_bytes", "type": "long"},
                    {"name": "column_sizes", "type": ["null", {"type": "array", "items": ["null", {"type": "record", "name": "c1", "fields": [{"name": "key", "type": "int"}, {"name": "value", "type": "long"}]}]}]},
                    {"name": "value_counts", "type": ["null", {"type": "array", "items": ["null", {"type": "record", "name": "c2", "fields": [{"name": "key", "type": "int"}, {"name": "value", "type": "long"}]}]}]},
                    {"name": "null_value_counts", "type": ["null", {"type": "array", "items": ["null", {"type": "record", "name": "c3", "fields": [{"name": "key", "type": "int"}, {"name": "value", "type": "long"}]}]}]},
                    {"name": "nan_value_counts", "type": ["null", {"type": "array", "items": ["null", {"type": "record", "name": "c4", "fields": [{"name": "key", "type": "int"}, {"name": "value", "type": "long"}]}]}]},
                    {"name": "lower_bounds", "type": ["null", {"type": "array", "items": ["null", {"type": "record", "name": "c5", "fields": [{"name": "key", "type": "int"}, {"name": "value", "type": "bytes"}]}]}]},
                    {"name": "upper_bounds", "type": ["null", {"type": "array", "items": ["null", {"type": "record", "name": "c6", "fields": [{"name": "key", "type": "int"}, {"name": "value", "type": "bytes"}]}]}]},
                    {"name": "key_metadata", "type": ["null", "bytes"]},
                    {"name": "split_offsets", "type": ["null", {"type": "array", "items": "long"}]},
                    {"name": "equality_ids", "type": ["null", {"type": "array", "items": "int"}]},
                    {"name": "sort_order_id", "type": ["null", "int"]}
                ]}}
            ]
        }"#;

        fn manifest_list_record(manifest_path: &str) -> Value {
            Value::Record(vec![
                ("manifest_path".into(), Value::String(manifest_path.into())),
                ("manifest_length".into(), Value::Long(0)),
                ("partition_spec_id".into(), Value::Int(0)),
                ("content".into(), Value::Int(0)),
                ("sequence_number".into(), Value::Long(0)),
                ("min_sequence_number".into(), Value::Long(0)),
                ("added_snapshot_id".into(), Value::Long(1001)),
                ("added_files_count".into(), Value::Int(0)),
                ("existing_files_count".into(), Value::Int(0)),
                ("deleted_files_count".into(), Value::Int(0)),
                ("added_rows_count".into(), Value::Long(0)),
                ("existing_rows_count".into(), Value::Long(0)),
                ("deleted_rows_count".into(), Value::Long(0)),
                ("partitions".into(), Value::Null),
                ("key_metadata".into(), Value::Null),
            ])
        }

        fn manifest_entry(status: i64, file_path: &str) -> Value {
            Value::Record(vec![
                ("status".into(), Value::Int(status as i32)),
                ("snapshot_id".into(), Value::Null),
                ("sequence_number".into(), Value::Null),
                ("file_sequence_number".into(), Value::Null),
                (
                    "data_file".into(),
                    Value::Record(vec![
                        ("content".into(), Value::Int(0)),
                        ("file_path".into(), Value::String(file_path.into())),
                        ("file_format".into(), Value::String("PARQUET".into())),
                        ("partition".into(), Value::Record(vec![])),
                        ("record_count".into(), Value::Long(1)),
                        ("file_size_in_bytes".into(), Value::Long(1)),
                        ("column_sizes".into(), Value::Null),
                        ("value_counts".into(), Value::Null),
                        ("null_value_counts".into(), Value::Null),
                        ("nan_value_counts".into(), Value::Null),
                        ("lower_bounds".into(), Value::Null),
                        ("upper_bounds".into(), Value::Null),
                        ("key_metadata".into(), Value::Null),
                        ("split_offsets".into(), Value::Null),
                        ("equality_ids".into(), Value::Null),
                        ("sort_order_id".into(), Value::Null),
                    ]),
                ),
            ])
        }

        // location must be absolute so resolve_path joins correctly.
        let location = table.to_string_lossy().replace('\\', "/");

        let metadata = serde_json::json!({
            "format-version": 2,
            "location": location,
            "current-snapshot-id": 1001,
            "current-schema-id": 0,
            "schemas": [],
            "snapshots": [
                {
                    "snapshot-id": 1001,
                    "timestamp-ms": 1710000000000i64,
                    "summary": {"operation": "append"},
                    "manifest-list": "metadata/snap-1001-manifest-list.avro"
                }
            ]
        });
        fs::write(
            meta_dir.join("v1.metadata.json"),
            serde_json::to_string(&metadata).unwrap(),
        )
        .unwrap();
        fs::write(meta_dir.join("version-hint.text"), "1\n").unwrap();

        let ml_schema = parse_schema(manifest_list_schema).unwrap();
        write_container(
            &meta_dir.join("snap-1001-manifest-list.avro"),
            manifest_list_schema,
            "deflate",
            &[
                (ml_schema.clone(), manifest_list_record("metadata/snap-1001-m0.avro")),
                (ml_schema.clone(), manifest_list_record("metadata/snap-1001-m1.avro")),
            ],
        );

        let m_schema = parse_schema(manifest_schema).unwrap();
        // m0: one added, one existing, one deleted.
        write_container(
            &meta_dir.join("snap-1001-m0.avro"),
            manifest_schema,
            "deflate",
            &[
                (m_schema.clone(), manifest_entry(1, "data/active1.parquet")),
                (m_schema.clone(), manifest_entry(0, "data/active2.parquet")),
                (m_schema.clone(), manifest_entry(2, "data/deleted1.parquet")),
            ],
        );
        // m1: everything deleted (e.g. after compaction).
        write_container(
            &meta_dir.join("snap-1001-m1.avro"),
            manifest_schema,
            "deflate",
            &[(m_schema.clone(), manifest_entry(2, "data/compacted.parquet"))],
        );

        // An orphan file on disk not referenced by any manifest must not leak in.
        let data_dir = table.join("data");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("orphan.parquet"), "x").unwrap();

        let active = crate::native_reader::list_active_data_files(&location).unwrap();
        assert_eq!(
            active,
            vec![
                format!("{location}/data/active1.parquet"),
                format!("{location}/data/active2.parquet"),
            ],
            "active files must come from manifests only (no deleted/orphan files)"
        );
    }

    #[test]
    fn test_iceberg_no_snapshot_returns_empty() {
        let table = temp_dir("iceberg_no_snapshot");
        let meta_dir = table.join("metadata");
        fs::create_dir_all(&meta_dir).unwrap();
        let metadata = serde_json::json!({
            "format-version": 2,
            "location": table.to_string_lossy().replace('\\', "/"),
            "snapshots": []
        });
        fs::write(
            meta_dir.join("v1.metadata.json"),
            serde_json::to_string(&metadata).unwrap(),
        )
        .unwrap();

        let active = crate::native_reader::list_active_data_files(&table.to_string_lossy().replace('\\', "/")).unwrap();
        assert!(active.is_empty());
    }

    #[test]
    fn test_value_field_lookup() {
        let v = Value::Record(vec![
            ("a".into(), Value::String("x".into())),
            ("b".into(), Value::Long(7)),
        ]);
        assert_eq!(v.field("a").unwrap().as_str(), Some("x"));
        assert_eq!(v.field("b").unwrap().as_i64(), Some(7));
        assert!(v.field("missing").is_none());
    }
}
