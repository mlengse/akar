//! Hierarchical multi-vector RRF and authority re-weighting.
//!
//! The memory-recall stack ranks a candidate through more than one signal:
//!
//! | level | signal | what it answers |
//! |-------|--------|-----------------|
//! | L0 | summary vector | "is this memory about the right thing at all?" |
//! | L1 | content vector | "does it actually say the right thing?" |
//! | — | BM25 keyword score | "does it use the queried words?" |
//!
//! [`fuse_hierarchical`] merges all three with per-level weights, so the coarse
//! L0 level can be given more say than the fine L1 level — a candidate whose
//! summary matches is a stronger signal than one whose wording happens to match.
//!
//! [`apply_authority`] then re-weights the fused ranking by a per-item authority
//! score **before** the limit is applied, so a well-trusted item can displace a
//! better-ranked but less trusted one. [`fuse_hierarchical_with_authority`]
//! chains both steps in the order that guarantees that.

use crate::hybrid::SearchResult;
use crate::rrf::{self, DEFAULT_K, FusedItem};

/// Per-level weights for the three-stream hierarchical retrieval stack.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HierarchicalRrfConfig {
    /// Weight of the L0 summary-vector channel.
    pub l0_weight: f64,
    /// Weight of the L1 content-vector channel.
    pub l1_weight: f64,
    /// Weight of the BM25 keyword channel.
    pub bm25_weight: f64,
    /// RRF constant (higher = flatter rank influence).
    pub rrf_k: usize,
    /// Maximum number of fused results to return.
    pub limit: usize,
}

impl Default for HierarchicalRrfConfig {
    fn default() -> Self {
        Self {
            // L0 leads: a summary hit is a coarser but more reliable signal than
            // a content hit, which keyword overlap can trigger by accident.
            l0_weight: 2.0,
            l1_weight: 1.0,
            bm25_weight: 1.0,
            rrf_k: DEFAULT_K,
            limit: 10,
        }
    }
}

/// Re-tag a ranked list's channel label.
///
/// [`fuse_hierarchical`] keeps the *first* channel a given id appeared in as the
/// representative item, so labelling each input list makes the level an id won
/// on visible in the fused output.
pub fn tag_channel(results: Vec<SearchResult>, channel: &'static str) -> Vec<SearchResult> {
    results
        .into_iter()
        .map(|result| SearchResult { channel, ..result })
        .collect()
}

/// Fuse the three hierarchical retrieval streams with per-level weights.
///
/// Each input list must be **pre-ranked** (index order = rank, best first).
/// Results are sorted by descending fused score and truncated to
/// `config.limit`; ids appearing in several channels accumulate score, which is
/// what makes a memory that every level agrees on rank first. Empty streams
/// contribute nothing.
pub fn fuse_hierarchical(
    l0_results: Vec<SearchResult>,
    l1_results: Vec<SearchResult>,
    bm25_results: Vec<SearchResult>,
    config: HierarchicalRrfConfig,
) -> Vec<FusedItem<SearchResult>> {
    let sets = vec![
        (l0_results, config.l0_weight),
        (l1_results, config.l1_weight),
        (bm25_results, config.bm25_weight),
    ];
    rrf::weighted_rrf_fuse(sets, |result| result.id, config.rrf_k, config.limit)
}

/// How far an item's authority is allowed to move it in the fused ranking.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AuthorityConfig {
    /// Multiplier applied to an item whose authority is `0.0`.
    pub floor_multiplier: f64,
    /// Multiplier applied to an item whose authority is `1.0` or above.
    pub ceiling_multiplier: f64,
}

impl Default for AuthorityConfig {
    fn default() -> Self {
        Self {
            floor_multiplier: 0.5,
            ceiling_multiplier: 1.5,
        }
    }
}

/// Map an authority score to a score multiplier.
///
/// Authority is expected on a `0.0..=1.0` scale and is clamped to it, so an
/// out-of-range input degrades to "least trusted" or "most trusted" rather than
/// inverting the ranking. `0.5` is neutral — it maps to exactly `1.0`.
pub fn authority_multiplier(authority: f64, config: &AuthorityConfig) -> f64 {
    let authority = authority.clamp(0.0, 1.0);
    config.floor_multiplier + (config.ceiling_multiplier - config.floor_multiplier) * authority
}

/// Re-weight a fused ranking by authority, then sort and truncate.
///
/// `authority_of` returns the item's authority on the `0.0..=1.0` scale (see
/// [`authority_multiplier`]); map labels or provenance onto that scale in the
/// closure to express label-based re-weighting. Because the limit is applied
/// last, an item outside the fused top-k can still be promoted into it.
pub fn apply_authority<T, F>(
    items: Vec<FusedItem<T>>,
    authority_of: F,
    config: &AuthorityConfig,
    limit: usize,
) -> Vec<FusedItem<T>>
where
    F: Fn(&T) -> f64,
{
    let mut items = items;
    for item in &mut items {
        item.rrf_score *= authority_multiplier(authority_of(&item.item), config);
    }
    items.sort_by(|a, b| b.rrf_score.total_cmp(&a.rrf_score));
    items.truncate(limit);
    items
}

/// Fuse the hierarchical streams, re-weight by authority, then take `limit`.
///
/// Equivalent to [`fuse_hierarchical`] with an unbounded limit followed by
/// [`apply_authority`] with `config.limit`: authority is applied *before* the
/// cut, so it can promote an item the raw fusion ranked outside the top-k.
/// (`fuse_hierarchical` alone would drop that item before authority ever saw
/// it — that ordering bug is what this function exists to prevent.)
pub fn fuse_hierarchical_with_authority<F>(
    l0_results: Vec<SearchResult>,
    l1_results: Vec<SearchResult>,
    bm25_results: Vec<SearchResult>,
    config: HierarchicalRrfConfig,
    authority_of: F,
    authority: &AuthorityConfig,
) -> Vec<FusedItem<SearchResult>>
where
    F: Fn(&SearchResult) -> f64,
{
    let fuse_all = HierarchicalRrfConfig {
        limit: usize::MAX,
        ..config
    };
    let fused = fuse_hierarchical(l0_results, l1_results, bm25_results, fuse_all);
    apply_authority(fused, authority_of, authority, config.limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranked(id: u64, channel: &'static str) -> SearchResult {
        SearchResult {
            id,
            score: 1.0,
            channel,
        }
    }

    fn ranked_all(ids: &[u64], channel: &'static str) -> Vec<SearchResult> {
        ids.iter().map(|&id| ranked(id, channel)).collect()
    }

    fn ids(items: &[FusedItem<SearchResult>]) -> Vec<u64> {
        items.iter().map(|item| item.item.id).collect()
    }

    #[test]
    fn defaults_let_the_summary_level_lead() {
        let config = HierarchicalRrfConfig::default();
        assert!(config.l0_weight > config.l1_weight, "{config:?}");
        assert_eq!(config.rrf_k, DEFAULT_K);
    }

    #[test]
    fn l0_outranks_l1_at_equal_rank() {
        // Two distinct ids, each rank 0 in its own channel: the L0 one wins.
        let out = fuse_hierarchical(
            vec![ranked(1, "vector")],
            vec![ranked(2, "vector")],
            vec![],
            HierarchicalRrfConfig::default(),
        );
        assert_eq!(ids(&out), vec![1, 2]);
    }

    #[test]
    fn agreement_across_levels_wins() {
        // Id 2 is rank 1 in both vector levels; id 1 leads L0 alone.
        let out = fuse_hierarchical(
            ranked_all(&[1, 2], "vector"),
            ranked_all(&[2], "vector"),
            vec![],
            HierarchicalRrfConfig::default(),
        );
        assert_eq!(
            out[0].item.id,
            2,
            "the id both levels agree on must lead: {:?}",
            ids(&out)
        );
    }

    #[test]
    fn bm25_only_stream_still_contributes() {
        let out = fuse_hierarchical(
            vec![],
            vec![],
            ranked_all(&[7], "fts"),
            HierarchicalRrfConfig::default(),
        );
        assert_eq!(ids(&out), vec![7]);
    }

    #[test]
    fn empty_streams_fuse_to_nothing() {
        let out = fuse_hierarchical(vec![], vec![], vec![], HierarchicalRrfConfig::default());
        assert!(out.is_empty());
    }

    #[test]
    fn limit_is_respected() {
        let out = fuse_hierarchical(
            ranked_all(&[1, 2, 3, 4], "vector"),
            vec![],
            vec![],
            HierarchicalRrfConfig {
                limit: 2,
                ..Default::default()
            },
        );
        assert_eq!(ids(&out), vec![1, 2]);
    }

    #[test]
    fn tag_channel_relabels_without_touching_rank_or_score() {
        let out = tag_channel(ranked_all(&[3, 4], "vector"), "vector_l0");
        assert_eq!(out[0].id, 3);
        assert_eq!(out[1].id, 4);
        assert!(out.iter().all(|result| result.channel == "vector_l0"));
        assert!(out.iter().all(|result| result.score == 1.0));
    }

    #[test]
    fn authority_multiplier_is_neutral_at_the_midpoint() {
        let config = AuthorityConfig::default();
        assert!((authority_multiplier(0.5, &config) - 1.0).abs() < 1e-12);
        assert!((authority_multiplier(0.0, &config) - 0.5).abs() < 1e-12);
        assert!((authority_multiplier(1.0, &config) - 1.5).abs() < 1e-12);
    }

    #[test]
    fn authority_multiplier_clamps_out_of_range_input() {
        let config = AuthorityConfig::default();
        assert!((authority_multiplier(-3.0, &config) - 0.5).abs() < 1e-12);
        assert!((authority_multiplier(9.0, &config) - 1.5).abs() < 1e-12);
    }

    #[test]
    fn authority_promotes_a_lower_ranked_item() {
        let fused = vec![
            FusedItem {
                item: ranked(1, "vector"),
                rrf_score: 1.0 / 61.0,
            },
            FusedItem {
                item: ranked(2, "vector"),
                rrf_score: 1.0 / 62.0,
            },
        ];
        let authority = |result: &SearchResult| if result.id == 2 { 1.0 } else { 0.0 };
        let out = apply_authority(fused, authority, &AuthorityConfig::default(), 10);
        assert_eq!(out[0].item.id, 2, "the trusted item must overtake: {:?}", ids(&out));
    }

    #[test]
    fn authority_uses_the_original_order_to_break_ties() {
        // Neutral multipliers must leave the fused order exactly as it was.
        let fused = vec![
            FusedItem {
                item: ranked(1, "vector"),
                rrf_score: 0.9,
            },
            FusedItem {
                item: ranked(2, "vector"),
                rrf_score: 0.9,
            },
        ];
        let neutral = AuthorityConfig {
            floor_multiplier: 1.0,
            ceiling_multiplier: 1.0,
        };
        let out = apply_authority(fused, |_| 0.0, &neutral, 10);
        assert_eq!(ids(&out), vec![1, 2]);
    }

    #[test]
    fn authority_truncates_after_re_weighting() {
        let fused = vec![
            FusedItem {
                item: ranked(1, "vector"),
                rrf_score: 1.0 / 61.0,
            },
            FusedItem {
                item: ranked(2, "vector"),
                rrf_score: 1.0 / 62.0,
            },
        ];
        let authority = |result: &SearchResult| if result.id == 2 { 1.0 } else { 0.0 };
        let out = apply_authority(fused, authority, &AuthorityConfig::default(), 1);
        assert_eq!(ids(&out), vec![2], "the limit must cut after the re-weighting");
    }

    #[test]
    fn authority_is_applied_before_the_limit() {
        // Regression guard for the ordering: id 2 loses the raw fusion badly
        // enough to fall outside `limit = 1`, yet must still be promotable.
        let config = HierarchicalRrfConfig {
            limit: 1,
            ..Default::default()
        };
        let l0 = vec![ranked(1, "vector")];
        let l1 = vec![ranked(2, "vector")];
        let authority = |result: &SearchResult| if result.id == 2 { 1.0 } else { 0.0 };

        let raw = fuse_hierarchical(
            l0.clone(),
            l1.clone(),
            vec![],
            HierarchicalRrfConfig {
                limit: usize::MAX,
                ..config
            },
        );
        assert_eq!(ids(&raw), vec![1, 2], "id 1 leads the raw fusion");

        let ranked_limit = fuse_hierarchical(l0.clone(), l1.clone(), vec![], config);
        assert_eq!(
            ids(&ranked_limit),
            vec![1],
            "a plain limited fusion would never see id 2"
        );

        let out = fuse_hierarchical_with_authority(l0, l1, vec![], config, authority, &AuthorityConfig::default());
        assert_eq!(ids(&out), vec![2], "authority must be able to promote past the limit");
    }

    #[test]
    fn authority_re_weights_the_fused_score_in_place() {
        let fused = vec![FusedItem {
            item: ranked(1, "vector"),
            rrf_score: 0.02,
        }];
        let out = apply_authority(fused, |_| 1.0, &AuthorityConfig::default(), 10);
        assert!((out[0].rrf_score - 0.03).abs() < 1e-12, "{:?}", out[0].rrf_score);
    }
}
