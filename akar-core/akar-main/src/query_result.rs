//! QueryResult — encapsulates the result of a query execution.

use akar_common::vector::DataChunk;
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
/// # use akar_main::database::{Database, SystemConfig};
/// # use akar_main::connection::Connection;
/// # let db = std::sync::Arc::new(Database::new("./db", SystemConfig::default())?);
/// # let conn = Connection::new(&db);
/// let result = conn.query("MATCH (n) RETURN n LIMIT 5")?;
/// println!("Rows: {}, Columns: {}", result.num_rows, result.num_columns);
/// for chunk in &result.chunks {
///     for field_idx in 0..chunk.fields.len() {
///         for row in 0..chunk.size {
///             if let Some(val) = chunk.get_value(field_idx, row) {
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
        if let Some(head) = result_summary_head(
            self.message.as_deref(),
            self.success,
            self.error_message.as_deref(),
            self.num_rows != 0,
        ) {
            return head;
        }
        format!("Returned {} rows in {} columns", self.num_rows, self.num_columns)
    }
}

/// Shared head logic for result summaries (local [`QueryResult`] and remote
/// [`crate::remote::WireResponse`], DRY P51.43).
///
/// Precedence: message → error → empty-result marker. Returns `Some(summary)`
/// when the result carries no data rows; `None` means the caller should append
/// its row-count summary (or render rows).
pub(crate) fn result_summary_head(
    message: Option<&str>,
    success: bool,
    error_message: Option<&str>,
    has_rows: bool,
) -> Option<String> {
    if let Some(msg) = message {
        return Some(msg.to_string());
    }
    if !success {
        return Some(format!("Error: {}", error_message.unwrap_or("Unknown error")));
    }
    if !has_rows {
        return Some("(empty result)".into());
    }
    None
}

impl fmt::Display for QueryResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(head) = result_summary_head(
            self.message.as_deref(),
            self.success,
            self.error_message.as_deref(),
            !self.chunks.is_empty(),
        ) {
            return write!(f, "{head}");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_result_summary_cases() {
        // Message takes precedence over data.
        let mut r = QueryResult::new(Vec::new());
        r.message = Some("Table created".into());
        assert_eq!(r.result_summary(), "Table created");

        // Error rendering.
        let err = QueryResult::error("boom".into());
        assert_eq!(err.result_summary(), "Error: boom");

        // Empty result marker.
        assert_eq!(QueryResult::new(Vec::new()).result_summary(), "(empty result)");

        // Row-count summary.
        let mut rows = QueryResult::new(Vec::new());
        rows.num_rows = 3;
        rows.num_columns = 2;
        assert_eq!(rows.result_summary(), "Returned 3 rows in 2 columns");
    }

    fn remote(
        success: bool,
        message: Option<&str>,
        error: Option<&str>,
        columns: usize,
        row_count: usize,
    ) -> crate::remote::WireResponse {
        crate::remote::WireResponse {
            success,
            message: message.map(str::to_string),
            error_message: error.map(str::to_string),
            column_names: (0..columns).map(|i| format!("c{i}")).collect(),
            rows: vec![Vec::new(); row_count],
            stats: None,
        }
    }

    /// Local and remote results must produce byte-identical summaries for the
    /// same logical outcome (P51.43 — single shared head logic).
    #[test]
    fn test_local_remote_summary_parity() {
        // Message case.
        assert_eq!(
            QueryResult::success_message("Table created".into()).result_summary(),
            remote(true, Some("Table created"), None, 0, 0).result_summary()
        );
        // Error case.
        assert_eq!(
            QueryResult::error("boom".into()).result_summary(),
            remote(false, None, Some("boom"), 0, 0).result_summary()
        );
        // Empty case.
        assert_eq!(
            QueryResult::new(Vec::new()).result_summary(),
            remote(true, None, None, 0, 0).result_summary()
        );
        // Data case.
        let mut local = QueryResult::new(Vec::new());
        local.num_rows = 2;
        local.num_columns = 3;
        assert_eq!(local.result_summary(), remote(true, None, None, 3, 2).result_summary());
    }
}
