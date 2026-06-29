//! Column statistics for cardinality estimation in the optimizer.

use std::collections::HashMap;

/// Statistics for a single column.
#[derive(Debug, Clone)]
pub struct ColumnStats {
    pub table_id: u64,
    pub column_id: u32,
    pub num_distinct_values: u64,
    pub num_null_values: u64,
    pub min_value: Option<Vec<u8>>,
    pub max_value: Option<Vec<u8>>,
}

/// Per-table statistics.
#[derive(Debug, Default)]
pub struct TableStats {
    pub num_rows: u64,
    pub columns: HashMap<u32, ColumnStats>,
}

/// Global statistics manager.
#[derive(Debug, Default)]
pub struct StatsStore {
    tables: HashMap<u64, TableStats>,
}

impl StatsStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_table_stats(&self, table_id: u64) -> Option<&TableStats> {
        self.tables.get(&table_id)
    }

    pub fn get_column_stats(&self, table_id: u64, column_id: u32) -> Option<&ColumnStats> {
        self.tables.get(&table_id).and_then(|t| t.columns.get(&column_id))
    }

    pub fn update_table_stats(&mut self, table_id: u64, stats: TableStats) {
        self.tables.insert(table_id, stats);
    }
}
