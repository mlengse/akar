//! Enumeration types used across Kuzu.

/// Compression algorithm types for column storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionType {
    Uncompressed,
    Constant,
    OneValue,
    Boolean,
    IntegerBitpacking,
    StringDictionary,
    Float,
    ListDelta,
}

/// Transaction action type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionAction {
    BeginRead,
    BeginWrite,
    Commit,
    Rollback,
    Checkpoint,
}
