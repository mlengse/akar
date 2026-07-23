//! String dictionary encoding for efficient string storage.
//!
//! Dictionary encoding maps strings to integer IDs, storing each unique
//! string only once in a dictionary. Repeated strings reference the dict
//! by ID, reducing storage for low-cardinality string columns.

use std::collections::HashMap;

/// A dictionary-encoded string column.
#[derive(Debug, Clone)]
pub struct StringDictionary {
    /// The dictionary: string_id -> string value.
    strings: Vec<String>,
    /// Reverse lookup: string value -> string_id.
    lookup: HashMap<String, u32>,
}

impl StringDictionary {
    /// Create a new empty dictionary.
    pub fn new() -> Self {
        Self {
            strings: Vec::new(),
            lookup: HashMap::new(),
        }
    }

    /// Encode a batch of strings, returning (dictionary, encoded_ids).
    ///
    /// The dictionary contains unique strings. `encoded_ids` is a `Vec<u32>`
    /// where each entry is the dictionary ID for the corresponding input string.
    /// Unknown/NULL strings get ID `u32::MAX`.
    pub fn encode(strings: &[Option<&str>]) -> (Self, Vec<u32>) {
        let mut dict = Self::new();
        let mut ids = Vec::with_capacity(strings.len());
        for s in strings {
            let id = match s {
                None => u32::MAX,
                Some(val) => dict.intern(val),
            };
            ids.push(id);
        }
        (dict, ids)
    }

    /// Insert a single string and return its ID.
    pub fn intern(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.lookup.get(s) {
            return id;
        }
        let id = self.strings.len() as u32;
        self.strings.push(s.to_string());
        self.lookup.insert(s.to_string(), id);
        id
    }

    /// Look up a string by ID. Returns `None` if ID is out of range.
    pub fn lookup(&self, id: u32) -> Option<&str> {
        self.strings.get(id as usize).map(|s| s.as_str())
    }

    /// Look up a string value and return its ID. Returns `None` if not found.
    pub fn lookup_id(&self, s: &str) -> Option<u32> {
        self.lookup.get(s).copied()
    }

    /// Return the number of unique strings in the dictionary.
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// Check if the dictionary is empty.
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }

    /// Serialize the dictionary to bytes.
    ///
    /// Format: `[num_strings: u32][for each string: [len: u32][bytes...]]`
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(self.strings.len() as u32).to_le_bytes());
        for s in &self.strings {
            buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
            buf.extend_from_slice(s.as_bytes());
        }
        buf
    }

    /// Deserialize a dictionary from bytes.
    pub fn deserialize(data: &[u8]) -> std::io::Result<Self> {
        if data.len() < 4 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "data too short for dictionary header",
            ));
        }
        let num_strings = u32::from_le_bytes(data[..4].try_into().unwrap()) as usize;
        let mut dict = Self::with_capacity(num_strings);
        let mut offset = 4usize;
        for _ in 0..num_strings {
            if offset + 4 > data.len() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "data too short for string length",
                ));
            }
            let len = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
            offset += 4;
            if offset + len > data.len() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "data too short for string content",
                ));
            }
            let s = String::from_utf8(data[offset..offset + len].to_vec())
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            offset += len;
            let id = dict.strings.len() as u32;
            dict.strings.push(s.clone());
            dict.lookup.insert(s, id);
        }
        Ok(dict)
    }

    /// Compute the memory usage in bytes.
    pub fn memory_usage(&self) -> usize {
        let mut total = std::mem::size_of::<Self>();
        total += self.strings.capacity() * std::mem::size_of::<String>();
        for s in &self.strings {
            total += s.capacity();
        }
        total += self.lookup.capacity() * (std::mem::size_of::<String>() + std::mem::size_of::<u32>());
        for k in self.lookup.keys() {
            total += k.capacity();
        }
        total
    }

    fn with_capacity(cap: usize) -> Self {
        Self {
            strings: Vec::with_capacity(cap),
            lookup: HashMap::with_capacity(cap),
        }
    }
}

impl Default for StringDictionary {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let input = [
            Some("apple"),
            Some("banana"),
            Some("apple"),
            Some("cherry"),
            Some("banana"),
        ];
        let (dict, ids) = StringDictionary::encode(&input);
        assert_eq!(dict.len(), 3);
        for (i, s) in input.iter().enumerate() {
            let expected = ids[i];
            if let Some(val) = s {
                assert_eq!(dict.lookup(expected), Some(*val));
            }
        }
    }

    #[test]
    fn test_intern_dedup() {
        let mut dict = StringDictionary::new();
        let id1 = dict.intern("hello");
        let id2 = dict.intern("hello");
        assert_eq!(id1, id2);
        let id3 = dict.intern("world");
        assert_ne!(id1, id3);
        assert_eq!(dict.len(), 2);
    }

    #[test]
    fn test_serialize_roundtrip() {
        let mut dict = StringDictionary::new();
        dict.intern("alpha");
        dict.intern("beta");
        dict.intern("gamma");
        let bytes = dict.serialize();
        let deserialized = StringDictionary::deserialize(&bytes).unwrap();
        assert_eq!(deserialized.len(), dict.len());
        assert_eq!(deserialized.lookup_id("alpha"), Some(0));
        assert_eq!(deserialized.lookup_id("beta"), Some(1));
        assert_eq!(deserialized.lookup_id("gamma"), Some(2));
        assert_eq!(deserialized.lookup_id("delta"), None);
    }

    #[test]
    fn test_empty_input() {
        let input: &[Option<&str>] = &[];
        let (dict, ids) = StringDictionary::encode(input);
        assert!(dict.is_empty());
        assert!(ids.is_empty());
    }

    #[test]
    fn test_null_handling() {
        let input = [Some("a"), None, Some("b"), None];
        let (dict, ids) = StringDictionary::encode(&input);
        assert_eq!(dict.len(), 2);
        assert_eq!(ids[0], 0);
        assert_eq!(ids[1], u32::MAX);
        assert_eq!(ids[2], 1);
        assert_eq!(ids[3], u32::MAX);
    }

    #[test]
    fn test_lookup_miss() {
        let mut dict = StringDictionary::new();
        dict.intern("foo");
        assert_eq!(dict.lookup(0), Some("foo"));
        assert_eq!(dict.lookup(1), None);
        assert_eq!(dict.lookup(u32::MAX), None);
        assert_eq!(dict.lookup_id("bar"), None);
    }

    #[test]
    fn test_memory_usage() {
        let mut dict = StringDictionary::new();
        dict.intern("short");
        dict.intern("a longer string value");
        let usage = dict.memory_usage();
        assert!(usage > 0);
        assert!(usage > std::mem::size_of::<StringDictionary>());
    }

    #[test]
    fn test_deserialize_empty() {
        let bytes = 0u32.to_le_bytes().to_vec();
        let dict = StringDictionary::deserialize(&bytes).unwrap();
        assert!(dict.is_empty());
        assert_eq!(dict.len(), 0);
    }

    #[test]
    fn test_deserialize_truncated() {
        let result = StringDictionary::deserialize(&[1, 0, 0, 0]);
        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_invalid_utf8() {
        let bytes = [1u8, 0, 0, 0, 1, 0, 0, 0, 0xFF];
        let result = StringDictionary::deserialize(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_string() {
        let mut dict = StringDictionary::new();
        let id = dict.intern("");
        assert_eq!(dict.lookup(id), Some(""));
        assert_eq!(dict.lookup_id(""), Some(id));
    }

    #[test]
    fn test_compression_integration() {
        let dict = StringDictionary::new();
        let serialized = dict.serialize();
        let chunk = crate::compression::compress(akar_common::enums::CompressionType::StringDictionary, &serialized, 0);
        let decompressed = crate::compression::decompress(&chunk, serialized.len());
        let deserialized = StringDictionary::deserialize(&decompressed).unwrap();
        assert!(deserialized.is_empty());
    }
}
