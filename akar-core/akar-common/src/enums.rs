//! Enumeration types used across Akar.

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

/// Path traversal semantics for variable-length path matching.
///
/// Controls what kinds of repetition are allowed when traversing a path:
/// - `WALK`: No restrictions — nodes and edges may repeat.
/// - `TRAIL`: Edges may not repeat (but nodes can).
/// - `ACYCLIC`: Nodes may not repeat (edges automatically won't either).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathSemantic {
    Walk,
    Trail,
    Acyclic,
}

/// Accumulate type for the LogicalAccumulate operator.
///
/// Controls how the accumulate materializes input data:
/// - `Regular`: Standard materialization (all rows are collected).
/// - `Optional`: Used for OPTIONAL MATCH — produces a mark indicating
///   whether at least one row was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccumulateType {
    Regular,
    Optional,
}

/// Edge traversal direction for recursive extend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtendDirection {
    /// Forward — follow outgoing edges.
    Fwd,
    /// Backward — follow incoming edges.
    Bwd,
    /// Both directions.
    Both,
}
