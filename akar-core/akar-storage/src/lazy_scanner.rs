//! Lazy column scanner — loads NodeGroups on demand during iteration.
//!
//! Instead of materializing all columns upfront, the `LazyColumnScan`
//! iterator loads one NodeGroup at a time, yields its rows, then releases
//! the group. This reduces peak memory for large tables scanned with
//! filters or limits.
//!
//! # Usage
//!
//! ```ignore
//! let scan = LazyColumnScan::new(&node_table, &[0, 1, 2], None, None);
//! for row in scan {
//!     // row is Vec<Value> with columns 0, 1, 2
//! }
//! ```

use crate::table::NodeTable;
use akar_common::types::Value;

/// A lazy scanner that yields rows from a NodeTable one NodeGroup at a time.
///
/// A lazy scanner that yields rows from a NodeTable one NodeGroup at a time.
#[derive(Debug)]
pub struct LazyColumnScan<'a> {
    table: &'a NodeTable,
    column_indices: Vec<usize>,
    group_idx: usize,
    row_in_group: usize,
    group_size: usize,
    current_data: Option<Vec<Vec<Value>>>,
    done: bool,
}

impl<'a> LazyColumnScan<'a> {
    /// Create a new lazy scan over `table` reading the given `column_indices`.
    pub fn new(table: &'a NodeTable, column_indices: Vec<usize>) -> Self {
        Self {
            table,
            column_indices,
            group_idx: 0,
            row_in_group: 0,
            group_size: 0,
            current_data: None,
            done: table.node_groups.is_empty(),
        }
    }

    /// Load the next group's data into `current_data`.
    fn load_next_group(&mut self) {
        self.current_data = None;
        self.row_in_group = 0;

        while self.group_idx < self.table.node_groups.len() {
            let group = &self.table.node_groups[self.group_idx];
            if group.num_nodes == 0 {
                self.group_idx += 1;
                continue;
            }

            self.group_size = group.num_nodes as usize;

            // Materialize only the requested columns for this group
            let mut data = Vec::with_capacity(self.group_size);
            for row_offset in 0..self.group_size {
                let mut row = Vec::with_capacity(self.column_indices.len());
                for &col_idx in &self.column_indices {
                    let val = group
                        .columns
                        .get(col_idx)
                        .and_then(|col| col.get(row_offset))
                        .cloned()
                        .unwrap_or(Value::Null);
                    row.push(val);
                }
                data.push(row);
            }

            if !data.is_empty() {
                self.current_data = Some(data);
                // Don't advance group_idx yet — we yield rows from this group first
                return;
            }
            self.group_idx += 1;
        }

        self.done = true;
    }
}

impl<'a> Iterator for LazyColumnScan<'a> {
    type Item = Vec<Value>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        if self.current_data.is_none() || self.row_in_group >= self.group_size {
            if self.current_data.is_some() {
                self.group_idx += 1;
            }
            self.load_next_group();
            if self.done {
                return None;
            }
        }

        let data = self.current_data.as_ref()?;
        if self.row_in_group >= data.len() {
            self.group_idx += 1;
            self.load_next_group();
            return self.next();
        }

        let row = data[self.row_in_group].clone();
        self.row_in_group += 1;
        Some(row)
    }
}

/// A filtered lazy scan that applies a predicate while scanning.
///
/// Loads one NodeGroup at a time and applies the predicate to each row,
/// yielding only matching rows. Useful for `WHERE` clauses on large tables.
pub struct FilteredLazyScan<'a> {
    inner: LazyColumnScan<'a>,
    predicate: Box<dyn Fn(&[Value]) -> bool + 'a>,
    lookahead: Vec<Vec<Value>>,
}

impl<'a> FilteredLazyScan<'a> {
    pub fn new(
        table: &'a NodeTable,
        column_indices: Vec<usize>,
        predicate: Box<dyn Fn(&[Value]) -> bool + 'a>,
    ) -> Self {
        Self {
            inner: LazyColumnScan::new(table, column_indices),
            predicate,
            lookahead: Vec::new(),
        }
    }
}

impl<'a> Iterator for FilteredLazyScan<'a> {
    type Item = Vec<Value>;

    fn next(&mut self) -> Option<Self::Item> {
        // Drain lookahead buffer first
        if !self.lookahead.is_empty() {
            return Some(self.lookahead.remove(0));
        }

        // Scan until we find a matching row or exhaust all groups
        self.inner.by_ref().find(|row| (self.predicate)(row))
    }
}

/// Scan a table with an optional row limit, loading groups lazily.
pub fn lazy_scan_table(table: &NodeTable, column_indices: &[usize], max_rows: Option<usize>) -> Vec<Vec<Value>> {
    let mut result = Vec::new();
    let scan = LazyColumnScan::new(table, column_indices.to_vec());

    for row in scan {
        result.push(row);
        if let Some(limit) = max_rows {
            if result.len() >= limit {
                break;
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::{ColumnDefinition, NodeTable};
    use akar_common::enums::CompressionType;
    use akar_common::types::{LogicalTypeID, Value};

    fn make_test_table(rows: usize) -> NodeTable {
        let cols = vec![
            ColumnDefinition {
                name: "id".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: true,
                compression: CompressionType::Uncompressed,
            },
            ColumnDefinition {
                name: "name".into(),
                logical_type: LogicalTypeID::String,
                is_primary_key: false,
                compression: CompressionType::Uncompressed,
            },
        ];
        let mut table = NodeTable::new(0, "test".into(), cols);
        for i in 0..rows {
            table
                .insert_row(vec![Value::Int64(i as i64), Value::String(format!("node_{i}"))])
                .unwrap();
        }
        table
    }

    #[test]
    fn test_lazy_scan_all_rows() {
        let table = make_test_table(100);
        let result = lazy_scan_table(&table, &[0, 1], None);
        assert_eq!(result.len(), 100);
        assert_eq!(result[0], vec![Value::Int64(0), Value::String("node_0".into())]);
        assert_eq!(result[99], vec![Value::Int64(99), Value::String("node_99".into())]);
    }

    #[test]
    fn test_lazy_scan_single_column() {
        let table = make_test_table(50);
        let result = lazy_scan_table(&table, &[0], None);
        assert_eq!(result.len(), 50);
        assert_eq!(result[0], vec![Value::Int64(0)]);
        assert_eq!(result[49], vec![Value::Int64(49)]);
    }

    #[test]
    fn test_lazy_scan_max_rows() {
        let table = make_test_table(1000);
        let result = lazy_scan_table(&table, &[0], Some(10));
        assert_eq!(result.len(), 10);
    }

    #[test]
    fn test_lazy_scan_empty_table() {
        let table = make_test_table(0);
        let result = lazy_scan_table(&table, &[0], None);
        assert!(result.is_empty());
    }

    #[test]
    fn test_filtered_lazy_scan() {
        let table = make_test_table(100);
        let scan = FilteredLazyScan::new(
            &table,
            vec![0, 1],
            Box::new(|row| if let Value::Int64(id) = row[0] { id >= 50 } else { false }),
        );
        let result: Vec<_> = scan.collect();
        assert_eq!(result.len(), 50);
        assert_eq!(result[0], vec![Value::Int64(50), Value::String("node_50".into())]);
    }

    #[test]
    fn test_lazy_iterator_interface() {
        let table = make_test_table(5);
        let scan = LazyColumnScan::new(&table, vec![0]);
        let ids: Vec<i64> = scan
            .filter_map(|row| if let Value::Int64(id) = row[0] { Some(id) } else { None })
            .collect();
        assert_eq!(ids, vec![0, 1, 2, 3, 4]);
    }
}
