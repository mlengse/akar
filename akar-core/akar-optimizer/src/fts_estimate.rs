// ========================================================================
// Trait: FTS Cardinality Estimator
// Provider interface for FTS selectivity estimation at optimisation time.
// Implementations live behind the `fts-extension` feature flag in akar-main.
// ========================================================================

/// Estimate the number of documents matching an FTS query in a Tantivy index.
///
/// The optimizer uses this to avoid overestimating scan cardinality for
/// `USING FTS INDEX` queries: instead of assuming the entire source table
/// is scanned, the cardinality pass takes `min(table_rows, estimated_matches)`.
///
/// Returns `None` on any failure (missing index, parse error, etc.) — the
/// optimizer falls back to the full table cardinality.
pub trait FtsCardinalityEstimator: Send + Sync {
    /// Estimate how many documents in the index named `index_name` match
    /// `query` against `column_name`.
    fn estimate_match_count(&self, index_name: &str, column_name: &str, query: &str) -> Option<u64>;
}
