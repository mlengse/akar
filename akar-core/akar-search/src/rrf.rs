//! Reciprocal Rank Fusion (RRF) for merging ranked result sets.
//!
//! RRF combines results from multiple ranking sources by accumulating
//! `1 / (K + rank)` scores per item, where `K` is a constant (default 60).
//! Items appearing in more ranked lists receive higher fused scores.

use hashbrown::HashMap;

/// Default RRF constant. Higher values reduce the impact of rank differences.
pub const DEFAULT_K: usize = 60;

/// A single fused result with its RRF score.
#[derive(Debug, Clone)]
pub struct FusedItem<T> {
    pub item: T,
    pub rrf_score: f64,
}

/// Fuse N ranked result sets (owned version).
///
/// Simpler API that takes ownership of the result sets.
pub fn rrf_fuse_owned<T, I, F>(
    sets: Vec<Vec<T>>,
    id_fn: F,
    k: usize,
    limit: usize,
) -> Vec<FusedItem<T>>
where
    I: Eq + std::hash::Hash + Clone,
    F: Fn(&T) -> I,
{
    let mut scores: HashMap<I, f64> = HashMap::new();
    let mut id_to_item: HashMap<I, T> = HashMap::new();

    for set in sets {
        for (rank, item) in set.into_iter().enumerate() {
            let id = id_fn(&item);
            let rrf = 1.0 / (k as f64 + rank as f64 + 1.0);

            *scores.entry(id.clone()).or_insert(0.0) += rrf;
            id_to_item.entry(id).or_insert(item);
        }
    }

    let mut results: Vec<FusedItem<T>> = scores
        .into_iter()
        .filter_map(|(id, score)| {
            id_to_item.remove(&id).map(|item| FusedItem {
                item,
                rrf_score: score,
            })
        })
        .collect();

    results.sort_by(|a, b| b.rrf_score.partial_cmp(&a.rrf_score).unwrap());
    results.truncate(limit);
    results
}

/// Fuse N ranked result sets using borrowed slices with index-based dedup.
///
/// Returns items with their fused RRF scores. Items are deduplicated by
/// the identity extracted via `id_fn`. When the same item appears in
/// multiple sets, its score is accumulated.
///
/// # Returns
/// A vector of `(T, f64)` pairs sorted by RRF score descending.
pub fn rrf_fuse_ref<'a, T, I, F>(
    sets: &'a [Vec<T>],
    id_fn: F,
    k: usize,
    limit: usize,
) -> Vec<(&'a T, f64)>
where
    I: Eq + std::hash::Hash + Clone,
    F: Fn(&T) -> I,
{
    let mut scores: HashMap<I, f64> = HashMap::new();
    let mut first_seen: HashMap<I, (&'a T, usize)> = HashMap::new();
    let mut order: Vec<I> = Vec::new();

    for set in sets {
        for (rank, item) in set.iter().enumerate() {
            let id = id_fn(item);
            let rrf = 1.0 / (k as f64 + rank as f64 + 1.0);

            *scores.entry(id.clone()).or_insert(0.0) += rrf;

            if !first_seen.contains_key(&id) {
                first_seen.insert(id.clone(), (item, order.len()));
                order.push(id);
            }
        }
    }

    let mut results: Vec<(&'a T, f64)> = order
        .into_iter()
        .filter_map(|id| {
            let (item, _) = first_seen.remove(&id)?;
            let score = scores.remove(&id)?;
            Some((item, score))
        })
        .collect();

    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    results.truncate(limit);
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rrf_empty() {
        let result = rrf_fuse_ref::<i32, i32, _>(&[], |x| *x, DEFAULT_K, 10);
        assert!(result.is_empty());
    }

    #[test]
    fn test_rrf_single_set() {
        let a = vec![1, 2, 3];
        let sets = [a];
        let result = rrf_fuse_ref(&sets, |x| *x, DEFAULT_K, 10);
        assert_eq!(result.len(), 3);
        assert_eq!(*result[0].0, 1);
        assert_eq!(*result[1].0, 2);
        assert_eq!(*result[2].0, 3);
    }

    #[test]
    fn test_rrf_two_sets_overlap() {
        let a = vec![1, 2, 3];
        let b = vec![2, 3, 4];
        let sets = [a, b];
        let result = rrf_fuse_ref(&sets, |x| *x, DEFAULT_K, 10);
        // Items 2 and 3 appear in both sets → higher score
        let ids: Vec<i32> = result.iter().map(|(id, _)| **id).collect();
        assert_eq!(ids, vec![2, 3, 1, 4]);
    }

    #[test]
    fn test_rrf_three_sets() {
        let a = vec![1, 2];
        let b = vec![2, 3];
        let c = vec![3, 4];
        let sets = [a, b, c];
        let result = rrf_fuse_ref(&sets, |x| *x, DEFAULT_K, 10);
        // 2 and 3 appear in 2 sets each, 1 and 4 in 1 set
        let ids: Vec<i32> = result.iter().map(|(id, _)| **id).collect();
        assert!(ids[0] == 2 || ids[0] == 3);
        assert!(ids[1] == 2 || ids[1] == 3);
        assert!(ids[2] == 1 || ids[2] == 4);
        assert!(ids[3] == 1 || ids[3] == 4);
    }

    #[test]
    fn test_rrf_dedup_within_set() {
        let a = vec![1, 1, 2];
        let sets = [a];
        let result = rrf_fuse_ref(&sets, |x| *x, DEFAULT_K, 10);
        // Item 1 appears twice → score accumulated
        assert_eq!(result.len(), 2);
        assert_eq!(*result[0].0, 1);
        // Score should be 1/(60+0+1) + 1/(60+1+1) > 1/(60+0+1)
        assert!(result[0].1 > 1.0 / 61.0);
    }

    #[test]
    fn test_rrf_limit() {
        let a = vec![1, 2, 3, 4, 5];
        let sets = [a];
        let result = rrf_fuse_ref(&sets, |x| *x, DEFAULT_K, 3);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_rrf_score_ordering() {
        // a ranks item 20 higher than item 10
        // b ranks item 20 higher than item 10 as well
        // → 20 should be above 10
        let a = vec![20, 10, 30];
        let b = vec![20, 30, 10];
        let sets = [a, b];
        let result = rrf_fuse_ref(&sets, |x| *x, DEFAULT_K, 10);
        // 20: rank 0 in both → highest
        // 10: rank 1 in a + rank 2 in b
        // 30: rank 2 in a + rank 1 in b → ties with 10
        assert_eq!(*result[0].0, 20);
    }

    #[test]
    fn test_rrf_multi_perspective_3_queries() {
        let q1 = vec![1, 2, 3];
        let q2 = vec![2, 3, 4];
        let q3 = vec![3, 4, 5];
        let sets = [q1, q2, q3];
        let result = rrf_fuse_ref(&sets, |x| *x, DEFAULT_K, 10);
        // Item 3 appears in all 3 → highest
        assert_eq!(*result[0].0, 3);
        // Items 2 and 4 appear in 2 sets
        assert!(*result[1].0 == 2 || *result[1].0 == 4);
        assert!(*result[2].0 == 2 || *result[2].0 == 4);
    }
}
