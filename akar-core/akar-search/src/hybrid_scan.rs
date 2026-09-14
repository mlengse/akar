//! Fused hybrid-scan operator — combines a pre-ranked vector hit list with
//! on-the-fly native BM25 scoring, fused via weighted RRF.
//!
//! This is the *operator* layer of the Tantivy-free fused path: it owns the
//! per-query orchestration that [`crate::fused`] exposes as raw functions.
//! Vector hits are typically produced by an HNSW/ANN scan (`akar-vector`);
//! BM25 scoring happens locally against a [`crate::native_bm25::NativeBm25Index`]
//! with no Tantivy dependency.
//!
//! # Pipeline
//!
//! ```text
//! vector hits (ranked) ──┐
//!                        ├─► execute(query_terms) ─► fused top-k
//! native BM25 index ─────┘
//! ```

use crate::fused::{self, FusedSearchConfig, VectorHit};
use crate::hybrid::SearchResult;
use crate::native_bm25::{Bm25Params, NativeBm25Index};
use crate::rrf::FusedItem;

/// Configure a fused scan over one vector hit list.
#[derive(Debug, Clone, Copy, Default)]
pub struct HybridScanConfig {
    /// Fusion parameters (per-channel weights, RRF k, result limit).
    pub fuse: FusedSearchConfig,
    /// BM25 tuning for the full-text channel.
    pub bm25_params: Bm25Params,
}

/// A ready-to-execute fused scan over one index + one vector hit list.
///
/// Cheap to construct per query; [`HybridScan::execute`] does the BM25 pass
/// and the weighted-RRF fusion.
pub struct HybridScan<'a> {
    bm25: &'a NativeBm25Index,
    vector_results: Vec<SearchResult>,
    config: HybridScanConfig,
}

impl<'a> HybridScan<'a> {
    /// Build a scan from the index, a ranked vector hit list, and config.
    pub fn new(bm25: &'a NativeBm25Index, vector_hits: Vec<VectorHit>, config: HybridScanConfig) -> Self {
        Self {
            bm25,
            vector_results: fused::to_vector_results(vector_hits),
            config,
        }
    }

    /// Run the fused operator for one multi-term query.
    ///
    /// Scores every document in the index against `query_terms` with BM25,
    /// fuses that ranking with the vector hits, and returns the fused top-k.
    pub fn execute(&self, query_terms: &[String]) -> Vec<FusedItem<SearchResult>> {
        let bm25_results = {
            let ranked = self.bm25.score_docs(query_terms, self.config.bm25_params);
            fused::to_bm25_results(ranked)
        };
        fused::fuse_vector_and_bm25(self.vector_results.clone(), bm25_results, self.config.fuse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_bm25::tokenize;

    fn corpus() -> NativeBm25Index {
        NativeBm25Index::built_from(&[
            (1, &tokenize("the cat sat on the mat")),
            (2, &tokenize("the quick brown fox jumps over the lazy dog")),
            (3, &tokenize("rust database indexing engine vector search")),
        ])
    }

    #[test]
    fn test_fused_scan_ranks_vector_hit() {
        let index = corpus();
        let scan = HybridScan::new(
            &index,
            vec![VectorHit { id: 3, score: 0.99 }, VectorHit { id: 1, score: 0.5 }],
            HybridScanConfig::default(),
        );
        let out = scan.execute(&tokenize("rust index"));
        assert_eq!(out[0].item.id, 3, "vector hit should lead: {out:?}");
    }

    #[test]
    fn test_fused_scan_bm25_only_query() {
        let index = corpus();
        // Query "the dog" is a pure lexical hit — the vector list carries no
        // relevant rank-0 candidate, so BM25 must rank docs 1–2 ahead.
        let config = HybridScanConfig {
            fuse: FusedSearchConfig {
                vector_weight: 0.0,
                bm25_weight: 1.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let scan = HybridScan::new(&index, vec![VectorHit { id: 3, score: 0.1 }], config);
        let out = scan.execute(&tokenize("the dog"));
        assert!(!out.is_empty());
        assert!(out[0].item.id == 2, "dog doc should lead: {out:?}");
    }

    #[test]
    fn test_fused_scan_overlap_boost() {
        let index = corpus();
        let scan = HybridScan::new(
            &index,
            vec![VectorHit { id: 3, score: 0.9 }, VectorHit { id: 2, score: 0.2 }],
            HybridScanConfig::default(),
        );
        let out = scan.execute(&tokenize("rust database"));
        assert_eq!(out[0].item.id, 3);
        // Doc 3 matches both query terms AND is rank 0 in the vector list.
        assert!(out[0].rrf_score > out[1].rrf_score);
    }

    #[test]
    fn test_fused_scan_no_vector_hits() {
        let index = corpus();
        let scan = HybridScan::new(&index, vec![], HybridScanConfig::default());
        let out = scan.execute(&tokenize("cat"));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].item.id, 1);
    }

    #[test]
    fn test_fused_scan_limit() {
        let index = corpus();
        let config = HybridScanConfig {
            fuse: FusedSearchConfig {
                limit: 2,
                ..Default::default()
            },
            ..Default::default()
        };
        let scan = HybridScan::new(
            &index,
            vec![
                VectorHit { id: 1, score: 1.0 },
                VectorHit { id: 2, score: 0.9 },
                VectorHit { id: 3, score: 0.8 },
            ],
            config,
        );
        let out = scan.execute(&tokenize("the")); // matches docs 1 & 2 only
        assert!(out.len() <= 2);
    }
}
