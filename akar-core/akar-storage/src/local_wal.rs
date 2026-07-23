//! LocalWAL — per-transaction in-memory WAL buffer.
//!
//! Each write transaction has its own `LocalWAL` that buffers WAL records
//! in-memory during the transaction. On commit, the entire buffer is
//! bulk-copied into the global `WAL` via `WAL::log_committed_wal()`.
//!
//! This avoids contention on the global WAL mutex during writes — only
//! the commit path needs to serialize.

use crate::wal::WALRecord;
use std::io::Write;

/// A serialized WAL buffer backed by an in-memory byte vector.
///
/// Records are written in the same binary format as the global `WAL`
/// (see `WAL::flush_to_disk()`), so the buffer can be bulk-copied
/// directly into the global WAL file on commit.
#[derive(Debug, Default)]
pub struct LocalWAL {
    /// Serialized WAL records in memory.
    buffer: Vec<u8>,
    /// Number of records buffered.
    count: usize,
    /// Estimated total size in bytes.
    size: usize,
}

impl LocalWAL {
    /// Create a new empty LocalWAL buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Serialize a WAL record into the in-memory buffer.
    fn write_record(&mut self, record: &WALRecord) {
        use akar_common::serialization::Serialize;
        match record {
            WALRecord::Insert { table_id, data } => {
                self.buffer.write_all(b"I").unwrap();
                table_id.serialize(&mut self.buffer).unwrap();
                (data.len() as u32).serialize(&mut self.buffer).unwrap();
                self.buffer.write_all(data).unwrap();
                self.size += 1 + 8 + 4 + data.len();
            }
            WALRecord::Delete { table_id, row_id } => {
                self.buffer.write_all(b"D").unwrap();
                table_id.serialize(&mut self.buffer).unwrap();
                row_id.serialize(&mut self.buffer).unwrap();
                self.size += 1 + 8 + 8;
            }
            WALRecord::Update {
                table_id,
                row_id,
                column,
                data,
            } => {
                self.buffer.write_all(b"U").unwrap();
                table_id.serialize(&mut self.buffer).unwrap();
                row_id.serialize(&mut self.buffer).unwrap();
                column.serialize(&mut self.buffer).unwrap();
                (data.len() as u32).serialize(&mut self.buffer).unwrap();
                self.buffer.write_all(data).unwrap();
                self.size += 1 + 8 + 8 + 4 + 4 + data.len();
            }
            WALRecord::UpdateFsm { page_idx, is_free } => {
                self.buffer.write_all(b"F").unwrap();
                page_idx.serialize(&mut self.buffer).unwrap();
                let is_free_u8: u8 = if *is_free { 1 } else { 0 };
                is_free_u8.serialize(&mut self.buffer).unwrap();
                self.size += 1 + 8 + 1;
            }
            WALRecord::ColumnWrite {
                table_id,
                col_id,
                page_id,
                data,
            } => {
                self.buffer.write_all(b"W").unwrap();
                table_id.serialize(&mut self.buffer).unwrap();
                col_id.serialize(&mut self.buffer).unwrap();
                page_id.serialize(&mut self.buffer).unwrap();
                (data.len() as u32).serialize(&mut self.buffer).unwrap();
                self.buffer.write_all(data).unwrap();
                self.size += 1 + 8 + 4 + 8 + 4 + data.len();
            }
            WALRecord::Commit { transaction_id } => {
                self.buffer.write_all(b"C").unwrap();
                transaction_id.serialize(&mut self.buffer).unwrap();
                self.size += 1 + 8;
            }
            WALRecord::Rollback { transaction_id } => {
                self.buffer.write_all(b"R").unwrap();
                transaction_id.serialize(&mut self.buffer).unwrap();
                self.size += 1 + 8;
            }
            WALRecord::Checkpoint => {
                self.buffer.write_all(b"K").unwrap();
                self.size += 1;
            }
            // LocalWALData is the raw buffer from a committed LocalWAL.
            // It is never written by a LocalWAL itself (only by the global WAL
            // when merging). We log it as raw bytes with the 'L' tag.
            WALRecord::LocalWALData { data } => {
                self.buffer.write_all(b"L").unwrap();
                (data.len() as u32).serialize(&mut self.buffer).unwrap();
                self.buffer.write_all(data).unwrap();
                self.size += 1 + 4 + data.len();
            }
            // DDL variants — each writes a tag + u64 table_id
            WALRecord::CreateTable { table_id }
            | WALRecord::DropTable { table_id }
            | WALRecord::AlterTable { table_id }
            | WALRecord::CreateIndex { table_id }
            | WALRecord::DropIndex { table_id }
            | WALRecord::CreateSequence { table_id } => {
                let tag: u8 = match record {
                    WALRecord::CreateTable { .. } => b'T',
                    WALRecord::DropTable { .. } => b'A',
                    WALRecord::AlterTable { .. } => b'M',
                    WALRecord::CreateIndex { .. } => b'N',
                    WALRecord::DropIndex { .. } => b'X',
                    WALRecord::CreateSequence { .. } => b'Q',
                    _ => unreachable!(),
                };
                self.buffer.write_all(&[tag]).unwrap();
                table_id.serialize(&mut self.buffer).unwrap();
                self.size += 1 + 8;
            }
        }
        self.count += 1;
    }

    /// Log a table row insertion.
    pub fn log_insert(&mut self, table_id: u64, data: Vec<u8>) {
        self.write_record(&WALRecord::Insert { table_id, data });
    }

    /// Log a table row deletion.
    pub fn log_delete(&mut self, table_id: u64, row_id: u64) {
        self.write_record(&WALRecord::Delete { table_id, row_id });
    }

    /// Log a table row update.
    pub fn log_update(&mut self, table_id: u64, row_id: u64, column: u32, data: Vec<u8>) {
        self.write_record(&WALRecord::Update {
            table_id,
            row_id,
            column,
            data,
        });
    }

    /// Log a column page write.
    pub fn log_column_write(&mut self, table_id: u64, col_id: u32, page_id: u64, data: Vec<u8>) {
        self.write_record(&WALRecord::ColumnWrite {
            table_id,
            col_id,
            page_id,
            data,
        });
    }

    /// Log the beginning of a write transaction.
    pub fn log_begin_transaction(&mut self) {
        // The begin transaction marker is implicit — no separate record.
        // The first log_*/logCommit records define the transaction boundary.
    }

    /// Log a commit (will be flushed to global WAL on commit).
    pub fn log_commit(&mut self, transaction_id: u64) {
        self.write_record(&WALRecord::Commit { transaction_id });
    }

    /// Log a rollback.
    pub fn log_rollback(&mut self, transaction_id: u64) {
        self.write_record(&WALRecord::Rollback { transaction_id });
    }

    /// Log a checkpoint marker.
    pub fn log_checkpoint(&mut self) {
        self.write_record(&WALRecord::Checkpoint);
    }

    /// Return a reference to the serialized buffer.
    pub fn buffer(&self) -> &[u8] {
        &self.buffer
    }

    /// Consume and return the serialized buffer (for bulk-copy to global WAL).
    pub fn into_buffer(mut self) -> Vec<u8> {
        std::mem::take(&mut self.buffer)
    }

    /// Number of records buffered.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Total size of buffered data in bytes.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Clear all buffered records.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.count = 0;
        self.size = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_wal_empty() {
        let lwal = LocalWAL::new();
        assert!(lwal.is_empty());
        assert_eq!(lwal.count(), 0);
        assert_eq!(lwal.size(), 0);
    }

    #[test]
    fn test_local_wal_insert_record() {
        let mut lwal = LocalWAL::new();
        lwal.log_insert(1, vec![0x01, 0x02, 0x03]);
        assert!(!lwal.is_empty());
        assert_eq!(lwal.count(), 1);
        assert!(lwal.size() > 0);
    }

    #[test]
    fn test_local_wal_multiple_records() {
        let mut lwal = LocalWAL::new();
        lwal.log_insert(1, vec![0x01]);
        lwal.log_delete(1, 42);
        lwal.log_update(1, 42, 0, vec![0x05]);
        lwal.log_commit(100);
        assert_eq!(lwal.count(), 4);
        assert!(lwal.size() > 0);
    }

    #[test]
    fn test_local_wal_clear() {
        let mut lwal = LocalWAL::new();
        lwal.log_insert(1, vec![0x01]);
        lwal.clear();
        assert!(lwal.is_empty());
        assert_eq!(lwal.count(), 0);
    }

    #[test]
    fn test_local_wal_into_buffer() {
        let mut lwal = LocalWAL::new();
        lwal.log_insert(1, vec![0x01, 0x02]);
        let buf = lwal.into_buffer();
        assert!(!buf.is_empty());
    }

    #[test]
    fn test_local_wal_buffer_content() {
        let mut lwal = LocalWAL::new();
        lwal.log_insert(42, vec![0xAB, 0xCD]);
        let buf = lwal.buffer();
        // Format: b"I" + u64 table_id(42) + u32 data_len(2) + data
        assert_eq!(buf[0], b'I', "First byte should be 'I' for Insert");
    }
}
