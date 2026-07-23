//! Zone map predicate skipping for column scans.
//!
//! Uses column min/max statistics to determine whether a column chunk
//! can be skipped during a scan, avoiding unnecessary I/O.
//!
//! Ported from C++ `src/include/storage/predicate/` (column_predicate.h,
//! constant_predicate.h, null_predicate.h) and `src/storage/predicate/`.

use akar_common::types::Value;

/// Result of checking a zone map against a predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneMapCheckResult {
    /// Cannot determine — must scan the chunk.
    AlwaysScan = 0,
    /// Predicate definitely won't match — safe to skip.
    SkipScan = 1,
}

/// Statistics for a column chunk, used for zone map predicate checking.
#[derive(Debug, Clone)]
pub struct ColumnChunkStats {
    /// Minimum value in the chunk (None if unknown).
    pub min: Option<Value>,
    /// Maximum value in the chunk (None if unknown).
    pub max: Option<Value>,
    /// Whether the chunk is guaranteed to have no nulls.
    pub guaranteed_no_nulls: bool,
    /// Whether the chunk is guaranteed to have all nulls.
    pub guaranteed_all_nulls: bool,
}

impl ColumnChunkStats {
    pub fn new(min: Option<Value>, max: Option<Value>) -> Self {
        Self {
            min,
            max,
            guaranteed_no_nulls: true,
            guaranteed_all_nulls: false,
        }
    }

    /// Create stats for an all-null chunk.
    pub fn all_nulls() -> Self {
        Self {
            min: None,
            max: None,
            guaranteed_no_nulls: false,
            guaranteed_all_nulls: true,
        }
    }

    /// Update min/max with a new value.
    /// Only works for ordered Value types (Int64, Double, Float, Int32, String).
    pub fn update(&mut self, val: &Value) {
        match (&self.min, &self.max) {
            (None, None) => {
                self.min = Some(val.clone());
                self.max = Some(val.clone());
            }
            (Some(min), Some(max)) => {
                if Self::value_lt(val, min) {
                    self.min = Some(val.clone());
                }
                if Self::value_gt(val, max) {
                    self.max = Some(val.clone());
                }
            }
            _ => unreachable!(),
        }
    }

    /// Compare two Values for ordering. Returns true if a < b.
    fn value_lt(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Int64(x), Value::Int64(y)) => x < y,
            (Value::Double(x), Value::Double(y)) => x < y,
            (Value::Float(x), Value::Float(y)) => x < y,
            (Value::Int32(x), Value::Int32(y)) => x < y,
            (Value::String(x), Value::String(y)) => x < y,
            _ => false,
        }
    }

    /// Compare two Values for ordering. Returns true if a > b.
    fn value_gt(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Int64(x), Value::Int64(y)) => x > y,
            (Value::Double(x), Value::Double(y)) => x > y,
            (Value::Float(x), Value::Float(y)) => x > y,
            (Value::Int32(x), Value::Int32(y)) => x > y,
            (Value::String(x), Value::String(y)) => x > y,
            _ => false,
        }
    }
}

// ==================== Check helpers ====================

/// Check if a value falls within [min, max].
fn in_range<T: PartialOrd>(min: &T, max: &T, val: &T) -> bool {
    val >= min && val <= max
}

/// Zone map check for constant-value comparison predicates.
/// Returns `SkipScan` if the predicate cannot possibly match the zone.
fn check_constant_predicate<T: PartialOrd>(min: &T, max: &T, constant: &T, op: &str) -> ZoneMapCheckResult {
    match op {
        "=" | "==" => {
            if !in_range(min, max, constant) {
                return ZoneMapCheckResult::SkipScan;
            }
        }
        "!=" | "<>" => {
            if constant == min && constant == max {
                return ZoneMapCheckResult::SkipScan;
            }
        }
        ">" => {
            if constant >= max {
                return ZoneMapCheckResult::SkipScan;
            }
        }
        ">=" => {
            if constant > max {
                return ZoneMapCheckResult::SkipScan;
            }
        }
        "<" => {
            if constant <= min {
                return ZoneMapCheckResult::SkipScan;
            }
        }
        "<=" if constant < min => {
            return ZoneMapCheckResult::SkipScan;
        }
        _ => {}
    }
    ZoneMapCheckResult::AlwaysScan
}

/// Check whether a column chunk with the given stats can be skipped
/// based on a predicate `(column op constant)`.
///
/// Returns `SkipScan` if the chunk definitely doesn't match.
pub fn check_zone_map(stats: &ColumnChunkStats, op: &str, constant: &Value) -> ZoneMapCheckResult {
    let (Some(min), Some(max)) = (&stats.min, &stats.max) else {
        return ZoneMapCheckResult::AlwaysScan;
    };

    // Type-based dispatch for comparison
    match (min, max, constant) {
        // Int64
        (Value::Int64(min_v), Value::Int64(max_v), Value::Int64(c)) => check_constant_predicate(min_v, max_v, c, op),
        // Double
        (Value::Double(min_v), Value::Double(max_v), Value::Double(c)) => check_constant_predicate(min_v, max_v, c, op),
        // Float
        (Value::Float(min_v), Value::Float(max_v), Value::Float(c)) => check_constant_predicate(min_v, max_v, c, op),
        // Int32
        (Value::Int32(min_v), Value::Int32(max_v), Value::Int32(c)) => check_constant_predicate(min_v, max_v, c, op),
        // String
        (Value::String(min_v), Value::String(max_v), Value::String(c)) => check_constant_predicate(min_v, max_v, c, op),
        // InternalID (compare by offset)
        (Value::InternalID(min_id), Value::InternalID(max_id), Value::InternalID(c_id)) => {
            check_constant_predicate(&min_id.offset, &max_id.offset, &c_id.offset, op)
        }
        // Mixed/unknown types — fall back to AlwaysScan
        _ => ZoneMapCheckResult::AlwaysScan,
    }
}

/// Check a null predicate against chunk stats.
/// `is_null` is true for `IS NULL`, false for `IS NOT NULL`.
pub fn check_null_zone_map(stats: &ColumnChunkStats, is_null: bool) -> ZoneMapCheckResult {
    if is_null {
        if stats.guaranteed_no_nulls {
            return ZoneMapCheckResult::SkipScan;
        }
    } else if stats.guaranteed_all_nulls {
        return ZoneMapCheckResult::SkipScan;
    }
    ZoneMapCheckResult::AlwaysScan
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_common::types::InternalID;

    fn int_stats(min: i64, max: i64) -> ColumnChunkStats {
        ColumnChunkStats::new(Some(Value::Int64(min)), Some(Value::Int64(max)))
    }

    #[test]
    fn test_eq_in_range() {
        let stats = int_stats(10, 20);
        assert_eq!(
            check_zone_map(&stats, "=", &Value::Int64(15)),
            ZoneMapCheckResult::AlwaysScan
        );
    }

    #[test]
    fn test_eq_out_of_range() {
        let stats = int_stats(10, 20);
        assert_eq!(
            check_zone_map(&stats, "=", &Value::Int64(25)),
            ZoneMapCheckResult::SkipScan
        );
    }

    #[test]
    fn test_eq_below_range() {
        let stats = int_stats(10, 20);
        assert_eq!(
            check_zone_map(&stats, "=", &Value::Int64(5)),
            ZoneMapCheckResult::SkipScan
        );
    }

    #[test]
    fn test_gt_all_below() {
        let stats = int_stats(10, 20);
        // constant > max → nothing in chunk can match
        assert_eq!(
            check_zone_map(&stats, ">", &Value::Int64(25)),
            ZoneMapCheckResult::SkipScan
        );
    }

    #[test]
    fn test_gt_some_above() {
        let stats = int_stats(10, 20);
        assert_eq!(
            check_zone_map(&stats, ">", &Value::Int64(5)),
            ZoneMapCheckResult::AlwaysScan
        );
    }

    #[test]
    fn test_lt_all_above() {
        let stats = int_stats(10, 20);
        // constant < min → nothing in chunk can match
        assert_eq!(
            check_zone_map(&stats, "<", &Value::Int64(5)),
            ZoneMapCheckResult::SkipScan
        );
    }

    #[test]
    fn test_lt_some_below() {
        let stats = int_stats(10, 20);
        assert_eq!(
            check_zone_map(&stats, "<", &Value::Int64(25)),
            ZoneMapCheckResult::AlwaysScan
        );
    }

    #[test]
    fn test_gte_eq_max_not_skip() {
        let stats = int_stats(10, 20);
        // constant == max → >= can match
        assert_eq!(
            check_zone_map(&stats, ">=", &Value::Int64(20)),
            ZoneMapCheckResult::AlwaysScan
        );
    }

    #[test]
    fn test_gte_gt_max() {
        let stats = int_stats(10, 20);
        // constant > max → >= can't match
        assert_eq!(
            check_zone_map(&stats, ">=", &Value::Int64(21)),
            ZoneMapCheckResult::SkipScan
        );
    }

    #[test]
    fn test_lte_eq_min_not_skip() {
        let stats = int_stats(10, 20);
        assert_eq!(
            check_zone_map(&stats, "<=", &Value::Int64(10)),
            ZoneMapCheckResult::AlwaysScan
        );
    }

    #[test]
    fn test_lte_lt_min() {
        let stats = int_stats(10, 20);
        assert_eq!(
            check_zone_map(&stats, "<=", &Value::Int64(9)),
            ZoneMapCheckResult::SkipScan
        );
    }

    #[test]
    fn test_not_eq_single_value() {
        let stats = int_stats(15, 15);
        // constant == min == max → not_eq can't match
        assert_eq!(
            check_zone_map(&stats, "!=", &Value::Int64(15)),
            ZoneMapCheckResult::SkipScan
        );
    }

    #[test]
    fn test_not_eq_range() {
        let stats = int_stats(10, 20);
        // constant within range → might match
        assert_eq!(
            check_zone_map(&stats, "!=", &Value::Int64(15)),
            ZoneMapCheckResult::AlwaysScan
        );
    }

    #[test]
    fn test_null_predicate_is_null_no_nulls() {
        let stats = ColumnChunkStats {
            min: Some(Value::Int64(1)),
            max: Some(Value::Int64(10)),
            guaranteed_no_nulls: true,
            guaranteed_all_nulls: false,
        };
        assert_eq!(check_null_zone_map(&stats, true), ZoneMapCheckResult::SkipScan);
    }

    #[test]
    fn test_null_predicate_is_null_has_nulls() {
        let stats = ColumnChunkStats {
            min: Some(Value::Int64(1)),
            max: Some(Value::Int64(10)),
            guaranteed_no_nulls: false,
            guaranteed_all_nulls: false,
        };
        assert_eq!(check_null_zone_map(&stats, true), ZoneMapCheckResult::AlwaysScan);
    }

    #[test]
    fn test_null_predicate_is_not_null_all_nulls() {
        let stats = ColumnChunkStats {
            min: None,
            max: None,
            guaranteed_no_nulls: false,
            guaranteed_all_nulls: true,
        };
        assert_eq!(check_null_zone_map(&stats, false), ZoneMapCheckResult::SkipScan);
    }

    #[test]
    fn test_no_stats_always_scan() {
        let stats = ColumnChunkStats::new(None, None);
        assert_eq!(
            check_zone_map(&stats, "=", &Value::Int64(5)),
            ZoneMapCheckResult::AlwaysScan
        );
    }

    #[test]
    fn test_string_zone_map() {
        let stats = ColumnChunkStats::new(
            Some(Value::String("apple".into())),
            Some(Value::String("banana".into())),
        );
        // "cherry" > "banana" → skip
        assert_eq!(
            check_zone_map(&stats, "=", &Value::String("cherry".into())),
            ZoneMapCheckResult::SkipScan
        );
        // "avocado" within [apple, banana] → scan
        assert_eq!(
            check_zone_map(&stats, "=", &Value::String("avocado".into())),
            ZoneMapCheckResult::AlwaysScan
        );
    }

    #[test]
    fn test_internal_id_zone_map() {
        let stats = ColumnChunkStats::new(
            Some(Value::InternalID(InternalID { offset: 0, table_id: 1 })),
            Some(Value::InternalID(InternalID {
                offset: 100,
                table_id: 1,
            })),
        );
        assert_eq!(
            check_zone_map(
                &stats,
                "=",
                &Value::InternalID(InternalID {
                    offset: 50,
                    table_id: 1
                })
            ),
            ZoneMapCheckResult::AlwaysScan
        );
        assert_eq!(
            check_zone_map(
                &stats,
                "=",
                &Value::InternalID(InternalID {
                    offset: 200,
                    table_id: 1
                })
            ),
            ZoneMapCheckResult::SkipScan
        );
    }

    #[test]
    fn test_double_zone_map() {
        let stats = ColumnChunkStats::new(Some(Value::Double(1.5)), Some(Value::Double(9.5)));
        assert_eq!(
            check_zone_map(&stats, ">", &Value::Double(10.0)),
            ZoneMapCheckResult::SkipScan
        );
        assert_eq!(
            check_zone_map(&stats, "<", &Value::Double(1.0)),
            ZoneMapCheckResult::SkipScan
        );
    }

    #[test]
    fn test_column_chunk_stats_update() {
        let mut stats = ColumnChunkStats::new(None, None);
        stats.update(&Value::Int64(5));
        assert_eq!(stats.min, Some(Value::Int64(5)));
        assert_eq!(stats.max, Some(Value::Int64(5)));

        stats.update(&Value::Int64(3));
        assert_eq!(stats.min, Some(Value::Int64(3)));
        assert_eq!(stats.max, Some(Value::Int64(5)));

        stats.update(&Value::Int64(10));
        assert_eq!(stats.min, Some(Value::Int64(3)));
        assert_eq!(stats.max, Some(Value::Int64(10)));
    }
}
