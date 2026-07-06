//! QueryResult — encapsulates the result of a query execution.

use kuzu_common::vector::DataChunk;
use std::fmt;
use std::time::Duration;

/// Timing summary for a query execution.
#[derive(Debug, Clone)]
pub struct QuerySummary {
    /// Total wall-clock time from query submission to result.
    pub elapsed: Duration,
    /// Time spent in compilation (parse + bind + plan + optimize).
    pub compile_time: Duration,
    /// Time spent in execution (physical operator execution).
    pub execution_time: Duration,
}

impl Default for QuerySummary {
    fn default() -> Self {
        Self {
            elapsed: Duration::ZERO,
            compile_time: Duration::ZERO,
            execution_time: Duration::ZERO,
        }
    }
}

impl fmt::Display for QuerySummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Query executed in {:.2}ms (compile: {:.2}ms, execution: {:.2}ms)",
            self.elapsed.as_secs_f64() * 1000.0,
            self.compile_time.as_secs_f64() * 1000.0,
            self.execution_time.as_secs_f64() * 1000.0,
        )
    }
}

/// The result of executing a Cypher query.
///
/// Contains result chunks as column-major [`DataChunk`] vectors, metadata
/// (row/column counts), and optional timing summary.
///
/// # Examples
///
/// ```no_run
/// # use kuzu_main::database::{Database, SystemConfig};
/// # use kuzu_main::connection::Connection;
/// # let db = Database::new("./db", SystemConfig::default())?;
/// # let conn = Connection::new(&db);
/// let result = conn.query("MATCH (n) RETURN n LIMIT 5")?;
/// println!("Rows: {}, Columns: {}", result.num_rows, result.num_columns);
/// for chunk in &result.chunks {
///     for field in &chunk.fields {
///         for row in 0..field.size() {
///             if let Some(val) = field.get_value(row) {
///                 println!("  {:?}", val);
///             }
///         }
///     }
/// }
/// # Ok::<(), String>(())
/// ```
#[derive(Debug, Clone)]
pub struct QueryResult {
    pub chunks: Vec<DataChunk>,
    pub num_rows: usize,
    pub num_columns: usize,
    pub success: bool,
    pub error_message: Option<String>,
    /// Human-readable message (e.g. "Table created" for DDL).
    pub message: Option<String>,
    /// Timing summary for the query.
    pub summary: Option<QuerySummary>,
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
            summary: None,
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
            summary: None,
        }
    }

    /// Attach timing summary to this result.
    pub fn with_summary(mut self, summary: QuerySummary) -> Self {
        self.summary = Some(summary);
        self
    }

    pub fn error(msg: String) -> Self {
        Self {
            chunks: Vec::new(),
            num_rows: 0,
            num_columns: 0,
            success: false,
            error_message: Some(msg),
            message: None,
            summary: None,
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
    pub fn result_summary(&self) -> String {
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
