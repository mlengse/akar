// ========================================================================
// FTS cardinality estimator (P108.2)
// Bridges the optimizer's FtsCardinalityEstimator trait to Tantivy's
// term statistics. The concrete Tantivy-backed implementation is compiled
// only when the `fts-extension` feature is enabled; otherwise the factory
// returns `None` and scans with an `fts_query` are estimated at their full
// table cardinality.
// ========================================================================

use akar_optimizer::fts_estimate::FtsCardinalityEstimator;
use akar_storage::TableCatalog;
use std::sync::Arc;

#[cfg(feature = "fts-extension")]
mod inner {
    use super::*;

    pub(crate) struct TantivyFtsEstimator {
        table_catalog: Arc<TableCatalog>,
    }

    impl TantivyFtsEstimator {
        pub(crate) fn new(table_catalog: Arc<TableCatalog>) -> Self {
            Self { table_catalog }
        }
    }

    impl FtsCardinalityEstimator for TantivyFtsEstimator {
        fn estimate_match_count(&self, index_name: &str, column_name: &str, query: &str) -> Option<u64> {
            // The FTS index lives at <db_path>/fts/<index_name>; runtime_handle
            // opens it lazily (sharing ONE handle + cached reader with the
            // commit sync hook and the physical scan — P107.2).
            let index_dir = self.table_catalog.db_path()?.join("fts").join(index_name);
            let handle = akar_fts::index::runtime_handle(&self.table_catalog, index_name, index_dir).ok()?;
            handle.estimate_match_count(column_name, query).ok()
        }
    }
}

/// Build the FTS cardinality estimator to hand to the optimizer.
///
/// Returns `None` when the `fts-extension` feature is disabled (or the index
/// cannot be resolved at optimisation time) — callers fall back to full table
/// cardinality.
pub(crate) fn build(table_catalog: Arc<TableCatalog>) -> Option<Arc<dyn FtsCardinalityEstimator>> {
    #[cfg(feature = "fts-extension")]
    {
        Some(Arc::new(inner::TantivyFtsEstimator::new(table_catalog)))
    }
    #[cfg(not(feature = "fts-extension"))]
    {
        let _ = table_catalog;
        None
    }
}
