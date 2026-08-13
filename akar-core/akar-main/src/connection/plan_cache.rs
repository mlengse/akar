//! PlanCache — LRU cache of optimized query plans keyed by normalized query.
//!
//! Repeated calls to `Connection::query()` with the same statement currently
//! re-run the full parse → bind → plan → optimize pipeline. Caching the
//! optimized plan skips all four steps on a cache hit.
//!
//! Plans are invalidated implicitly: every entry records the catalog version
//! at build time, and lookups discard entries whose version no longer matches
//! the live catalog (any DDL bumps the version).

use akar_binder::bound_statement::BoundStatement;
use akar_planner::logical_operator::LogicalOperator;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

/// A cached entry: the bound statement plus the optimized logical plan, both
/// tied to the catalog version they were built against.
///
/// The plan and bound statement are stored behind `Arc` so a cache hit only
/// bumps a reference count instead of deep-cloning the full operator tree on
/// every query (P51.47).
pub(crate) struct CachedPlan {
    pub bound: Arc<BoundStatement>,
    pub plan: Arc<Vec<LogicalOperator>>,
    pub catalog_version: u64,
}

/// A small LRU cache. `get`/`insert` move the key to the most-recently-used
/// end; when the cache is full, the least-recently-used entry is evicted.
pub(crate) struct PlanCache<T> {
    map: HashMap<String, T>,
    order: VecDeque<String>,
    capacity: usize,
}

impl<T> PlanCache<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    pub fn get(&mut self, key: &str) -> Option<&T> {
        if self.map.contains_key(key) {
            if let Some(pos) = self.order.iter().position(|k| k == key) {
                if pos + 1 != self.order.len() {
                    self.order.remove(pos);
                    self.order.push_back(key.to_string());
                }
            }
        }
        self.map.get(key)
    }

    pub fn insert(&mut self, key: String, value: T) {
        if let Some(pos) = self.order.iter().position(|k| k.as_str() == key.as_str()) {
            self.order.remove(pos);
        } else if self.map.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
        self.order.push_back(key.clone());
        self.map.insert(key, value);
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Maximum number of entries before least-recently-used eviction.
    #[allow(dead_code)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

/// Normalize a query string into a stable cache key: trim surrounding
/// whitespace and collapse horizontal whitespace runs into a single space.
///
/// Content inside single-quoted strings, double-quoted strings, and
/// backtick-quoted identifiers is preserved verbatim, and newlines are kept,
/// so normalization never changes a query's semantics (e.g. inside string
/// literals or line comments).
pub fn normalize_query(query: &str) -> String {
    let trimmed = query.trim();
    let mut result = String::with_capacity(trimmed.len());
    let mut prev_space = true;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;

    for ch in trimmed.chars() {
        if in_single {
            result.push(ch);
            if ch == '\'' {
                in_single = false;
            }
            prev_space = false;
            continue;
        }
        if in_double {
            result.push(ch);
            if ch == '"' {
                in_double = false;
            }
            prev_space = false;
            continue;
        }
        if in_backtick {
            result.push(ch);
            if ch == '`' {
                in_backtick = false;
            }
            prev_space = false;
            continue;
        }

        match ch {
            '\'' => {
                in_single = true;
                result.push(ch);
                prev_space = false;
            }
            '"' => {
                in_double = true;
                result.push(ch);
                prev_space = false;
            }
            '`' => {
                in_backtick = true;
                result.push(ch);
                prev_space = false;
            }
            '\n' | '\r' => {
                // Preserve newlines (line comments depend on them)
                result.push('\n');
                prev_space = false;
            }
            c if c.is_whitespace() => {
                if !prev_space {
                    result.push(' ');
                }
                prev_space = true;
            }
            other => {
                result.push(other);
                prev_space = false;
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_query_trims_and_collapses() {
        assert_eq!(normalize_query("  MATCH (p)   RETURN  p  "), "MATCH (p) RETURN p");
        assert_eq!(
            normalize_query("MATCH\n  (p)\tWHERE p.x  >  5"),
            "MATCH\n (p) WHERE p.x > 5"
        );
        assert_eq!(normalize_query(""), "");
        assert_eq!(normalize_query("   "), "");
    }

    #[test]
    fn test_normalize_preserves_string_literals() {
        assert_eq!(normalize_query("RETURN 'a  b'   AS x"), "RETURN 'a  b' AS x");
        assert_eq!(normalize_query("RETURN \"a  b\"   AS x"), "RETURN \"a  b\" AS x");
        assert_eq!(
            normalize_query("MATCH (`a  b`) RETURN `c d`"),
            "MATCH (`a  b`) RETURN `c d`"
        );
    }

    #[test]
    fn test_normalize_keeps_newlines_for_comments() {
        assert_eq!(normalize_query("MATCH (p) // c\nRETURN p"), "MATCH (p) // c\nRETURN p");
    }

    #[test]
    fn test_lru_eviction() {
        let mut cache: PlanCache<u32> = PlanCache::new(2);
        cache.insert("a".into(), 1);
        cache.insert("b".into(), 2);
        cache.insert("c".into(), 3);
        assert_eq!(cache.len(), 2);
        assert!(cache.get("a").is_none());
        assert_eq!(cache.get("b"), Some(&2));
        assert_eq!(cache.get("c"), Some(&3));
    }

    #[test]
    fn test_lru_touch_moves_to_back() {
        let mut cache: PlanCache<u32> = PlanCache::new(2);
        cache.insert("a".into(), 1);
        cache.insert("b".into(), 2);
        // Access "a" → it becomes most-recently-used; "b" evicted next
        assert_eq!(cache.get("a"), Some(&1));
        cache.insert("c".into(), 3);
        assert!(cache.get("b").is_none());
        assert_eq!(cache.get("a"), Some(&1));
        assert_eq!(cache.get("c"), Some(&3));
    }

    #[test]
    fn test_insert_existing_refreshes() {
        let mut cache: PlanCache<u32> = PlanCache::new(2);
        cache.insert("a".into(), 1);
        cache.insert("b".into(), 2);
        cache.insert("a".into(), 10);
        assert_eq!(cache.get("a"), Some(&10));
        assert_eq!(cache.get("b"), Some(&2));
    }

    #[test]
    fn test_clear() {
        let mut cache: PlanCache<u32> = PlanCache::new(4);
        cache.insert("a".into(), 1);
        cache.insert("b".into(), 2);
        cache.clear();
        assert_eq!(cache.len(), 0);
        assert!(cache.get("a").is_none());
    }

    #[test]
    fn test_capacity_min_one() {
        let cache: PlanCache<u32> = PlanCache::new(0);
        assert_eq!(cache.capacity(), 1);
    }
}
