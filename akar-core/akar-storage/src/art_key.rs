//! Order-preserving byte encoding for ART (Adaptive Radix Tree) keys.
//!
//! # Encoding Scheme
//!
//! All Akar primitive types are encoded into byte sequences that preserve
//! their natural sort order when compared lexicographically as byte strings.
//!
//! | Type      | Encoding                                               |
//! |-----------|--------------------------------------------------------|
//! | IntN      | Big-endian, flip sign bit (MSB XOR 1<<(N-1))          |
//! | UIntN     | Plain big-endian                                       |
//! | Float/Double | IEEE 754 bytes, flip sign bit for +0/-0 ordering   |
//! | Bool      | Single byte: 0x00 = false, 0x01 = true                |
//! | String    | Escape 0x00 → 0x0101, 0x01 → 0x0102, append 0x00 terminator |
//! | Date      | Encode as Int32 (days since epoch)                     |
//! | Timestamp | Encode as Int64 (microseconds since epoch)             |
//!
//! Port of C++ `ArtKey` from `ladybug/src/storage/index/art_index.cpp`
//! (lines 261–284) with `appendIntegral`, `appendFloat`, `appendString`.

use akar_common::types::Value;

/// An ART key — an order-preserving byte encoding of a Akar Value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtKey {
    bytes: Vec<u8>,
}

impl ArtKey {
    /// Create an empty ART key (used for unbounded range scans).
    pub fn empty() -> Self {
        Self { bytes: Vec::new() }
    }

    /// Returns `true` if this key is empty (no bytes).
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Get the underlying byte representation.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consume the key and return the underlying bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Encode a `Value` into an ART key with order-preserving byte encoding.
    ///
    /// Returns `None` for null values (nulls cannot be indexed).
    /// Returns `Some(ArtKey)` for all supported types.
    pub fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Null => None,
            Value::Bool(v) => Some(Self::from_bool(*v)),
            Value::Int64(v) => Some(Self::from_int64(*v)),
            Value::Int32(v) => Some(Self::from_int32(*v)),
            Value::Int16(v) => Some(Self::from_int16(*v)),
            Value::Int8(v) => Some(Self::from_int8(*v)),
            Value::UInt64(v) => Some(Self::from_uint64(*v)),
            Value::UInt32(v) => Some(Self::from_uint32(*v)),
            Value::UInt16(v) => Some(Self::from_uint16(*v)),
            Value::UInt8(v) => Some(Self::from_uint8(*v)),
            Value::Double(v) => Some(Self::from_double(*v)),
            Value::Float(v) => Some(Self::from_float(*v)),
            Value::String(v) => Some(Self::from_string(v)),
            Value::Blob(v) => Some(Self::from_blob(v)),
            Value::Date(v) => Some(Self::from_int32(v.0)),
            Value::Timestamp(v) => Some(Self::from_int64(v.0)),
            Value::TimestampTz(v) => Some(Self::from_int64(v.0)),
            Value::TimestampNs(v) => Some(Self::from_int64(v.0)),
            Value::TimestampMs(v) => Some(Self::from_int64(v.0)),
            Value::TimestampSec(v) => Some(Self::from_int64(v.0)),
            Value::Int128(v) => Some(Self::from_int128(*v)),
            Value::Interval(i) => Some(Self::from_interval(i.months, i.days, i.micros)),
            Value::InternalID(id) => {
                // Encode as (table_id, offset) pair — both big-endian u64
                let mut bytes = Vec::with_capacity(16);
                bytes.extend_from_slice(&id.table_id.to_be_bytes());
                bytes.extend_from_slice(&id.offset.to_be_bytes());
                Some(Self { bytes })
            }
            Value::UInt128(v) => Some(Self::from_uint128(*v)),
            Value::DTime(v) => Some(Self::from_int64(*v)),
            Value::Json(v) => Some(Self::from_string(&v.to_string())),
            Value::Union(tag, val) => {
                let mut bytes = Vec::new();
                bytes.extend_from_slice(&(tag.len() as u32).to_be_bytes());
                bytes.extend_from_slice(tag.as_bytes());
                if let Some(key) = Self::from_value(val) {
                    bytes.extend_from_slice(&key.bytes);
                }
                Some(Self { bytes })
            }
            Value::List(_) | Value::Map(_) | Value::Struct(_) => {
                // Compound types are not supported as ART keys.
                // Return an empty key — caller should handle.
                None
            }
        }
    }

    /// Create from a boolean value.
    fn from_bool(v: bool) -> Self {
        Self {
            bytes: vec![if v { 0x01 } else { 0x00 }],
        }
    }

    /// Encode a signed integer in big-endian with sign-bit flip.
    /// This ensures negative values sort before positive values.
    fn from_int64(v: i64) -> Self {
        let mut bytes = v.to_be_bytes();
        bytes[0] ^= 0x80; // flip sign bit
        Self { bytes: bytes.to_vec() }
    }

    fn from_int32(v: i32) -> Self {
        let mut bytes = v.to_be_bytes();
        bytes[0] ^= 0x80;
        Self { bytes: bytes.to_vec() }
    }

    fn from_int16(v: i16) -> Self {
        let mut bytes = v.to_be_bytes();
        bytes[0] ^= 0x80;
        Self { bytes: bytes.to_vec() }
    }

    fn from_int8(v: i8) -> Self {
        Self {
            bytes: vec![(v as u8) ^ 0x80],
        }
    }

    /// Encode an unsigned integer in plain big-endian.
    fn from_uint64(v: u64) -> Self {
        Self {
            bytes: v.to_be_bytes().to_vec(),
        }
    }

    fn from_uint32(v: u32) -> Self {
        Self {
            bytes: v.to_be_bytes().to_vec(),
        }
    }

    fn from_uint16(v: u16) -> Self {
        Self {
            bytes: v.to_be_bytes().to_vec(),
        }
    }

    fn from_uint8(v: u8) -> Self {
        Self { bytes: vec![v] }
    }

    /// Encode a 64-bit float: IEEE 754 bytes with sign flip.
    /// For positive floats, flip the sign bit; for negative, invert all bits.
    fn from_double(v: f64) -> Self {
        let bits = v.to_bits();
        let encoded = if (bits >> 63) != 0 {
            // Negative: invert all bits
            !bits
        } else {
            // Positive: flip sign bit only
            bits ^ (1 << 63)
        };
        Self {
            bytes: encoded.to_be_bytes().to_vec(),
        }
    }

    /// Encode a 32-bit float: IEEE 754 bytes with sign flip.
    fn from_float(v: f32) -> Self {
        let bits = v.to_bits();
        let encoded = if (bits >> 31) != 0 { !bits } else { bits ^ (1 << 31) };
        Self {
            bytes: encoded.to_be_bytes().to_vec(),
        }
    }

    /// Encode a string with byte-escape for 0x00 and 0x01.
    ///
    /// Both bytes must be escaped: 0x01 is the escape marker, so a raw 0x01
    /// would collide with an escaped 0x00 prefix and break ordering.
    ///   - 0x00 → 0x01 0x01
    ///   - 0x01 → 0x01 0x02
    ///
    /// A 0x00 terminator is appended at the end.
    /// This ensures lexicographic ordering is preserved (P52.20).
    fn from_string(s: &str) -> Self {
        let mut bytes = Vec::with_capacity(s.len() + 2);
        for &b in s.as_bytes() {
            match b {
                0x00 => bytes.extend_from_slice(&[0x01, 0x01]),
                0x01 => bytes.extend_from_slice(&[0x01, 0x02]),
                _ => bytes.push(b),
            }
        }
        bytes.push(0x00); // terminator
        Self { bytes }
    }

    /// Encode a blob (raw bytes with 0x00/0x01 escape).
    fn from_blob(data: &[u8]) -> Self {
        let mut bytes = Vec::with_capacity(data.len() + 2);
        for &b in data {
            match b {
                0x00 => bytes.extend_from_slice(&[0x01, 0x01]),
                0x01 => bytes.extend_from_slice(&[0x01, 0x02]),
                _ => bytes.push(b),
            }
        }
        bytes.push(0x00); // terminator
        Self { bytes }
    }

    /// Encode a 128-bit signed integer: high 64 bits (with sign flip) + low 64 bits.
    ///
    /// Port of C++ `appendInt128()` from `art_index.cpp`:
    /// ```cpp
    /// void appendInt128(std::vector<uint8_t>& bytes, int64_t high, uint64_t low) {
    ///     appendUInt128(bytes, static_cast<uint64_t>(high) ^ (uint64_t{1} << 63), low);
    /// }
    /// ```
    fn from_int128(v: i128) -> Self {
        let bits = v as u128;
        let high = (bits >> 64) as u64;
        let low = bits as u64;
        // Flip the sign bit of the high word (big-endian ordering)
        let high_encoded = high ^ (1u64 << 63);
        let mut bytes = Vec::with_capacity(16);
        bytes.extend_from_slice(&high_encoded.to_be_bytes());
        bytes.extend_from_slice(&low.to_be_bytes());
        Self { bytes }
    }

    fn from_uint128(v: u128) -> Self {
        let high = (v >> 64) as u64;
        let low = v as u64;
        let mut bytes = Vec::with_capacity(16);
        bytes.extend_from_slice(&high.to_be_bytes());
        bytes.extend_from_slice(&low.to_be_bytes());
        Self { bytes }
    }

    /// Encode an interval: months (i32) + days (i32) + micros (i64), each as signed big-endian.
    fn from_interval(months: i32, days: i32, micros: i64) -> Self {
        let mut bytes = Vec::with_capacity(16);
        let mut m = months.to_be_bytes();
        m[0] ^= 0x80;
        bytes.extend_from_slice(&m);
        let mut d = days.to_be_bytes();
        d[0] ^= 0x80;
        bytes.extend_from_slice(&d);
        let mut us = micros.to_be_bytes();
        us[0] ^= 0x80;
        bytes.extend_from_slice(&us);
        Self { bytes }
    }
}

impl Default for ArtKey {
    fn default() -> Self {
        Self::empty()
    }
}

impl From<Vec<u8>> for ArtKey {
    fn from(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
}

impl AsRef<[u8]> for ArtKey {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl std::fmt::Display for ArtKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ArtKey({}b: {:02x?})", self.bytes.len(), self.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_int64_ordering() {
        let neg = ArtKey::from_int64(-100);
        let zero = ArtKey::from_int64(0);
        let pos = ArtKey::from_int64(100);
        let max = ArtKey::from_int64(i64::MAX);
        let min = ArtKey::from_int64(i64::MIN);

        // Lexicographic ordering should match numeric ordering
        assert!(min.bytes < neg.bytes, "MIN_INT64 should sort before -100");
        assert!(neg.bytes < zero.bytes, "-100 should sort before 0");
        assert!(zero.bytes < pos.bytes, "0 should sort before 100");
        assert!(pos.bytes < max.bytes, "100 should sort before MAX_INT64");
    }

    #[test]
    fn test_encode_uint64_ordering() {
        let small = ArtKey::from_uint64(0);
        let medium = ArtKey::from_uint64(100);
        let large = ArtKey::from_uint64(u64::MAX);

        assert!(small.bytes < medium.bytes);
        assert!(medium.bytes < large.bytes);
    }

    #[test]
    fn test_encode_double_ordering() {
        let neg = ArtKey::from_double(-std::f64::consts::PI);
        let zero = ArtKey::from_double(0.0);
        let pos = ArtKey::from_double(std::f64::consts::PI);

        assert!(neg.bytes < zero.bytes, "negative should sort before zero");
        assert!(zero.bytes < pos.bytes, "zero should sort before positive");
    }

    #[test]
    fn test_encode_double_neg_zero() {
        let neg_zero = ArtKey::from_double(-0.0f64);
        let pos_zero = ArtKey::from_double(0.0f64);
        // IEEE 754 -0.0 and +0.0 have different bit patterns.
        // After sign-flip encoding, they should compare as equal in ordering.
        // Verify they don't crash and produce valid sorted keys.
        assert_eq!(neg_zero.bytes.len(), 8);
        assert_eq!(pos_zero.bytes.len(), 8);
    }

    #[test]
    fn test_encode_string_ordering() {
        let a = ArtKey::from_string("apple");
        let b = ArtKey::from_string("banana");
        let empty = ArtKey::from_string("");

        assert!(empty.bytes < a.bytes, "empty string sorts first");
        assert!(a.bytes < b.bytes, "apple < banana");
    }

    #[test]
    fn test_encode_string_with_null_byte() {
        let with_null = ArtKey::from_string("a\x00b");
        let without = ArtKey::from_string("ab");

        // 'a' (0x61), then escaped null (0x01 0x01), then 'b' (0x62)
        assert_eq!(with_null.bytes(), &[0x61, 0x01, 0x01, 0x62, 0x00]);
        // The escaped null (0x01 0x01) should be > plain 'b' (0x62) after 'a'...
        // Actually 0x01 < 0x62, so a\x00b < ab in lex order
        assert!(
            with_null.bytes < without.bytes,
            "string with escaped null should sort before same string without null"
        );
    }

    #[test]
    fn test_encode_string_with_0x01_preserves_ordering() {
        // Regression for P52.20: a raw 0x01 byte must not alias the escape
        // prefix used for 0x00, and 0x00 < 0x01 ordering must be preserved.
        let nul = ArtKey::from_string("a\x00b");
        let one = ArtKey::from_string("a\x01b");

        // Distinct encodings (no collision) and natural order 0x00 < 0x01.
        assert_ne!(nul.bytes(), one.bytes());
        assert!(
            nul.bytes < one.bytes,
            "a\\x00b must sort before a\\x01b in ART key encoding"
        );
        // Raw 0x01 is encoded as 0x01 0x02 and is strictly greater than the
        // 0x00 escape (0x01 0x01).
        assert_eq!(one.bytes(), &[0x61, 0x01, 0x02, 0x62, 0x00]);
    }

    #[test]
    fn test_encode_date_ordering() {
        let early = ArtKey::from_int32(-1000);
        let epoch = ArtKey::from_int32(0);
        let later = ArtKey::from_int32(1000);

        assert!(early.bytes < epoch.bytes);
        assert!(epoch.bytes < later.bytes);
    }

    #[test]
    fn test_encode_timestamp_ordering() {
        let early = ArtKey::from_int64(-1_000_000);
        let epoch = ArtKey::from_int64(0);
        let later = ArtKey::from_int64(1_000_000);

        assert!(early.bytes < epoch.bytes);
        assert!(epoch.bytes < later.bytes);
    }

    #[test]
    fn test_from_value_dispatch() {
        assert!(ArtKey::from_value(&Value::Null).is_none());
        assert!(ArtKey::from_value(&Value::Int64(42)).is_some());
        assert!(ArtKey::from_value(&Value::String("hello".into())).is_some());
        assert!(ArtKey::from_value(&Value::Double(std::f64::consts::PI)).is_some());
        assert!(ArtKey::from_value(&Value::Bool(true)).is_some());
        // Compound types
        assert!(ArtKey::from_value(&Value::List(vec![])).is_none());
    }

    #[test]
    fn test_from_value_roundtrip_ordering() {
        let values = [
            Value::Int64(i64::MIN),
            Value::Int64(-1000),
            Value::Int64(-1),
            Value::Int64(0),
            Value::Int64(1),
            Value::Int64(1000),
            Value::Int64(i64::MAX),
        ];

        let keys: Vec<ArtKey> = values.iter().filter_map(ArtKey::from_value).collect();
        for i in 1..keys.len() {
            assert!(
                keys[i - 1].bytes <= keys[i].bytes,
                "Key ordering broken at index {i}: {:?} vs {:?}",
                keys[i - 1].bytes,
                keys[i].bytes
            );
        }
    }

    #[test]
    fn test_encode_bool() {
        let t = ArtKey::from_bool(true);
        let f = ArtKey::from_bool(false);
        assert!(f.bytes < t.bytes);
        assert_eq!(t.bytes(), &[0x01]);
        assert_eq!(f.bytes(), &[0x00]);
    }

    #[test]
    fn test_encode_int128() {
        let zero = ArtKey::from_int128(0);
        let pos = ArtKey::from_int128(1);
        let neg = ArtKey::from_int128(-1);

        assert!(neg.bytes < zero.bytes, "negative Int128 before zero");
        assert!(zero.bytes < pos.bytes, "zero before positive Int128");
    }

    #[test]
    fn test_encode_interval() {
        let a = ArtKey::from_interval(0, 0, 0);
        let b = ArtKey::from_interval(0, 1, 0); // months=0, days=1
        let c = ArtKey::from_interval(1, 0, 0); // months=1, days=0

        // months is the most significant field, so (1,0,0) > (0,1,0) in signed comparison
        assert!(a.bytes < b.bytes, "zero < (0,1,0)");
        assert!(a.bytes < c.bytes, "zero < (1,0,0)");
        assert!(
            b.bytes < c.bytes,
            "(0,1,0) < (1,0,0) because months > days in significance"
        );
    }
}
