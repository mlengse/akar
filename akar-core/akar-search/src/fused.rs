//! Fused hybrid scoring — combines vector similarity and native BM25 rankings
//! with a single weighted RRF pass.
//!
//! This is the Tantivy-free fused path ([`crate::native_bm25`] supplies the
//! full-text side; vector scores come from `akar-vector`). Results are fused
//! with [`crate::rrf::weighted_rrf_fuse`], so per-channel weights express
//! channel importance and overlap between channels is rewarded exactly like
//! the existing RRF machinery.
//!
//! # Integration
//!
//! ```text
//! vector top-k  ─┐
//!                ├─► weighted_rrf_fuse ─► fused top-k (SearchResult)
//! native BM25   ─┘      (per-channel weights)
//! ```

use crate::hybrid::SearchResult;
use crate::rrf::{self, DEFAULT_K, FusedItem};

/// Recommended defaults for the fused path (balanced).
#[derive(Debug, Clone, Copy)]
pub struct FusedSearchConfig {
    /// Weight for the full-text (BM25) channel in RRF.
    pub bm25_weight: f64,
    /// Weight for the vector channel in RRF.
    pub vector_weight: f64,
    /// RRF constant (higher = flatter rank influence).
    pub rrf_k: usize,
    /// Maximum number of fused results to return.
    pub limit: usize,
}

impl Default for FusedSearchConfig {
    fn default() -> Self {
        Self {
            bm25_weight: 0.5,
            vector_weight: 0.5,
            rrf_k: DEFAULT_K,
            limit: 10,
        }
    }
}

/// Fuse a vector ranking and a native-BM25 ranking into one ranked list.
///
/// `vector_results` must be pre-ranked (index order = rank). Returns fused
/// items sorted by descending RRF score, truncated to `config.limit`.
/// Items appearing in both channels win via score accumulation.
pub fn fuse_vector_and_bm25(
    vector_results: Vec<SearchResult>,
    bm25_results: Vec<SearchResult>,
    config: FusedSearchConfig,
) -> Vec<FusedItem<SearchResult>> {
    let sets = vec![
        (bm25_results, config.bm25_weight),
        (vector_results, config.vector_weight),
    ];
    rrf::weighted_rrf_fuse(sets, |r| r.id, config.rrf_k, config.limit)
}

/// One vector result paired with its raw similarity, ready to be ranked.
#[derive(Debug, Clone)]
pub struct VectorHit {
    pub id: u64,
    /// Raw similarity (higher = more similar) — used for ordering only.
    pub score: f64,
}

/// Wrap vector hits as ranked [`SearchResult`]s (descending similarity).
pub fn to_vector_results(hits: Vec<VectorHit>) -> Vec<SearchResult> {
    let mut hits = hits;
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    hits.into_iter()
        .map(|h| SearchResult {
            id: h.id,
            score: h.score,
            channel: "vector",
        })
        .collect()
}

/// Wrap raw `(id, score)` BM25 pairs (already descending) as [`SearchResult`]s.
pub fn to_bm25_results(ranked: Vec<(u64, f64)>) -> Vec<SearchResult> {
    ranked
        .into_iter()
        .map(|(id, score)| SearchResult {
            id,
            score,
            channel: "fts",
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hybrid::hybrid_search;

    #[test]
    fn test_fuse_empty() {
        let out = fuse_vector_and_bm25(vec![], vec![], FusedSearchConfig::default());
        assert!(out.is_empty());
    }

    #[test]
    fn test_fuse_overlap_ranks_first() {
        // Doc 2 in both channels should beat anything in a single channel.
        let vector_results =
            to_vector_results(vec![VectorHit { id: 1, score: 0.95 }, VectorHit { id: 2, score: 0.90 }]);
        let bm25_results = to_bm25_results(vec![(2, 8.0), (3, 7.0)]);
        let out = fuse_vector_and_bm25(vector_results, bm25_results, FusedSearchConfig::default());
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].item.id, 2);
    }

    #[test]
    fn test_fuse_weights_dominate_channel() {
        // Vector-heavy: doc 4 (vector rank 0) beats doc 5 (bm25 rank 0).
        let vector_heavy = FusedSearchConfig {
            vector_weight: 2.0,
            bm25_weight: 0.5,
            ..Default::default()
        };
        let bm25_heavy = FusedSearchConfig {
            vector_weight: 0.5,
            bm25_weight: 2.0,
            ..Default::default()
        };
        let run = |config: FusedSearchConfig| {
            fuse_vector_and_bm25(
                to_vector_results(vec![VectorHit { id: 4, score: 1.0 }]),
                to_bm25_results(vec![(5, 99.0)]),
                config,
            )
            .into_iter()
            .next()
            .map(|f| f.item.id)
        };
        assert_eq!(run(vector_heavy), Some(4));
        assert_eq!(run(bm25_heavy), Some(5));
    }

    #[test]
    fn test_unweighted_matches_existing_hybrid() {
        let vector_results = to_vector_results(vec![VectorHit { id: 1, score: 0.9 }, VectorHit { id: 2, score: 0.8 }]);
        let bm25_results = to_bm25_results(vec![(2, 8.0), (3, 6.0)]);
        let config = FusedSearchConfig {
            vector_weight: 1.0,
            bm25_weight: 1.0,
            ..Default::default()
        };
        let fused = fuse_vector_and_bm25(vector_results.clone(), bm25_results.clone(), config);

        // Reuse the existing unweighted RRF path as an oracle.
        let legacy = hybrid_search(vector_results, bm25_results, 10);
        assert_eq!(fused.len(), legacy.len());
        for (f, l) in fused.iter().zip(legacy.iter()) {
            assert_eq!(f.item.id, l.item.id);
            assert!((f.rrf_score - l.rrf_score).abs() < 1e-12);
        }
    }

    #[test]
    fn test_limit_respected() {
        let vector_results = to_vector_results(vec![
            VectorHit { id: 1, score: 1.0 },
            VectorHit { id: 2, score: 0.9 },
            VectorHit { id: 3, score: 0.8 },
        ]);
        let config = FusedSearchConfig {
            limit: 2,
            ..Default::default()
        };
        let out = fuse_vector_and_bm25(vector_results, vec![], config);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn test_conversions_preserve_channel_tag() {
        let v = to_vector_results(vec![VectorHit { id: 7, score: 0.5 }]);
        assert_eq!(v[0].channel, "vector");
        let b = to_bm25_results(vec![(7, 3.0)]);
        assert_eq!(b[0].channel, "fts");
    }
}
