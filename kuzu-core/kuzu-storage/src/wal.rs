//! Write-Ahead Log for crash recovery.

use std::path::PathBuf;

/// A record in the WAL.
#[derive(Debug, Clone)]
pub enum WALRecord {
    Insert { table_id: u64, data: Vec<u8> },
    Delete { table_id: u64, row_id: u64 },
    Update { table_id: u64, row_id: u64, column: u32, data: Vec<u8> },
    Commit { transaction_id: u64 },
    Rollback { transaction_id: u64 },
    Checkpoint,
}

/// Write-Ahead Log for durability.
pub struct WAL {
    path: PathBuf,
    records: Vec<WALRecord>,
}

impl WAL {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            records: Vec::new(),
        }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn append(&mut self, record: WALRecord) {
        self.records.push(record);
    }

    pub fn records(&self) -> &[WALRecord] {
        &self.records
    }

    pub fn clear(&mut self) {
        self.records.clear();
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}
