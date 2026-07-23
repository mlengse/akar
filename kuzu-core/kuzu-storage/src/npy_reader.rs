//! NumPy NPY file format reader.
//!
//! Reads `.npy` files (NumPy array binary format, v1.0).
//! Format: magic "\x93NUMPY", version byte, header_len (u16 LE),
//!         Python dict header, then raw array data.
//!
//! Supports: f8 (float64), f4 (float32), i8 (int64), i4 (int32),
//!           i2 (int16), i1 (int8), u8 (uint64), u4 (uint32),
//!           u2 (uint16), u1 (uint8), b (bool).

use kuzu_common::types::Value;
use std::io::Read;

/// Error type for NPY reader operations.
#[derive(Debug)]
pub enum NpyReaderError {
    IoError(std::io::Error),
    InvalidMagic,
    InvalidVersion,
    InvalidHeader,
    TypeNotSupported(String),
    ShapeMismatch { expected_rows: usize, actual: usize },
}

impl std::fmt::Display for NpyReaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NpyReaderError::IoError(e) => write!(f, "NPY I/O error: {e}"),
            NpyReaderError::InvalidMagic => write!(f, "Invalid NPY magic bytes"),
            NpyReaderError::InvalidVersion => write!(f, "Unsupported NPY version"),
            NpyReaderError::InvalidHeader => write!(f, "Invalid or unparseable NPY header"),
            NpyReaderError::TypeNotSupported(t) => write!(f, "NPY dtype '{}' not supported", t),
            NpyReaderError::ShapeMismatch { expected_rows, actual } => write!(
                f, "Shape mismatch: expected {} rows, file has {} elements", expected_rows, actual
            ),
        }
    }
}

/// Parsed NPY file header.
#[derive(Debug)]
struct NpyHeader {
    _descr: String,
    _fortran_order: bool,
    shape: Vec<usize>,
    data_offset: usize,
    dtype: NpyDtype,
}

#[derive(Debug, Clone, PartialEq)]
enum NpyDtype {
    Float64,
    Float32,
    Int64,
    Int32,
    Int16,
    Int8,
    UInt64,
    UInt32,
    UInt16,
    UInt8,
    Bool,
    String, // Fixed-length string
}

impl NpyDtype {
    fn from_str(s: &str) -> Result<Self, NpyReaderError> {
        // Normalize: strip byte order prefix and whitespace
        let s = s.trim();
        let s = if s.starts_with('<') || s.starts_with('>') || s.starts_with('=') || s.starts_with('|') {
            &s[1..]
        } else {
            s
        };
        match s {
            "f8" | "float64" => Ok(NpyDtype::Float64),
            "f4" | "float32" => Ok(NpyDtype::Float32),
            "i8" | "int64" => Ok(NpyDtype::Int64),
            "i4" | "int32" => Ok(NpyDtype::Int32),
            "i2" | "int16" => Ok(NpyDtype::Int16),
            "i1" | "int8" => Ok(NpyDtype::Int8),
            "u8" | "uint64" => Ok(NpyDtype::UInt64),
            "u4" | "uint32" => Ok(NpyDtype::UInt32),
            "u2" | "uint16" => Ok(NpyDtype::UInt16),
            "u1" | "uint8" => Ok(NpyDtype::UInt8),
            "b1" | "bool" => Ok(NpyDtype::Bool),
            _ if s.starts_with('S') || s.starts_with('U') => Ok(NpyDtype::String),
            _ => Err(NpyReaderError::TypeNotSupported(s.to_string())),
        }
    }

    fn size(&self) -> usize {
        match self {
            NpyDtype::Float64 => 8,
            NpyDtype::Float32 => 4,
            NpyDtype::Int64 | NpyDtype::UInt64 => 8,
            NpyDtype::Int32 | NpyDtype::UInt32 => 4,
            NpyDtype::Int16 | NpyDtype::UInt16 => 2,
            NpyDtype::Int8 | NpyDtype::UInt8 | NpyDtype::Bool => 1,
            NpyDtype::String => 1, // variable, handle separately
        }
    }
}

/// Parse the NPY header from a byte buffer.
fn parse_header(data: &[u8]) -> Result<NpyHeader, NpyReaderError> {
    if data.len() < 10 {
        return Err(NpyReaderError::InvalidMagic);
    }

    // Check magic: \x93NUMPY
    if &data[0..6] != b"\x93NUMPY" {
        return Err(NpyReaderError::InvalidMagic);
    }

    // Version byte
    let version = data[6];
    if version != 1 && version != 2 && version != 3 {
        return Err(NpyReaderError::InvalidVersion);
    }

    // Header length (u16 LE)
    let header_len = u16::from_le_bytes([data[8], data[9]]) as usize;
    let data_offset = 10 + header_len;

    if data.len() < data_offset {
        return Err(NpyReaderError::InvalidHeader);
    }

    // Parse Python dict header
    let header_str = std::str::from_utf8(&data[10..data_offset])
        .map_err(|_| NpyReaderError::InvalidHeader)?
        .trim()
        .trim_end_matches('\n');

    // Parse simple Python dict: {'descr': '...', 'fortran_order': bool, 'shape': (...)}
    let descr = extract_py_str(header_str, "descr")
        .unwrap_or("<f8".to_string());
    let fortran_order = extract_py_bool(header_str, "fortran_order").unwrap_or(false);
    let shape = extract_py_tuple(header_str, "shape").unwrap_or(vec![0]);

    // Validate Fortran order
    if fortran_order {
        return Err(NpyReaderError::TypeNotSupported(
            "Fortran-ordered arrays are not supported".into()
        ));
    }

    let dtype = NpyDtype::from_str(&descr)?;

    Ok(NpyHeader {
        _descr: descr,
        _fortran_order: fortran_order,
        shape,
        data_offset,
        dtype,
    })
}

fn extract_py_str(data: &str, key: &str) -> Option<String> {
    let key_pat = format!("'{}':", key);
    let start = data.find(&key_pat)?;
    let rest = &data[start + key_pat.len()..];
    let rest = rest.trim();
    let delim = rest.chars().next()?;
    let start_inner = 1; // skip opening quote
    let end_inner = rest[1..].find(delim)?;
    Some(rest[start_inner..start_inner + end_inner].to_string())
}

fn extract_py_bool(data: &str, key: &str) -> Option<bool> {
    let key_pat = format!("'{}':", key);
    let start = data.find(&key_pat)?;
    let rest = &data[start + key_pat.len()..].trim();
    if rest.starts_with("True") { Some(true) } else { Some(false) }
}

fn extract_py_tuple(data: &str, key: &str) -> Option<Vec<usize>> {
    let key_pat = format!("'{}':", key);
    let start = data.find(&key_pat)?;
    let rest = &data[start + key_pat.len()..].trim();

    if !rest.starts_with('(') {
        // Single integer
        let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        let num: usize = rest[..end].parse().ok()?;
        return Some(vec![num]);
    }

    let end_paren = rest.find(')')?;
    let inner = &rest[1..end_paren];
    // Remove trailing comma before closing paren
    let inner = inner.trim_end_matches(',');
    if inner.is_empty() {
        return Some(vec![]);
    }
    let nums: Vec<usize> = inner
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if nums.is_empty() { None } else { Some(nums) }
}

/// Read an NPY file and return its contents as `Vec<Value>`.
///
/// For 1D arrays, returns one Value per element.
/// For multi-dimensional arrays, returns the total element count.
pub fn read_npy(path: &str) -> Result<Vec<Value>, NpyReaderError> {
    let mut file = std::fs::File::open(path).map_err(NpyReaderError::IoError)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data).map_err(NpyReaderError::IoError)?;

    let header = parse_header(&data)?;

    let total_elements: usize = header.shape.iter().product();
    let raw = &data[header.data_offset..];

    read_values(raw, &header.dtype, total_elements)
}

fn read_values(raw: &[u8], dtype: &NpyDtype, count: usize) -> Result<Vec<Value>, NpyReaderError> {
    let elem_size = dtype.size();
    if count * elem_size > raw.len() {
        return Err(NpyReaderError::ShapeMismatch {
            expected_rows: count,
            actual: raw.len() / elem_size.max(1),
        });
    }

    let mut values = Vec::with_capacity(count);
    for i in 0..count {
        let offset = i * elem_size;
        let val = match dtype {
            NpyDtype::Float64 => {
                let bytes: [u8; 8] = raw[offset..offset + 8].try_into().unwrap();
                Value::Double(f64::from_le_bytes(bytes))
            }
            NpyDtype::Float32 => {
                let bytes: [u8; 4] = raw[offset..offset + 4].try_into().unwrap();
                Value::Float(f32::from_le_bytes(bytes))
            }
            NpyDtype::Int64 => {
                let bytes: [u8; 8] = raw[offset..offset + 8].try_into().unwrap();
                Value::Int64(i64::from_le_bytes(bytes))
            }
            NpyDtype::Int32 => {
                let bytes: [u8; 4] = raw[offset..offset + 4].try_into().unwrap();
                Value::Int32(i32::from_le_bytes(bytes))
            }
            NpyDtype::Int16 => {
                let bytes: [u8; 2] = raw[offset..offset + 2].try_into().unwrap();
                Value::Int16(i16::from_le_bytes(bytes))
            }
            NpyDtype::Int8 => Value::Int8(raw[offset] as i8),
            NpyDtype::UInt64 => {
                let bytes: [u8; 8] = raw[offset..offset + 8].try_into().unwrap();
                Value::UInt64(u64::from_le_bytes(bytes))
            }
            NpyDtype::UInt32 => {
                let bytes: [u8; 4] = raw[offset..offset + 4].try_into().unwrap();
                Value::UInt32(u32::from_le_bytes(bytes))
            }
            NpyDtype::UInt16 => {
                let bytes: [u8; 2] = raw[offset..offset + 2].try_into().unwrap();
                Value::UInt16(u16::from_le_bytes(bytes))
            }
            NpyDtype::UInt8 => Value::UInt8(raw[offset]),
            NpyDtype::Bool => Value::Bool(raw[offset] != 0),
            NpyDtype::String => Value::Null, // not implemented
        };
        values.push(val);
    }

    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[allow(dead_code)]
    fn write_test_npy(path: &str, values: &[f64]) {
        let mut file = std::fs::File::create(path).unwrap();

        // Magic
        file.write_all(b"\x93NUMPY").unwrap();
        // Version
        file.write_all(&[1u8, 0u8]).unwrap();
        // Header
        let header = format!(
            "{{'descr': '<f8', 'fortran_order': False, 'shape': ({},), }}",
            values.len()
        );
        // Pad header to 16-byte alignment, ensure it ends with \n
        let mut header_bytes = header.into_bytes();
        while (10 + header_bytes.len()) % 16 != 0 {
            header_bytes.push(b' ');
        }
        header_bytes.push(b'\n');
        let header_len = header_bytes.len() as u16;
        file.write_all(&header_len.to_le_bytes()).unwrap();
        file.write_all(&header_bytes).unwrap();

        // Data
        for &v in values {
            file.write_all(&v.to_le_bytes()).unwrap();
        }
    }

    #[test]
    fn test_parse_simple_header() {
        let header_str = "{'descr': '<f8', 'fortran_order': False, 'shape': (3,), }";
        let mut header_bytes = header_str.as_bytes().to_vec();
        #[allow(clippy::manual_is_multiple_of)]
        while (10 + header_bytes.len()) % 16 != 0 {
            header_bytes.push(b' ');
        }
        header_bytes.push(b'\n');
        let header_len = header_bytes.len() as u16;
        let data_offset: usize = 10 + header_len as usize;

        let mut buf = vec![];
        buf.extend_from_slice(b"\x93NUMPY\x01\x00");
        buf.extend_from_slice(&header_len.to_le_bytes());
        buf.extend_from_slice(&header_bytes);
        // Pad to data_offset + data
        buf.resize(data_offset + 3 * 8, 0);
        buf[data_offset..data_offset + 8].copy_from_slice(&1.0f64.to_le_bytes());
        buf[data_offset + 8..data_offset + 16].copy_from_slice(&2.0f64.to_le_bytes());
        buf[data_offset + 16..data_offset + 24].copy_from_slice(&3.0f64.to_le_bytes());

        let header = parse_header(&buf).unwrap();
        assert_eq!(header.shape, vec![3]);
        assert_eq!(header.dtype, NpyDtype::Float64);

        let vals = read_values(&buf[header.data_offset..], &header.dtype, 3).unwrap();
        assert_eq!(vals.len(), 3);
        assert_eq!(vals[0], Value::Double(1.0));
        assert_eq!(vals[1], Value::Double(2.0));
        assert_eq!(vals[2], Value::Double(3.0));
    }

    #[test]
    fn test_npy_int32() {
        let header_str = "{'descr': '<i4', 'fortran_order': False, 'shape': (2,), }";
        let mut header_bytes = header_str.as_bytes().to_vec();
        #[allow(clippy::manual_is_multiple_of)]
        while (10 + header_bytes.len()) % 16 != 0 {
            header_bytes.push(b' ');
        }
        header_bytes.push(b'\n');
        let header_len = header_bytes.len() as u16;
        let data_offset: usize = 10 + header_len as usize;

        let mut buf = vec![];
        buf.extend_from_slice(b"\x93NUMPY\x01\x00");
        buf.extend_from_slice(&header_len.to_le_bytes());
        buf.extend_from_slice(&header_bytes);
        buf.resize(data_offset + 2 * 4, 0);
        buf[data_offset..data_offset + 4].copy_from_slice(&42i32.to_le_bytes());
        buf[data_offset + 4..data_offset + 8].copy_from_slice(&(-7i32).to_le_bytes());

        let header = parse_header(&buf).unwrap();
        let vals = read_values(&buf[header.data_offset..], &header.dtype, 2).unwrap();
        assert_eq!(vals[0], Value::Int32(42));
        assert_eq!(vals[1], Value::Int32(-7));
    }
}
