//! QueryResult — encapsulates the result of a query execution.

use kuzu_common::vector::DataChunk;

/// The result of executing a query.
#[derive(Debug)]
pub struct QueryResult {
    pub chunks: Vec<DataChunk>,
    pub num_rows: usize,
    pub num_columns: usize,
    pub success: bool,
    pub error_message: Option<String>,
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
        }
    }

    pub fn error(msg: String) -> Self {
        Self {
            chunks: Vec::new(),
            num_rows: 0,
            num_columns: 0,
            success: false,
            error_message: Some(msg),
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
}
