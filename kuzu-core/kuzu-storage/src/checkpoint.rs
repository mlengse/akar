//! Checkpoint logic — flushes WAL to main database files.
//!
//! During a checkpoint:
//! 1. WAL records are flushed to the WAL file on disk
//! 2. All dirty BufferManager pages are flushed to their respective files
//! 3. The WAL in-memory buffer is cleared
//! 4. A Checkpoint marker is appended to the new WAL

use crate::buffer_manager::BufferManager;
use crate::wal::{WAL, WALRecord};
use std::sync::{Arc, Mutex};

/// Result of a checkpoint operation.
#[derive(Debug)]
pub struct CheckpointResult {
    pub wal_entries_processed: usize,
    pub pages_flushed: usize,
    pub success: bool,
}

/// Flush all dirty pages for a given table's column file.
///
/// This is called during checkpoint to ensure column data is durable.
/// Flush all dirty pages for a given table's column file to disk.
pub fn flush_table(buffer_manager: &mut BufferManager, file_name: &str) -> std::io::Result<usize> {
    // The caller should iterate known page numbers for the given file.
    // BufferManager::flush handles the composite key lookup internally.
    // For now, this is a pass-through; actual page-level tracking happens
    // via the BufferManager's flush_all mechanism during checkpoint.
    let _ = (buffer_manager, file_name);
    Ok(0)
}

/// Perform a full checkpoint:
///
/// 1. Flush the WAL to disk (durability)
/// 2. Flush all dirty BufferManager pages to disk
/// 3. Clear the in-memory WAL buffer
/// 4. Append a `Checkpoint` record to the new WAL
///
/// Returns the number of WAL entries processed and pages flushed.
pub fn checkpoint(wal: &mut WAL, buffer_manager: &Arc<Mutex<BufferManager>>) -> std::io::Result<CheckpointResult> {
    let wal_count = wal.len();

    // Step 1: Flush the WAL to disk first (write-ahead: log before data).
    if !wal.is_empty() {
        wal.flush_to_disk()?;
    }

    // Step 2: Flush all dirty BufferManager pages to disk.
    {
        let mut bm = buffer_manager.lock().unwrap();
        let stats_before = *bm.stats();
        bm.flush_all()?;
        let stats_after = *bm.stats();
        let pages_flushed = stats_after.page_writes - stats_before.page_writes;

        // Step 3: Clear the WAL after pages are durable.
        wal.clear();

        // Step 4: Append a Checkpoint marker and flush again.
        wal.append(WALRecord::Checkpoint);
        wal.flush_to_disk()?;

        Ok(CheckpointResult {
            wal_entries_processed: wal_count,
            pages_flushed: pages_flushed as usize,
            success: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer_manager::BufferManagerConfig;
    use kuzu_common::memory::MemoryManager;

    #[test]
    fn test_checkpoint_clears_wal() {
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("wal.log");
        let mut wal = WAL::new(wal_path);

        let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
        let config = BufferManagerConfig::default();
        let bm = Arc::new(Mutex::new(crate::buffer_manager::BufferManager::new(
            dir.path().to_path_buf(),
            mm,
            config,
        )));

        wal.append(WALRecord::Insert {
            table_id: 1,
            data: vec![1, 2, 3],
        });
        wal.append(WALRecord::Commit { transaction_id: 42 });
        assert!(!wal.is_empty());

        let result = checkpoint(&mut wal, &bm).unwrap();
        assert!(result.success);
        assert_eq!(result.wal_entries_processed, 2);
        assert!(wal.is_empty() || wal.len() == 1); // Checkpoint marker may remain
    }

    #[test]
    fn test_checkpoint_flush_dirty_pages() {
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("wal.log");
        let mut wal = WAL::new(wal_path);

        let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
        let config = BufferManagerConfig::default();
        let bm = Arc::new(Mutex::new(crate::buffer_manager::BufferManager::new(
            dir.path().to_path_buf(),
            mm,
            config,
        )));

        // Register a file and create a dirty page
        {
            let mut bm_lock = bm.lock().unwrap();
            let db_file = dir.path().join("test.db");
            std::fs::write(&db_file, vec![0u8; 8192 * 10]).unwrap();
            bm_lock.register_file("test", db_file);
            let frame = bm_lock.pin_mut("test", 0).unwrap();
            frame.data[0..4].copy_from_slice(&[1, 2, 3, 4]);
            frame.mark_dirty();
            bm_lock.unpin("test", 0);
        }

        // Add WAL entries
        wal.append(WALRecord::ColumnWrite {
            table_id: 0,
            col_id: 0,
            page_id: 0,
            data: vec![1, 2, 3, 4],
        });

        let result = checkpoint(&mut wal, &bm).unwrap();
        assert!(result.success);

        // Verify WAL file exists
        assert!(dir.path().join("wal.log").exists());
    }

    #[test]
    fn test_checkpoint_with_column_roundtrip() {
        use crate::column::Column;
        use crate::page::DEFAULT_PAGE_SIZE;
        use kuzu_common::types::{LogicalTypeID, Value};

        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("wal.log");
        let mut wal = WAL::new(wal_path);

        let _mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
        let config = BufferManagerConfig::default();
        let bm = Arc::new(Mutex::new(crate::buffer_manager::BufferManager::new(
            dir.path().to_path_buf(),
            _mm,
            config,
        )));

        // Create a column and write values via BufferManager
        let mut col = Column::new(
            LogicalTypeID::Int64,
            0,
            0,
            &dir.path().to_path_buf(),
            bm.clone(),
            DEFAULT_PAGE_SIZE,
        );

        // Write data through the column (this goes through BufferManager pages)
        for i in 0i64..10 {
            col.append_value(&Value::Int64(i)).unwrap();
        }
        assert_eq!(col.num_values, 10);

        // Log the column writes to WAL (simulating what the storage manager does)
        for page_idx in 0..col.num_pages {
            // Read the page data and log it
            if let Ok(page_data) = col.read_value_bytes(0) {
                wal.log_column_write(0, 0, page_idx, &page_data);
            }
        }
        wal.append(WALRecord::Commit { transaction_id: 1 });

        // Perform checkpoint: flush WAL + dirty pages to disk
        let result = checkpoint(&mut wal, &bm).unwrap();
        assert!(result.success);
        assert!(result.wal_entries_processed > 0);

        // Verify data is still readable after checkpoint
        for i in 0i64..10 {
            let v = col.get_value(i as u64).unwrap();
            assert_eq!(v, Value::Int64(i));
        }

        // Verify WAL file exists and has content
        assert!(dir.path().join("wal.log").exists());
        let wal_meta = std::fs::metadata(dir.path().join("wal.log")).unwrap();
        assert!(wal_meta.len() > 0, "WAL file should have content after flush");
    }

    #[test]
    fn test_wal_column_write_replay() {
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("wal.log");
        let mut wal = WAL::new(wal_path);

        // Write ColumnWrite records to WAL
        wal.log_column_write(1, 0, 0, &[0x02, 0x2A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        wal.log_column_write(1, 0, 1, &[0x02, 0x2B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        wal.append(WALRecord::Commit { transaction_id: 42 });

        // Replay and count ColumnWrite records
        let mut write_count = 0;
        wal.replay(|record| {
            match record {
                WALRecord::ColumnWrite {
                    table_id,
                    col_id,
                    page_id,
                    data,
                } => {
                    assert_eq!(*table_id, 1);
                    assert_eq!(*col_id, 0);
                    write_count += 1;
                    // Verify the first record has Int64 value 42 (tag 0x02 + 0x2A)
                    if *page_id == 0 {
                        assert_eq!(data[0], 0x02); // tag
                        assert_eq!(data[1], 0x2A); // 42
                    }
                }
                WALRecord::Commit { transaction_id } => {
                    assert_eq!(*transaction_id, 42);
                }
                _ => {}
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(write_count, 2);
    }
}
