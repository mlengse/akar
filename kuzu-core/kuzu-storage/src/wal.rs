//! Write-Ahead Log for crash recovery.
//!
//! Logs all write operations (column writes, table inserts, etc.) before
//! they are applied to the main storage. During checkpoint, the WAL is
//! flushed to disk and the storage pages are synchronized.

use std::io::Write;
use std::path::PathBuf;

/// A record in the WAL.
#[derive(Debug, Clone)]
pub enum WALRecord {
    Insert {
        table_id: u64,
        data: Vec<u8>,
    },
    Delete {
        table_id: u64,
        row_id: u64,
    },
    Update {
        table_id: u64,
        row_id: u64,
        column: u32,
        data: Vec<u8>,
    },
    /// Log a write to a column page: (table_id, col_id, page_id, serialized_data).
    ColumnWrite {
        table_id: u64,
        col_id: u32,
        page_id: u64,
        data: Vec<u8>,
    },
    /// Bulk-copied serialized WAL data from a transaction's LocalWAL.
    /// The raw bytes are flushed directly to disk during checkpoint.
    LocalWALData {
        data: Vec<u8>,
    },
    Commit {
        transaction_id: u64,
    },
    Rollback {
        transaction_id: u64,
    },
    Checkpoint,
}

/// Write-Ahead Log for durability.
pub struct WAL {
    path: PathBuf,
    records: Vec<WALRecord>,
    total_size: usize,
    is_dirty: bool,
}

impl WAL {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            records: Vec::new(),
            total_size: 0,
            is_dirty: false,
        }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn append(&mut self, record: WALRecord) {
        let size = match &record {
            WALRecord::Insert { data, .. } => data.len(),
            WALRecord::Update { data, .. } => data.len(),
            WALRecord::ColumnWrite { data, .. } => data.len(),
            WALRecord::LocalWALData { data } => data.len(),
            _ => 8,
        };
        self.total_size += size;
        self.is_dirty = true;
        self.records.push(record);
    }

    /// Log a column page write before it is applied to the BufferManager.
    pub fn log_column_write(&mut self, table_id: u64, col_id: u32, page_id: u64, data: &[u8]) {
        self.append(WALRecord::ColumnWrite {
            table_id,
            col_id,
            page_id,
            data: data.to_vec(),
        });
    }

    /// Bulk-copy a `LocalWAL`'s serialized buffer into this global WAL.
    ///
    /// Called during commit path: the transaction's `LocalWAL` has already
    /// been serialized to a byte buffer; this method appends the raw bytes
    /// directly to the in-memory record list.
    ///
    /// The caller (`StorageManager::commit_transaction()`) is responsible
    /// for holding the `Arc<Mutex<WAL>>` lock to serialize concurrent calls.
    pub fn write_raw_buffer(&mut self, local_wal_buffer: &[u8]) {
        if local_wal_buffer.is_empty() {
            return;
        }
        self.total_size += local_wal_buffer.len();
        self.is_dirty = true;
        self.records.push(WALRecord::LocalWALData {
            data: local_wal_buffer.to_vec(),
        });
    }

    pub fn records(&self) -> &[WALRecord] {
        &self.records
    }
    pub fn clear(&mut self) {
        self.records.clear();
        self.total_size = 0;
        self.is_dirty = false;
    }
    pub fn len(&self) -> usize {
        self.records.len()
    }
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
    pub fn total_size(&self) -> usize {
        self.total_size
    }
    pub fn is_dirty(&self) -> bool {
        self.is_dirty
    }

    /// Replay the WAL to recover state after a crash.
    pub fn replay<F>(&self, mut apply: F) -> std::io::Result<()>
    where
        F: FnMut(&WALRecord) -> std::io::Result<()>,
    {
        for record in &self.records {
            apply(record)?;
        }
        Ok(())
    }

    /// Persist the WAL to disk.
    pub fn flush_to_disk(&self) -> std::io::Result<()> {
        use kuzu_common::serialization::Serialize;
        let mut file = std::fs::File::create(&self.path)?;
        for record in &self.records {
            match record {
                WALRecord::Insert { table_id, data } => {
                    file.write_all(b"I")?;
                    table_id.serialize(&mut file)?;
                    (data.len() as u32).serialize(&mut file)?;
                    file.write_all(data)?;
                }
                WALRecord::Delete { table_id, row_id } => {
                    file.write_all(b"D")?;
                    table_id.serialize(&mut file)?;
                    row_id.serialize(&mut file)?;
                }
                WALRecord::Update {
                    table_id,
                    row_id,
                    column,
                    data,
                } => {
                    file.write_all(b"U")?;
                    table_id.serialize(&mut file)?;
                    row_id.serialize(&mut file)?;
                    column.serialize(&mut file)?;
                    (data.len() as u32).serialize(&mut file)?;
                    file.write_all(data)?;
                }
                WALRecord::ColumnWrite {
                    table_id,
                    col_id,
                    page_id,
                    data,
                } => {
                    file.write_all(b"W")?;
                    table_id.serialize(&mut file)?;
                    col_id.serialize(&mut file)?;
                    page_id.serialize(&mut file)?;
                    (data.len() as u32).serialize(&mut file)?;
                    file.write_all(data)?;
                }
                WALRecord::LocalWALData { data } => {
                    file.write_all(b"L")?;
                    (data.len() as u32).serialize(&mut file)?;
                    file.write_all(data)?;
                }
                WALRecord::Commit { transaction_id } => {
                    file.write_all(b"C")?;
                    transaction_id.serialize(&mut file)?;
                }
                WALRecord::Rollback { transaction_id } => {
                    file.write_all(b"R")?;
                    transaction_id.serialize(&mut file)?;
                }
                WALRecord::Checkpoint => {
                    file.write_all(b"K")?;
                }
            }
        }
        file.flush()?;
        Ok(())
    }

    /// Load and deserialize WAL records from disk.
    ///
    /// Reads the WAL file at `self.path` and populates `self.records` with
    /// the deserialized records. Returns an error if the file doesn't exist
    /// or is corrupt.
    ///
    /// # Format
    ///
    /// Each record on disk starts with a single-byte type tag:
    /// - `I` → Insert { table_id: u64, data_len: u32, data: [u8; data_len] }
    /// - `D` → Delete { table_id: u64, row_id: u64 }
    /// - `U` → Update { table_id: u64, row_id: u64, column: u32, data_len: u32, data }
    /// - `W` → ColumnWrite { table_id: u64, col_id: u32, page_id: u64, data_len: u32, data }
    /// - `C` → Commit { transaction_id: u64 }
    /// - `R` → Rollback { transaction_id: u64 }
    /// - `K` → Checkpoint
    pub fn load_from_disk(&mut self) -> std::io::Result<()> {
        use kuzu_common::serialization::Deserialize;
        use std::io::{BufReader, Read};

        if !self.path.exists() {
            return Ok(()); // Nothing to recover
        }

        let file = std::fs::File::open(&self.path)?;
        let mut reader = BufReader::new(file);

        // Read all data into a buffer for easier parsing
        let mut buffer = Vec::new();
        reader.read_to_end(&mut buffer)?;

        if buffer.is_empty() {
            return Ok(());
        }

        let mut cursor = std::io::Cursor::new(&buffer);

        while cursor.position() < buffer.len() as u64 {
            let mut tag_buf = [0u8; 1];
            if cursor.read_exact(&mut tag_buf).is_err() {
                break;
            }
            let tag = tag_buf[0];

            match tag {
                b'I' => {
                    let table_id = u64::deserialize(&mut cursor)?;
                    let data_len = u32::deserialize(&mut cursor)? as usize;
                    let mut data = vec![0u8; data_len];
                    cursor.read_exact(&mut data)?;
                    self.records.push(WALRecord::Insert { table_id, data });
                }
                b'D' => {
                    let table_id = u64::deserialize(&mut cursor)?;
                    let row_id = u64::deserialize(&mut cursor)?;
                    self.records.push(WALRecord::Delete { table_id, row_id });
                }
                b'U' => {
                    let table_id = u64::deserialize(&mut cursor)?;
                    let row_id = u64::deserialize(&mut cursor)?;
                    let column = u32::deserialize(&mut cursor)?;
                    let data_len = u32::deserialize(&mut cursor)? as usize;
                    let mut data = vec![0u8; data_len];
                    cursor.read_exact(&mut data)?;
                    self.records.push(WALRecord::Update {
                        table_id,
                        row_id,
                        column,
                        data,
                    });
                }
                b'W' => {
                    let table_id = u64::deserialize(&mut cursor)?;
                    let col_id = u32::deserialize(&mut cursor)?;
                    let page_id = u64::deserialize(&mut cursor)?;
                    let data_len = u32::deserialize(&mut cursor)? as usize;
                    let mut data = vec![0u8; data_len];
                    cursor.read_exact(&mut data)?;
                    self.records.push(WALRecord::ColumnWrite {
                        table_id,
                        col_id,
                        page_id,
                        data,
                    });
                }
                b'C' => {
                    let transaction_id = u64::deserialize(&mut cursor)?;
                    self.records.push(WALRecord::Commit { transaction_id });
                }
                b'R' => {
                    let transaction_id = u64::deserialize(&mut cursor)?;
                    self.records.push(WALRecord::Rollback { transaction_id });
                }
                b'L' => {
                    let data_len = u32::deserialize(&mut cursor)? as usize;
                    let mut data = vec![0u8; data_len];
                    cursor.read_exact(&mut data)?;
                    self.records.push(WALRecord::LocalWALData { data });
                }
                b'K' => {
                    self.records.push(WALRecord::Checkpoint);
                }
                _ => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("WAL: unknown record tag byte: 0x{:02x}", tag),
                    ));
                }
            }
        }

        self.total_size = buffer.len();
        self.is_dirty = !self.records.is_empty();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wal_append_clear() {
        let dir = tempfile::tempdir().unwrap();
        let mut wal = WAL::new(dir.path().join("wal.log"));
        assert!(wal.is_empty());
        wal.append(WALRecord::Insert {
            table_id: 1,
            data: vec![1, 2, 3],
        });
        assert_eq!(wal.len(), 1);
        assert!(wal.is_dirty());
        wal.append(WALRecord::Commit { transaction_id: 42 });
        assert_eq!(wal.len(), 2);
        wal.clear();
        assert!(wal.is_empty());
        assert!(!wal.is_dirty());
    }

    #[test]
    fn test_wal_replay() {
        let dir = tempfile::tempdir().unwrap();
        let mut wal = WAL::new(dir.path().join("wal.log"));
        wal.append(WALRecord::Insert {
            table_id: 1,
            data: vec![10, 20],
        });
        wal.append(WALRecord::Commit { transaction_id: 1 });
        let mut count = 0;
        wal.replay(|record| {
            count += 1;
            if let WALRecord::Insert { table_id, data } = record {
                assert_eq!(*table_id, 1);
                assert_eq!(data, &[10, 20]);
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_wal_flush_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("wal.log");
        let mut wal = WAL::new(wal_path.clone());
        wal.append(WALRecord::Insert {
            table_id: 1,
            data: vec![1, 2, 3],
        });
        wal.flush_to_disk().unwrap();
        assert!(wal_path.exists());
        assert!(std::fs::metadata(&wal_path).unwrap().len() > 0);
    }
}
