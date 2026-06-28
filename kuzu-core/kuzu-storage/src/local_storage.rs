//! Local storage — per-transaction write buffer before commit.

use std::collections::HashMap;

/// A local table insert/update buffer for an in-progress transaction.
#[derive(Debug, Default)]
pub struct LocalTableData {
    inserted_rows: Vec<Vec<u8>>,
    deleted_row_ids: Vec<u64>,
    updated_rows: HashMap<u64, Vec<u8>>,
}

impl LocalTableData {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, row_data: Vec<u8>) {
        self.inserted_rows.push(row_data);
    }

    pub fn delete(&mut self, row_id: u64) {
        self.deleted_row_ids.push(row_id);
    }

    pub fn update(&mut self, row_id: u64, row_data: Vec<u8>) {
        self.updated_rows.insert(row_id, row_data);
    }

    pub fn inserted_rows(&self) -> &[Vec<u8>] {
        &self.inserted_rows
    }

    pub fn deleted_row_ids(&self) -> &[u64] {
        &self.deleted_row_ids
    }

    pub fn updated_rows(&self) -> &HashMap<u64, Vec<u8>> {
        &self.updated_rows
    }

    pub fn clear(&mut self) {
        self.inserted_rows.clear();
        self.deleted_row_ids.clear();
        self.updated_rows.clear();
    }
}

/// Per-transaction local storage for all modified tables.
#[derive(Debug, Default)]
pub struct LocalStorage {
    tables: HashMap<u64, LocalTableData>,
}

impl LocalStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_or_create_table(&mut self, table_id: u64) -> &mut LocalTableData {
        self.tables.entry(table_id).or_default()
    }

    pub fn get_table(&self, table_id: u64) -> Option<&LocalTableData> {
        self.tables.get(&table_id)
    }

    pub fn clear(&mut self) {
        self.tables.clear();
    }
}
