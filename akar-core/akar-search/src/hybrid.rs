//! Hybrid search combining vector similarity and full-text search results.

use crate::rrf::{self, DEFAULT_K, FusedItem};

/// A search result from a single channel (vector or FTS).
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: u64,
    pub score: f64,
    pub channel: &'static str,
}

/// Fuse vector search results and FTS results using RRF.
///
/// # Arguments
/// * `vector_results` — Results from vector similarity search, ranked by score.
/// * `fts_results` — Results from full-text search, ranked by relevance.
/// * `limit` — Maximum results to return.
pub fn hybrid_search(
    vector_results: Vec<SearchResult>,
    fts_results: Vec<SearchResult>,
    limit: usize,
) -> Vec<FusedItem<SearchResult>> {
    rrf::rrf_fuse_owned(vec![vector_results, fts_results], |r| r.id, DEFAULT_K, limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hybrid_empty() {
        let result = hybrid_search(vec![], vec![], 10);
        assert!(result.is_empty());
    }

    #[test]
    fn test_hybrid_vector_only() {
        let vec_results = vec![
            SearchResult {
                id: 1,
                score: 0.9,
                channel: "vector",
            },
            SearchResult {
                id: 2,
                score: 0.8,
                channel: "vector",
            },
        ];
        let result = hybrid_search(vec_results, vec![], 10);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].item.id, 1);
    }

    #[test]
    fn test_hybrid_fts_only() {
        let fts_results = vec![
            SearchResult {
                id: 3,
                score: 1.0,
                channel: "fts",
            },
            SearchResult {
                id: 4,
                score: 0.5,
                channel: "fts",
            },
        ];
        let result = hybrid_search(vec![], fts_results, 10);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].item.id, 3);
    }

    #[test]
    fn test_hybrid_overlapping() {
        let vec_results = vec![
            SearchResult {
                id: 1,
                score: 0.9,
                channel: "vector",
            },
            SearchResult {
                id: 2,
                score: 0.8,
                channel: "vector",
            },
        ];
        let fts_results = vec![
            SearchResult {
                id: 2,
                score: 1.0,
                channel: "fts",
            },
            SearchResult {
                id: 3,
                score: 0.7,
                channel: "fts",
            },
        ];
        let result = hybrid_search(vec_results, fts_results, 10);
        assert_eq!(result.len(), 3);
        // Item 2 appears in both → highest RRF score
        assert_eq!(result[0].item.id, 2);
    }

    #[test]
    fn test_hybrid_limit() {
        let vec_results = (0..20)
            .map(|i| SearchResult {
                id: i,
                score: 1.0 - i as f64 * 0.01,
                channel: "vector",
            })
            .collect();
        let result = hybrid_search(vec_results, vec![], 5);
        assert_eq!(result.len(), 5);
    }
}
