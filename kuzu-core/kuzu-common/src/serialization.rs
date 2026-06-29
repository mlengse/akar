//! Binary serialization/deserialization primitives (LittleEndian).
//!
//! Used by the storage engine to persist data to disk.

use std::io::{Read, Write};

/// Serialize a value into a byte buffer (LittleEndian).
pub trait Serialize {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()>;
}

/// Deserialize a value from a byte buffer (LittleEndian).
pub trait Deserialize: Sized {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self>;
}

// --- Primitive implementations ---

macro_rules! impl_serialize_le {
    ($($ty:ty),*) => {
        $(
            impl Serialize for $ty {
                fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
                    writer.write_all(&self.to_le_bytes())
                }
            }

            impl Deserialize for $ty {
                fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
                    let mut buf = [0u8; std::mem::size_of::<Self>()];
                    reader.read_exact(&mut buf)?;
                    Ok(<$ty>::from_le_bytes(buf))
                }
            }
        )*
    };
}

impl_serialize_le!(i8, u8, i16, u16, i32, u32, i64, u64, f32, f64);

// --- Serialization for Kuzu types ---

impl Serialize for crate::types::InternalID {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        self.table_id.serialize(writer)?;
        self.offset.serialize(writer)
    }
}

impl Deserialize for crate::types::InternalID {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let table_id = u64::deserialize(reader)?;
        let offset = u64::deserialize(reader)?;
        Ok(crate::types::InternalID { table_id, offset })
    }
}

impl Serialize for crate::types::Date {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        self.0.serialize(writer)
    }
}

impl Deserialize for crate::types::Date {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(crate::types::Date(i32::deserialize(reader)?))
    }
}

impl Serialize for crate::types::Timestamp {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        self.0.serialize(writer)
    }
}

impl Deserialize for crate::types::Timestamp {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(crate::types::Timestamp(i64::deserialize(reader)?))
    }
}

impl Serialize for crate::types::Interval {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        self.months.serialize(writer)?;
        self.days.serialize(writer)?;
        self.micros.serialize(writer)
    }
}

impl Deserialize for crate::types::Interval {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let months = i32::deserialize(reader)?;
        let days = i32::deserialize(reader)?;
        let micros = i64::deserialize(reader)?;
        Ok(crate::types::Interval { months, days, micros })
    }
}

// --- VarInt encoding helpers for storage ---

/// Encode a u64 using unsigned varint encoding (used for smaller storage).
pub fn write_varint<W: Write>(writer: &mut W, mut value: u64) -> std::io::Result<()> {
    loop {
        if value < 0x80 {
            writer.write_all(&[value as u8])?;
            break;
        } else {
            writer.write_all(&[(value as u8) | 0x80])?;
            value >>= 7;
        }
    }
    Ok(())
}

/// Decode a u64 using unsigned varint encoding.
pub fn read_varint<R: Read>(reader: &mut R) -> std::io::Result<u64> {
    let mut result = 0u64;
    let mut shift = 0;
    loop {
        let mut byte = [0u8];
        reader.read_exact(&mut byte)?;
        result |= ((byte[0] & 0x7F) as u64) << shift;
        if byte[0] & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Date, InternalID, Interval, Timestamp};

    #[test]
    fn test_serialize_primitive_roundtrip() {
        let mut buf = Vec::new();
        42i64.serialize(&mut buf).unwrap();
        let val = i64::deserialize(&mut &buf[..]).unwrap();
        assert_eq!(val, 42);
    }

    #[test]
    fn test_serialize_u32_roundtrip() {
        let mut buf = Vec::new();
        12345u32.serialize(&mut buf).unwrap();
        let val = u32::deserialize(&mut &buf[..]).unwrap();
        assert_eq!(val, 12345);
    }

    #[test]
    fn test_serialize_f64_roundtrip() {
        let mut buf = Vec::new();
        std::f64::consts::PI.serialize(&mut buf).unwrap();
        let val = f64::deserialize(&mut &buf[..]).unwrap();
        assert!((val - std::f64::consts::PI).abs() < 1e-10);
    }

    #[test]
    fn test_serialize_internal_id() {
        let id = InternalID {
            table_id: 10,
            offset: 42,
        };
        let mut buf = Vec::new();
        id.serialize(&mut buf).unwrap();
        let val = InternalID::deserialize(&mut &buf[..]).unwrap();
        assert_eq!(val.table_id, 10);
        assert_eq!(val.offset, 42);
    }

    #[test]
    fn test_serialize_date() {
        let d = Date(365);
        let mut buf = Vec::new();
        d.serialize(&mut buf).unwrap();
        let val = Date::deserialize(&mut &buf[..]).unwrap();
        assert_eq!(val, d);
    }

    #[test]
    fn test_serialize_timestamp() {
        let ts = Timestamp(1_700_000_000_000_000);
        let mut buf = Vec::new();
        ts.serialize(&mut buf).unwrap();
        let val = Timestamp::deserialize(&mut &buf[..]).unwrap();
        assert_eq!(val, ts);
    }

    #[test]
    fn test_serialize_interval() {
        let iv = Interval {
            months: 12,
            days: 30,
            micros: 1_000_000,
        };
        let mut buf = Vec::new();
        iv.serialize(&mut buf).unwrap();
        let val = Interval::deserialize(&mut &buf[..]).unwrap();
        assert_eq!(val.months, 12);
        assert_eq!(val.days, 30);
        assert_eq!(val.micros, 1_000_000);
    }

    #[test]
    fn test_varint_roundtrip() {
        let test_values = [0u64, 1, 127, 128, 255, 16383, 16384, 1_000_000, u64::MAX];
        for &v in &test_values {
            let mut buf = Vec::new();
            write_varint(&mut buf, v).unwrap();
            let val = read_varint(&mut &buf[..]).unwrap();
            assert_eq!(val, v, "varint roundtrip failed for {v}");
        }
    }

    #[test]
    fn test_multi_value_serialization() {
        let mut buf = Vec::new();
        1i32.serialize(&mut buf).unwrap();
        2.5f64.serialize(&mut buf).unwrap();

        let mut reader = &buf[..];
        assert_eq!(i32::deserialize(&mut reader).unwrap(), 1);
        assert!((f64::deserialize(&mut reader).unwrap() - 2.5).abs() < 1e-10);
    }
}
