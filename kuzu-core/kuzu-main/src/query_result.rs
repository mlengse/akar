//! QueryResult — encapsulates the result of a query execution.

use kuzu_common::vector::DataChunk;
use std::fmt;

/// The result of executing a query.
#[derive(Debug, Clone)]
pub struct QueryResult {
    pub chunks: Vec<DataChunk>,
    pub num_rows: usize,
    pub num_columns: usize,
    pub success: bool,
    pub error_message: Option<String>,
    /// Human-readable message (e.g. "Table created" for DDL).
    pub message: Option<String>,
}

impl QueryResult {
    pub fn new(chunks: Vec<DataChunk>) -> Self {
        let num_rows = chunks.iter().map(|c| c.size).sum();
        let num_columns = chunks.first().map(|c| c.num_fields()).unwrap_or(0);
        Self {
            chunks,
            num_rows,
            num_columns,
            success: true,
            error_message: None,
            message: None,
        }
    }

    /// Create a success result with a human-readable message (no data chunks).
    pub fn success_message(msg: String) -> Self {
        Self {
            chunks: Vec::new(),
            num_rows: 0,
            num_columns: 0,
            success: true,
            error_message: None,
            message: Some(msg),
        }
    }

    pub fn error(msg: String) -> Self {
        Self {
            chunks: Vec::new(),
            num_rows: 0,
            num_columns: 0,
            success: false,
            error_message: Some(msg),
            message: None,
        }
    }

    pub fn is_success(&self) -> bool {
        self.success
    }

    pub fn num_rows(&self) -> usize {
        self.num_rows
    }

    pub fn num_columns(&self) -> usize {
        self.num_columns
    }

    /// Get a human-readable summary of the result.
    pub fn summary(&self) -> String {
        if let Some(msg) = &self.message {
            return msg.clone();
        }
        if !self.success {
            return format!("Error: {}", self.error_message.as_deref().unwrap_or("Unknown error"));
        }
        if self.num_rows == 0 {
            return "(empty result)".into();
        }
        format!("Returned {} rows in {} columns", self.num_rows, self.num_columns)
    }
}

impl fmt::Display for QueryResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(msg) = &self.message {
            return write!(f, "{msg}");
        }
        if !self.success {
            return write!(f, "Error: {}", self.error_message.as_deref().unwrap_or("Unknown error"));
        }
        if self.chunks.is_empty() {
            return write!(f, "(empty result)");
        }
        for (i, chunk) in self.chunks.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "Chunk {}: {} rows, {} columns", i, chunk.size, chunk.num_fields())?;
        }
        Ok(())
    }
}
