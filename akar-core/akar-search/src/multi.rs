//! Multi-perspective recall: run N queries and fuse results via RRF.

use crate::rrf::{self, FusedItem};

/// Execute multiple search queries and fuse results with explicit id extraction.
pub fn multi_perspective_recall_with_id<T, I, F, S>(
    queries: &[&str],
    search_fn: S,
    id_fn: F,
    k: usize,
    limit: usize,
) -> Vec<FusedItem<T>>
where
    I: Eq + std::hash::Hash + Clone,
    F: Fn(&T) -> I,
    S: Fn(&str) -> Vec<T>,
{
    let sets: Vec<Vec<T>> = queries.iter().map(|q| search_fn(q)).collect();
    rrf::rrf_fuse_owned(sets, id_fn, k, limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rrf::DEFAULT_K;

    #[test]
    fn test_multi_perspective_empty() {
        let result: Vec<FusedItem<i32>> = multi_perspective_recall_with_id(&[], |_q| vec![], |x| *x, DEFAULT_K, 10);
        assert!(result.is_empty());
    }

    #[test]
    fn test_multi_perspective_single_query() {
        let result = multi_perspective_recall_with_id(
            &["hello"],
            |q| {
                if q == "hello" { vec![1, 2, 3] } else { vec![] }
            },
            |x| *x,
            DEFAULT_K,
            10,
        );
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].item, 1);
    }

    #[test]
    fn test_multi_perspective_3_queries() {
        let search = |q: &str| -> Vec<i32> {
            match q {
                "rust" => vec![1, 2, 3],
                "graph" => vec![2, 3, 4],
                "database" => vec![3, 4, 5],
                _ => vec![],
            }
        };
        let result = multi_perspective_recall_with_id(&["rust", "graph", "database"], search, |x| *x, DEFAULT_K, 10);
        // Item 3 appears in all 3 → highest
        assert_eq!(result[0].item, 3);
        // Items 2 and 4 appear in 2 sets
        assert!(result[1].item == 2 || result[1].item == 4);
    }

    #[test]
    fn test_multi_perspective_fuse_dedup() {
        let search = |q: &str| -> Vec<String> {
            match q {
                "a" => vec!["x".into(), "y".into()],
                "b" => vec!["y".into(), "z".into()],
                _ => vec![],
            }
        };
        let result = multi_perspective_recall_with_id(&["a", "b"], search, |s: &String| s.clone(), DEFAULT_K, 10);
        assert_eq!(result.len(), 3);
        // "y" appears in both → highest
        assert_eq!(result[0].item, "y");
    }
}
