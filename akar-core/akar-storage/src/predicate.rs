//! Zone map predicate skipping for column scans.
//!
//! Uses column min/max statistics to determine whether a column chunk
//! can be skipped during a scan, avoiding unnecessary I/O.
//!
//! Ported from C++ `src/include/storage/predicate/` (column_predicate.h,
//! constant_predicate.h, null_predicate.h) and `src/storage/predicate/`.

use std::cmp::Ordering;

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
    ///
    /// Works for every ordered primitive `Value` type (numeric, Bool, String,
    /// Date/Timestamp variants, InternalID). Null values are not ordered: they
    /// clear `guaranteed_no_nulls` and leave min/max untouched so a later
    /// non-null value is not compared against a `Null` sentinel.
    ///
    /// Values whose type cannot be ordered relative to the existing min/max
    /// (mismatched types, Blob/List/Map/...) are ignored for the min/max
    /// bounds — the stats stay conservative and never cause a wrong
    /// `SkipScan` (P52.22).
    pub fn update(&mut self, val: &Value) {
        if matches!(val, Value::Null) {
            self.guaranteed_no_nulls = false;
            return;
        }
        self.guaranteed_all_nulls = false;
        match (&self.min, &self.max) {
            (None, None) => {
                self.min = Some(val.clone());
                self.max = Some(val.clone());
            }
            (Some(min), Some(max)) => {
                if value_cmp(val, min) == Some(Ordering::Less) {
                    self.min = Some(val.clone());
                }
                if value_cmp(val, max) == Some(Ordering::Greater) {
                    self.max = Some(val.clone());
                }
            }
            _ => unreachable!(),
        }
    }
}

/// Order two `Value`s of the same primitive kind.
///
/// Returns `Some(ordering)` when the pair is comparable, `None` otherwise
/// (mismatched types or non-ordered kinds such as Blob/List/Map/Union).
fn value_cmp(a: &Value, b: &Value) -> Option<Ordering> {
    let ord = match (a, b) {
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Int64(x), Value::Int64(y)) => x.cmp(y),
        (Value::Int32(x), Value::Int32(y)) => x.cmp(y),
        (Value::Int16(x), Value::Int16(y)) => x.cmp(y),
        (Value::Int8(x), Value::Int8(y)) => x.cmp(y),
        (Value::UInt64(x), Value::UInt64(y)) => x.cmp(y),
        (Value::UInt32(x), Value::UInt32(y)) => x.cmp(y),
        (Value::UInt16(x), Value::UInt16(y)) => x.cmp(y),
        (Value::UInt8(x), Value::UInt8(y)) => x.cmp(y),
        (Value::Int128(x), Value::Int128(y)) => x.cmp(y),
        (Value::UInt128(x), Value::UInt128(y)) => x.cmp(y),
        (Value::Double(x), Value::Double(y)) => x.partial_cmp(y)?,
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y)?,
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Date(x), Value::Date(y)) => x.cmp(y),
        (Value::Timestamp(x), Value::Timestamp(y)) => x.cmp(y),
        (Value::TimestampTz(x), Value::TimestampTz(y)) => x.0.cmp(&y.0),
        (Value::TimestampNs(x), Value::TimestampNs(y)) => x.cmp(y),
        (Value::TimestampMs(x), Value::TimestampMs(y)) => x.cmp(y),
        (Value::TimestampSec(x), Value::TimestampSec(y)) => x.cmp(y),
        // InternalID orders by table, then offset.
        (Value::InternalID(x), Value::InternalID(y)) => (x.table_id, x.offset).cmp(&(y.table_id, y.offset)),
        _ => return None,
    };
    Some(ord)
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
        (Value::Bool(a), Value::Bool(b), Value::Bool(c)) => check_constant_predicate(a, b, c, op),
        // Integers (all widths)
        (Value::Int64(a), Value::Int64(b), Value::Int64(c)) => check_constant_predicate(a, b, c, op),
        (Value::Int32(a), Value::Int32(b), Value::Int32(c)) => check_constant_predicate(a, b, c, op),
        (Value::Int16(a), Value::Int16(b), Value::Int16(c)) => check_constant_predicate(a, b, c, op),
        (Value::Int8(a), Value::Int8(b), Value::Int8(c)) => check_constant_predicate(a, b, c, op),
        (Value::UInt64(a), Value::UInt64(b), Value::UInt64(c)) => check_constant_predicate(a, b, c, op),
        (Value::UInt32(a), Value::UInt32(b), Value::UInt32(c)) => check_constant_predicate(a, b, c, op),
        (Value::UInt16(a), Value::UInt16(b), Value::UInt16(c)) => check_constant_predicate(a, b, c, op),
        (Value::UInt8(a), Value::UInt8(b), Value::UInt8(c)) => check_constant_predicate(a, b, c, op),
        (Value::Int128(a), Value::Int128(b), Value::Int128(c)) => check_constant_predicate(a, b, c, op),
        (Value::UInt128(a), Value::UInt128(b), Value::UInt128(c)) => check_constant_predicate(a, b, c, op),
        // Floats
        (Value::Double(a), Value::Double(b), Value::Double(c)) => check_constant_predicate(a, b, c, op),
        (Value::Float(a), Value::Float(b), Value::Float(c)) => check_constant_predicate(a, b, c, op),
        // String
        (Value::String(a), Value::String(b), Value::String(c)) => check_constant_predicate(a, b, c, op),
        // Temporal
        (Value::Date(a), Value::Date(b), Value::Date(c)) => check_constant_predicate(a, b, c, op),
        (Value::Timestamp(a), Value::Timestamp(b), Value::Timestamp(c)) => check_constant_predicate(a, b, c, op),
        (Value::TimestampTz(a), Value::TimestampTz(b), Value::TimestampTz(c)) => {
            // TimestampTZ is a bare i64 (no Ord impl) — compare the inner value.
            check_constant_predicate(&a.0, &b.0, &c.0, op)
        }
        (Value::TimestampNs(a), Value::TimestampNs(b), Value::TimestampNs(c)) => check_constant_predicate(a, b, c, op),
        (Value::TimestampMs(a), Value::TimestampMs(b), Value::TimestampMs(c)) => check_constant_predicate(a, b, c, op),
        (Value::TimestampSec(a), Value::TimestampSec(b), Value::TimestampSec(c)) => {
            check_constant_predicate(a, b, c, op)
        }
        // InternalID: order by (table_id, offset) so rows from different
        // tables never alias each other (P52.22).
        (Value::InternalID(a), Value::InternalID(b), Value::InternalID(c)) => check_constant_predicate(
            &(a.table_id, a.offset),
            &(b.table_id, b.offset),
            &(c.table_id, c.offset),
            op,
        ),
        // Mixed/unknown types — fall back to AlwaysScan (never wrong-skip)
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
    use akar_common::types::{Date, InternalID};

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

    #[test]
    fn test_column_chunk_stats_null_tracking() {
        let mut stats = ColumnChunkStats::new(None, None);
        assert!(stats.guaranteed_no_nulls, "empty chunk has no nulls");

        stats.update(&Value::Null);
        assert!(
            !stats.guaranteed_no_nulls,
            "appending Null must clear guaranteed_no_nulls"
        );
        assert_eq!(stats.min, None, "Null must not become min");
        assert_eq!(stats.max, None, "Null must not become max");

        stats.update(&Value::Int64(5));
        assert_eq!(stats.min, Some(Value::Int64(5)));
        assert_eq!(stats.max, Some(Value::Int64(5)));
        assert!(!stats.guaranteed_no_nulls, "flag stays cleared after a null");
        assert!(!stats.guaranteed_all_nulls, "non-null value clears all-nulls");

        stats.update(&Value::Null);
        assert_eq!(stats.min, Some(Value::Int64(5)), "later Null must not corrupt min/max");
        assert_eq!(stats.max, Some(Value::Int64(5)));
        assert_eq!(
            check_null_zone_map(&stats, true),
            ZoneMapCheckResult::AlwaysScan,
            "IS NULL must scan when chunk has nulls"
        );
    }

    #[test]
    fn test_column_chunk_stats_all_nulls() {
        let mut stats = ColumnChunkStats::new(None, None);
        stats.update(&Value::Null);
        stats.update(&Value::Null);
        assert!(!stats.guaranteed_no_nulls);
        assert_eq!(stats.min, None);
        assert_eq!(stats.max, None);
    }

    #[test]
    fn test_bool_zone_map_and_stats() {
        // Bool min/max must track correctly: false < true (P52.22).
        let mut stats = ColumnChunkStats::new(None, None);
        stats.update(&Value::Bool(true));
        stats.update(&Value::Bool(false));
        assert_eq!(stats.min, Some(Value::Bool(false)));
        assert_eq!(stats.max, Some(Value::Bool(true)));

        // Single-value chunk: "!=" against that value can skip; "=" can't.
        let single = ColumnChunkStats::new(Some(Value::Bool(true)), Some(Value::Bool(true)));
        assert_eq!(
            check_zone_map(&single, "!=", &Value::Bool(true)),
            ZoneMapCheckResult::SkipScan
        );
        assert_eq!(
            check_zone_map(&single, "=", &Value::Bool(true)),
            ZoneMapCheckResult::AlwaysScan
        );
        assert_eq!(
            check_zone_map(&single, "=", &Value::Bool(false)),
            ZoneMapCheckResult::SkipScan
        );
    }

    #[test]
    fn test_date_zone_map_and_stats() {
        // Date min/max must track (P52.22) so a scan never wrong-skips.
        let mut stats = ColumnChunkStats::new(None, None);
        stats.update(&Value::Date(Date(10)));
        stats.update(&Value::Date(Date(5)));
        stats.update(&Value::Date(Date(20)));
        assert_eq!(stats.min, Some(Value::Date(Date(5))));
        assert_eq!(stats.max, Some(Value::Date(Date(20))));

        assert_eq!(
            check_zone_map(&stats, "=", &Value::Date(Date(25))),
            ZoneMapCheckResult::SkipScan
        );
        assert_eq!(
            check_zone_map(&stats, "=", &Value::Date(Date(10))),
            ZoneMapCheckResult::AlwaysScan
        );
    }

    #[test]
    fn test_internal_id_zone_map_respects_table_id() {
        // Regression for P52.22: InternalID is ordered by (table_id, offset).
        // A chunk holding only table-1 rows must NOT be skipped for a constant
        // with the same offset but a different table — every row matches.
        let stats = ColumnChunkStats::new(
            Some(Value::InternalID(InternalID { table_id: 1, offset: 5 })),
            Some(Value::InternalID(InternalID { table_id: 1, offset: 5 })),
        );
        assert_eq!(
            check_zone_map(&stats, "!=", &Value::InternalID(InternalID { table_id: 2, offset: 5 })),
            ZoneMapCheckResult::AlwaysScan
        );
        assert_eq!(
            check_zone_map(&stats, "=", &Value::InternalID(InternalID { table_id: 2, offset: 5 })),
            ZoneMapCheckResult::SkipScan
        );
    }
}
