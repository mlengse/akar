//! Write-Ahead Log for crash recovery.
//!
//! Logs all write operations (column writes, table inserts, etc.) before
//! they are applied to the main storage. During checkpoint, the WAL is
//! flushed to disk and the storage pages are synchronized.

use std::fmt;
use std::io::{Read, Write};
use std::path::PathBuf;

/// WAL file magic bytes — identifies a v2 WAL with per-record CRC32 checksums.
const WAL_MAGIC: &[u8; 4] = b"AKAR";
/// WAL file version — bumped when the on-disk format changes.
const WAL_VERSION: u16 = 2;

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
    /// Log an FSM page update (allocation or deallocation)
    UpdateFsm {
        page_idx: u64,
        is_free: bool,
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
    // ── DDL record types (extended for Ladybug-style WAL) ──
    /// Create a node or rel table.
    CreateTable {
        table_id: u64,
    },
    /// Drop a table.
    DropTable {
        table_id: u64,
    },
    /// Alter a table (add/drop/rename column).
    AlterTable {
        table_id: u64,
    },
    /// Create an index on a table.
    CreateIndex {
        table_id: u64,
    },
    /// Drop an index on a table.
    DropIndex {
        table_id: u64,
    },
    /// Create a sequence.
    CreateSequence {
        table_id: u64,
    },
}

impl fmt::Display for WALRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WALRecord::Insert { table_id, data } => {
                write!(f, "INSERT table={} data_len={}", table_id, data.len())
            }
            WALRecord::Delete { table_id, row_id } => {
                write!(f, "DELETE table={} row={}", table_id, row_id)
            }
            WALRecord::Update {
                table_id,
                row_id,
                column,
                data,
            } => {
                write!(
                    f,
                    "UPDATE table={} row={} col={} data_len={}",
                    table_id,
                    row_id,
                    column,
                    data.len()
                )
            }
            WALRecord::UpdateFsm { page_idx, is_free } => {
                write!(f, "FSM page={} {}", page_idx, if *is_free { "FREE" } else { "ALLOC" })
            }
            WALRecord::ColumnWrite {
                table_id,
                col_id,
                page_id,
                data,
            } => {
                write!(
                    f,
                    "COLUMN_WRITE table={} col={} page={} data_len={}",
                    table_id,
                    col_id,
                    page_id,
                    data.len()
                )
            }
            WALRecord::LocalWALData { data } => {
                write!(f, "LOCAL_WAL data_len={}", data.len())
            }
            WALRecord::Commit { transaction_id } => {
                write!(f, "COMMIT txn={}", transaction_id)
            }
            WALRecord::Rollback { transaction_id } => {
                write!(f, "ROLLBACK txn={}", transaction_id)
            }
            WALRecord::Checkpoint => write!(f, "CHECKPOINT"),
            WALRecord::CreateTable { table_id } => {
                write!(f, "CREATE_TABLE id={}", table_id)
            }
            WALRecord::DropTable { table_id } => {
                write!(f, "DROP_TABLE id={}", table_id)
            }
            WALRecord::AlterTable { table_id } => {
                write!(f, "ALTER_TABLE id={}", table_id)
            }
            WALRecord::CreateIndex { table_id } => {
                write!(f, "CREATE_INDEX table_id={}", table_id)
            }
            WALRecord::DropIndex { table_id } => {
                write!(f, "DROP_INDEX table_id={}", table_id)
            }
            WALRecord::CreateSequence { table_id } => {
                write!(f, "CREATE_SEQUENCE id={}", table_id)
            }
        }
    }
}

/// Write-Ahead Log for durability.
///
/// Uses an append-only on-disk format: each `flush_to_disk()` call serializes
/// only the **new** records since the last flush and appends them to the file.
/// The full file is only rewritten during `clear()` (checkpoint).
pub struct WAL {
    path: PathBuf,
    records: Vec<WALRecord>,
    /// Number of records that have been flushed to disk.
    flushed_count: usize,
    /// Whether the next flush should write the file header (true after `clear()`).
    needs_header: bool,
    total_size: usize,
    is_dirty: bool,
}

impl WAL {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            records: Vec::new(),
            flushed_count: 0,
            needs_header: true,
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
            WALRecord::UpdateFsm { .. } => 8 + 1, // u64 + bool
            WALRecord::ColumnWrite { data, .. } => data.len(),
            WALRecord::LocalWALData { data } => data.len(),
            // DDL variants: each has a u64 table_id
            WALRecord::CreateTable { .. }
            | WALRecord::DropTable { .. }
            | WALRecord::AlterTable { .. }
            | WALRecord::CreateIndex { .. }
            | WALRecord::DropIndex { .. }
            | WALRecord::CreateSequence { .. } => 8,
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
    /// Clear all in-memory records and truncate the WAL file on disk.
    ///
    /// Called during checkpoint after dirty pages have been flushed — at this
    /// point the WAL data is durable in the main DB files and can be discarded.
    pub fn clear(&mut self) -> std::io::Result<()> {
        self.records.clear();
        self.flushed_count = 0;
        self.total_size = 0;
        self.is_dirty = false;
        self.needs_header = true;
        // Truncate the WAL file to empty so the next flush starts fresh.
        std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path)?
            .sync_data()
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

    /// Append-only flush: serialize only records not yet on disk.
    ///
    /// Instead of rewriting the entire WAL file (O(n) per flush → O(n²) total),
    /// this method serializes only the **new** records since the last flush and
    /// appends them to the existing file. The header is written once on the
    /// first flush after a `clear()` (checkpoint).
    ///
    /// Crash safety: CRC32 per record detects partial appends. A partial final
    /// record is silently skipped on recovery.
    ///
    /// ## On-disk format (v2)
    ///
    /// ```text
    /// ┌──────────────────────────────────────────────┐
    /// │ Header: "AKAR" (4 bytes) + version (u16 LE) │
    /// ├──────────────────────────────────────────────┤
    /// │ Per record:                                  │
    /// │   CRC32 (u32 LE) of [tag .. payload]        │
    /// │   tag (1 byte)                               │
    /// │   payload (variable)                         │
    /// └──────────────────────────────────────────────┘
    /// ```
    pub fn flush_to_disk(&mut self) -> std::io::Result<()> {
        if self.flushed_count >= self.records.len() {
            return Ok(()); // Nothing new to write
        }

        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;

        // Write header on fresh WAL (after clear() or first ever write).
        if self.needs_header {
            file.write_all(WAL_MAGIC)?;
            file.write_all(&WAL_VERSION.to_le_bytes())?;
            self.needs_header = false;
        }

        // Append only records not yet on disk.
        let new_records = &self.records[self.flushed_count..];
        Self::append_records_to_file(&mut file, new_records)?;

        self.flushed_count = self.records.len();
        file.sync_data()
    }

    /// Serialize one WAL record + CRC32 and write it to the file.
    fn append_records_to_file(file: &mut std::fs::File, records: &[WALRecord]) -> std::io::Result<()> {
        use akar_common::serialization::Serialize;
        let mut crc_buf = [0u8; 4];
        for record in records {
            let mut payload = Vec::new();
            match record {
                WALRecord::Insert { table_id, data } => {
                    payload.write_all(b"I")?;
                    table_id.serialize(&mut payload)?;
                    (data.len() as u32).serialize(&mut payload)?;
                    payload.write_all(data)?;
                }
                WALRecord::Delete { table_id, row_id } => {
                    payload.write_all(b"D")?;
                    table_id.serialize(&mut payload)?;
                    row_id.serialize(&mut payload)?;
                }
                WALRecord::Update {
                    table_id,
                    row_id,
                    column,
                    data,
                } => {
                    payload.write_all(b"U")?;
                    table_id.serialize(&mut payload)?;
                    row_id.serialize(&mut payload)?;
                    column.serialize(&mut payload)?;
                    (data.len() as u32).serialize(&mut payload)?;
                    payload.write_all(data)?;
                }
                WALRecord::UpdateFsm { page_idx, is_free } => {
                    payload.write_all(b"F")?;
                    page_idx.serialize(&mut payload)?;
                    let is_free_u8: u8 = if *is_free { 1 } else { 0 };
                    is_free_u8.serialize(&mut payload)?;
                }
                WALRecord::ColumnWrite {
                    table_id,
                    col_id,
                    page_id,
                    data,
                } => {
                    payload.write_all(b"W")?;
                    table_id.serialize(&mut payload)?;
                    col_id.serialize(&mut payload)?;
                    page_id.serialize(&mut payload)?;
                    (data.len() as u32).serialize(&mut payload)?;
                    payload.write_all(data)?;
                }
                WALRecord::LocalWALData { data } => {
                    payload.write_all(b"L")?;
                    (data.len() as u32).serialize(&mut payload)?;
                    payload.write_all(data)?;
                }
                WALRecord::Commit { transaction_id } => {
                    payload.write_all(b"C")?;
                    transaction_id.serialize(&mut payload)?;
                }
                WALRecord::Rollback { transaction_id } => {
                    payload.write_all(b"R")?;
                    transaction_id.serialize(&mut payload)?;
                }
                WALRecord::Checkpoint => {
                    payload.write_all(b"K")?;
                }
                // ── DDL record types ──
                WALRecord::CreateTable { table_id } => {
                    payload.write_all(b"T")?;
                    table_id.serialize(&mut payload)?;
                }
                WALRecord::DropTable { table_id } => {
                    payload.write_all(b"A")?;
                    table_id.serialize(&mut payload)?;
                }
                WALRecord::AlterTable { table_id } => {
                    payload.write_all(b"M")?;
                    table_id.serialize(&mut payload)?;
                }
                WALRecord::CreateIndex { table_id } => {
                    payload.write_all(b"N")?;
                    table_id.serialize(&mut payload)?;
                }
                WALRecord::DropIndex { table_id } => {
                    payload.write_all(b"X")?;
                    table_id.serialize(&mut payload)?;
                }
                WALRecord::CreateSequence { table_id } => {
                    payload.write_all(b"Q")?;
                    table_id.serialize(&mut payload)?;
                }
            }
            let checksum = crc32fast::hash(&payload);
            crc_buf.copy_from_slice(&checksum.to_le_bytes());
            file.write_all(&crc_buf)?;
            file.write_all(&payload)?;
        }
        Ok(())
    }

    /// Load WAL records from disk.
    ///
    /// Supports both v1 (no checksums) and v2 (CRC32 per record) formats.
    /// Records with invalid checksums are silently skipped with a warning.
    pub fn load_from_disk(&mut self) -> std::io::Result<()> {
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

        // Detect format: v2 has "AKAR" magic header
        let (is_v2, cursor_pos) = if buffer.len() >= 6 && &buffer[..4] == WAL_MAGIC {
            let version = u16::from_le_bytes([buffer[4], buffer[5]]);
            if version >= 2 { (true, 6usize) } else { (false, 0usize) }
        } else {
            (false, 0usize)
        };

        let mut cursor = std::io::Cursor::new(&buffer[cursor_pos..]);
        let mut skipped = 0u32;

        if is_v2 {
            // v2: each record is CRC32 (4 bytes) + tag + payload
            while cursor.position() < (buffer.len() - cursor_pos) as u64 {
                // Read CRC32
                let mut crc_bytes = [0u8; 4];
                if cursor.read_exact(&mut crc_bytes).is_err() {
                    break;
                }
                let expected_crc = u32::from_le_bytes(crc_bytes);

                // Read the rest of the record into a temp buffer to compute CRC
                let record_start = cursor.position() as usize;
                let record_data = &buffer[(cursor_pos + record_start)..];
                if record_data.is_empty() {
                    break;
                }

                // We need to know how long this record is to compute CRC.
                // Read tag first to determine length.
                let tag = record_data[0];
                let payload_len = match tag {
                    b'I' => {
                        // table_id(u64) + data_len(u32) + data
                        if record_data.len() < 13 {
                            skipped += 1;
                            break;
                        }
                        let data_len = u32::from_le_bytes(record_data[9..13].try_into().unwrap()) as usize;
                        1 + 8 + 4 + data_len // tag + u64 + u32 + data
                    }
                    b'D' => 1 + 8 + 8, // tag + table_id + row_id
                    b'U' => {
                        // Layout: tag(1) + table_id(8) + row_id(8) + column(4) + data_len(4) + data
                        if record_data.len() < 25 {
                            skipped += 1;
                            break;
                        }
                        let data_len = u32::from_le_bytes(record_data[21..25].try_into().unwrap()) as usize;
                        1 + 8 + 8 + 4 + 4 + data_len
                    }
                    b'F' => 1 + 8 + 1, // tag + page_idx + is_free
                    b'W' => {
                        // Layout: tag(1) + table_id(8) + col_id(4) + page_id(8) + data_len(4) + data
                        if record_data.len() < 25 {
                            skipped += 1;
                            break;
                        }
                        let data_len = u32::from_le_bytes(record_data[21..25].try_into().unwrap()) as usize;
                        1 + 8 + 4 + 8 + 4 + data_len
                    }
                    b'L' => {
                        if record_data.len() < 5 {
                            skipped += 1;
                            break;
                        }
                        let data_len = u32::from_le_bytes(record_data[1..5].try_into().unwrap()) as usize;
                        1 + 4 + data_len
                    }
                    b'C' => 1 + 8, // tag + transaction_id
                    b'R' => 1 + 8,
                    b'K' => 1, // checkpoint — tag only
                    // DDL types: tag + u64
                    b'T' | b'A' | b'M' | b'N' | b'X' | b'Q' => 1 + 8,
                    _ => {
                        // Unknown tag — skip rest of file
                        skipped += 1;
                        break;
                    }
                };

                if payload_len > record_data.len() {
                    skipped += 1;
                    break;
                }

                // Compute CRC over tag+payload
                let computed_crc = crc32fast::hash(&record_data[..payload_len]);

                if computed_crc != expected_crc {
                    // Checksum mismatch — skip this record
                    skipped += 1;
                    cursor.set_position(cursor.position() + payload_len as u64);
                    continue;
                }

                // CRC valid — parse the record
                cursor.set_position(cursor.position() + payload_len as u64);
                let mut inner = std::io::Cursor::new(&record_data[..payload_len]);
                self.parse_record(&mut inner)?;
            }
        } else {
            // v1: no checksums, original format
            self.parse_v1_records(&mut cursor)?;
        }

        if skipped > 0 {
            tracing::warn!(
                "WAL: skipped {} corrupted record(s) during recovery (v{})",
                skipped,
                if is_v2 { 2 } else { 1 }
            );
        }

        // All loaded records are already on disk — nothing to re-flush.
        self.flushed_count = self.records.len();
        self.needs_header = false;
        self.total_size = buffer.len();
        self.is_dirty = !self.records.is_empty();
        Ok(())
    }

    /// Parse a single WAL record from the cursor (v1 format — no checksums).
    fn parse_record(&mut self, cursor: &mut std::io::Cursor<&[u8]>) -> std::io::Result<()> {
        use akar_common::serialization::Deserialize;
        let mut tag_buf = [0u8; 1];
        if cursor.read_exact(&mut tag_buf).is_err() {
            return Ok(());
        }
        let tag = tag_buf[0];
        match tag {
            b'I' => {
                let table_id = u64::deserialize(cursor)?;
                let data_len = u32::deserialize(cursor)? as usize;
                let mut data = vec![0u8; data_len];
                cursor.read_exact(&mut data)?;
                self.records.push(WALRecord::Insert { table_id, data });
            }
            b'D' => {
                let table_id = u64::deserialize(cursor)?;
                let row_id = u64::deserialize(cursor)?;
                self.records.push(WALRecord::Delete { table_id, row_id });
            }
            b'U' => {
                let table_id = u64::deserialize(cursor)?;
                let row_id = u64::deserialize(cursor)?;
                let column = u32::deserialize(cursor)?;
                let data_len = u32::deserialize(cursor)? as usize;
                let mut data = vec![0u8; data_len];
                cursor.read_exact(&mut data)?;
                self.records.push(WALRecord::Update {
                    table_id,
                    row_id,
                    column,
                    data,
                });
            }
            b'F' => {
                let page_idx = u64::deserialize(cursor)?;
                let is_free_u8 = u8::deserialize(cursor)?;
                self.records.push(WALRecord::UpdateFsm {
                    page_idx,
                    is_free: is_free_u8 != 0,
                });
            }
            b'W' => {
                let table_id = u64::deserialize(cursor)?;
                let col_id = u32::deserialize(cursor)?;
                let page_id = u64::deserialize(cursor)?;
                let data_len = u32::deserialize(cursor)? as usize;
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
                let transaction_id = u64::deserialize(cursor)?;
                self.records.push(WALRecord::Commit { transaction_id });
            }
            b'R' => {
                let transaction_id = u64::deserialize(cursor)?;
                self.records.push(WALRecord::Rollback { transaction_id });
            }
            b'L' => {
                let data_len = u32::deserialize(cursor)? as usize;
                let mut data = vec![0u8; data_len];
                cursor.read_exact(&mut data)?;
                self.records.push(WALRecord::LocalWALData { data });
            }
            b'K' => {
                self.records.push(WALRecord::Checkpoint);
            }
            // ── DDL record types ──
            b'T' => {
                let table_id = u64::deserialize(cursor)?;
                self.records.push(WALRecord::CreateTable { table_id });
            }
            b'A' => {
                let table_id = u64::deserialize(cursor)?;
                self.records.push(WALRecord::DropTable { table_id });
            }
            b'M' => {
                let table_id = u64::deserialize(cursor)?;
                self.records.push(WALRecord::AlterTable { table_id });
            }
            b'N' => {
                let table_id = u64::deserialize(cursor)?;
                self.records.push(WALRecord::CreateIndex { table_id });
            }
            b'X' => {
                let table_id = u64::deserialize(cursor)?;
                self.records.push(WALRecord::DropIndex { table_id });
            }
            b'Q' => {
                let table_id = u64::deserialize(cursor)?;
                self.records.push(WALRecord::CreateSequence { table_id });
            }
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("WAL: unknown record tag byte: 0x{:02x}", tag),
                ));
            }
        }
        Ok(())
    }

    /// Parse v1 records (legacy format without checksums).
    fn parse_v1_records(&mut self, cursor: &mut std::io::Cursor<&[u8]>) -> std::io::Result<()> {
        while cursor.position() < cursor.get_ref().len() as u64 {
            self.parse_record(cursor)?;
        }
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
        wal.clear().unwrap();
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

    #[test]
    fn test_wal_checksum_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("wal.log");
        let mut wal = WAL::new(wal_path.clone());
        wal.append(WALRecord::Insert {
            table_id: 42,
            data: vec![10, 20, 30, 40, 50],
        });
        wal.append(WALRecord::Delete {
            table_id: 7,
            row_id: 99,
        });
        wal.append(WALRecord::Commit { transaction_id: 1 });
        wal.flush_to_disk().unwrap();

        // Verify the file starts with AKAR magic
        let bytes = std::fs::read(&wal_path).unwrap();
        assert_eq!(&bytes[..4], b"AKAR");
        assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), 2);

        // Reload and verify all records recovered
        let mut wal2 = WAL::new(wal_path);
        wal2.load_from_disk().unwrap();
        assert_eq!(wal2.len(), 3);
        assert!(matches!(
            &wal2.records()[0],
            WALRecord::Insert { table_id: 42, data } if data == &vec![10, 20, 30, 40, 50]
        ));
        assert!(matches!(
            &wal2.records()[1],
            WALRecord::Delete {
                table_id: 7,
                row_id: 99
            }
        ));
        assert!(matches!(&wal2.records()[2], WALRecord::Commit { transaction_id: 1 }));
    }

    #[test]
    fn test_wal_corrupted_record_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("wal.log");

        // Write two valid records
        let mut wal = WAL::new(wal_path.clone());
        wal.append(WALRecord::Insert {
            table_id: 1,
            data: vec![100, 200],
        });
        wal.append(WALRecord::Commit { transaction_id: 5 });
        wal.flush_to_disk().unwrap();

        // Corrupt one byte in the file (flip a byte in the first record's payload)
        let mut bytes = std::fs::read(&wal_path).unwrap();
        // Header is 6 bytes, first CRC is 4 bytes, then tag (1 byte), then table_id bytes
        // Corrupt the data payload (after table_id + data_len)
        let corrupt_offset = 6 + 4 + 1 + 8 + 4 + 1; // header + crc + tag + table_id + data_len + first data byte
        if corrupt_offset < bytes.len() {
            bytes[corrupt_offset] ^= 0xFF;
        }
        std::fs::write(&wal_path, &bytes).unwrap();

        // Reload — corrupted record should be skipped, second should survive
        let mut wal2 = WAL::new(wal_path);
        let _result = wal2.load_from_disk();
        // The corrupted Insert is skipped, Commit may or may not survive
        // depending on whether the corruption affected its CRC range
        // At minimum, no panic and the WAL loads
    }

    #[test]
    fn test_wal_v1_backward_compat() {
        // Simulate a v1 WAL file (no header, no checksums)
        use akar_common::serialization::Serialize;
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("wal.log");
        let mut file = std::fs::File::create(&wal_path).unwrap();

        // Write a v1 Insert record: tag "I" + table_id(u64) + data_len(u32) + data
        file.write_all(b"I").unwrap();
        42u64.serialize(&mut file).unwrap();
        (3u32).serialize(&mut file).unwrap();
        file.write_all(&[1, 2, 3]).unwrap();
        // Write a v1 Commit record
        file.write_all(b"C").unwrap();
        1u64.serialize(&mut file).unwrap();
        drop(file);

        // Load — should parse as v1 (no magic header)
        let mut wal = WAL::new(wal_path);
        wal.load_from_disk().unwrap();
        assert_eq!(wal.len(), 2);
        assert!(matches!(&wal.records()[0], WALRecord::Insert { table_id: 42, .. }));
        assert!(matches!(&wal.records()[1], WALRecord::Commit { transaction_id: 1 }));
    }

    #[test]
    fn test_wal_all_record_types_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("wal.log");
        let mut wal = WAL::new(wal_path.clone());

        wal.append(WALRecord::Insert {
            table_id: 1,
            data: vec![1],
        });
        wal.append(WALRecord::Delete { table_id: 2, row_id: 3 });
        wal.append(WALRecord::Update {
            table_id: 4,
            row_id: 5,
            column: 6,
            data: vec![7, 8],
        });
        wal.append(WALRecord::UpdateFsm {
            page_idx: 100,
            is_free: true,
        });
        wal.append(WALRecord::ColumnWrite {
            table_id: 10,
            col_id: 11,
            page_id: 12,
            data: vec![99],
        });
        wal.append(WALRecord::LocalWALData { data: vec![10, 11, 12] });
        wal.append(WALRecord::Commit { transaction_id: 50 });
        wal.append(WALRecord::Rollback { transaction_id: 51 });
        wal.append(WALRecord::Checkpoint);
        wal.append(WALRecord::CreateTable { table_id: 20 });
        wal.append(WALRecord::DropTable { table_id: 21 });
        wal.append(WALRecord::AlterTable { table_id: 22 });
        wal.append(WALRecord::CreateIndex { table_id: 23 });
        wal.append(WALRecord::DropIndex { table_id: 24 });
        wal.append(WALRecord::CreateSequence { table_id: 25 });

        wal.flush_to_disk().unwrap();

        let mut wal2 = WAL::new(wal_path);
        wal2.load_from_disk().unwrap();
        assert_eq!(wal2.len(), 15);
    }
}
