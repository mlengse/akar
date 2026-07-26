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

/// Flush all dirty pages for a given table's column file to disk.
///
/// Iterates all frames belonging to `file_name` in the BufferManager
/// and flushes each dirty page. Returns the number of pages flushed.
pub fn flush_table(buffer_manager: &mut BufferManager, file_name: &str) -> std::io::Result<usize> {
    let dirty_pages: Vec<u64> = buffer_manager.dirty_page_nums_for_file(file_name).into_iter().collect();

    let count = dirty_pages.len();
    for page_num in dirty_pages {
        buffer_manager.flush(file_name, page_num)?;
    }
    Ok(count)
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
        let mut bm = buffer_manager
            .lock()
            .map_err(|e| std::io::Error::other(format!("Lock poisoned: {e}")))?;
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
    use akar_common::memory::MemoryManager;

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
        use akar_common::types::{LogicalTypeID, Value};

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
        let mut col = Column::new(LogicalTypeID::Int64, 0, 0, dir.path(), bm.clone(), DEFAULT_PAGE_SIZE);

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

    // =================================================================
    // P36.7 — Checkpoint persistence tests
    // =================================================================

    #[test]
    fn test_flush_table_per_file() {
        let dir = tempfile::tempdir().unwrap();

        let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
        let config = BufferManagerConfig::default();
        let bm = Arc::new(Mutex::new(crate::buffer_manager::BufferManager::new(
            dir.path().to_path_buf(),
            mm,
            config,
        )));

        // Register a file and create dirty pages
        {
            let mut bm_lock = bm.lock().unwrap();
            let db_file = dir.path().join("col_0_0");
            std::fs::write(&db_file, vec![0u8; 8192 * 3]).unwrap();
            bm_lock.register_file("col_0_0", db_file);

            // Create 2 dirty pages
            for page in 0..2u64 {
                let frame = bm_lock.pin_mut("col_0_0", page).unwrap();
                frame.data[0..4].copy_from_slice(&[1, 2, 3, 4]);
                frame.mark_dirty();
                bm_lock.unpin("col_0_0", page);
            }
        }

        // flush_table should flush exactly 2 dirty pages
        {
            let mut bm_lock = bm.lock().unwrap();
            let flushed = super::flush_table(&mut bm_lock, "col_0_0").unwrap();
            assert_eq!(flushed, 2);
        }

        // After flush, no more dirty pages for this file
        {
            let bm_lock = bm.lock().unwrap();
            assert!(bm_lock.dirty_page_nums_for_file("col_0_0").is_empty());
        }
    }

    #[test]
    fn test_column_metadata_save_load_roundtrip() {
        use crate::column::Column;
        use crate::page::DEFAULT_PAGE_SIZE;
        use akar_common::types::{LogicalTypeID, Value};

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().to_path_buf();
        let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
        let config = BufferManagerConfig::default();
        let bm = Arc::new(Mutex::new(crate::buffer_manager::BufferManager::new(
            db_path.clone(),
            mm,
            config,
        )));

        // Create a column and write data
        let mut col = Column::new(LogicalTypeID::Int64, 0, 0, &db_path, bm.clone(), DEFAULT_PAGE_SIZE);
        for i in 0i64..100 {
            col.append_value(&Value::Int64(i)).unwrap();
        }
        assert_eq!(col.num_values, 100);
        let orig_pages = col.num_pages;
        let orig_offsets = col.page_row_offsets.clone();

        // Save metadata
        col.save_metadata().unwrap();

        // Verify the .meta file exists
        let meta_path = dir.path().join("col_0_0.meta");
        assert!(meta_path.exists(), ".meta file should be created");

        // Create a fresh column (no data) and load metadata
        let mut col2 = Column::new(LogicalTypeID::Int64, 0, 0, &db_path, bm.clone(), DEFAULT_PAGE_SIZE);
        assert_eq!(col2.num_values, 0);
        assert_eq!(col2.num_pages, 0);

        let loaded = col2.load_metadata().unwrap();
        assert!(loaded, "metadata should be loaded");
        assert_eq!(col2.num_values, 100);
        assert_eq!(col2.num_pages, orig_pages);
        assert_eq!(col2.page_row_offsets, orig_offsets);
    }

    #[test]
    fn test_column_persistence_full_roundtrip() {
        use crate::column::Column;
        use crate::page::DEFAULT_PAGE_SIZE;
        use akar_common::types::{LogicalTypeID, Value};

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().to_path_buf();
        let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
        let config = BufferManagerConfig::default();
        let bm = Arc::new(Mutex::new(crate::buffer_manager::BufferManager::new(
            db_path.clone(),
            mm,
            config,
        )));

        // Phase 1: Create column, write data, flush to disk, save metadata
        {
            let mut col = Column::new(LogicalTypeID::Int64, 0, 0, &db_path, bm.clone(), DEFAULT_PAGE_SIZE);
            for i in 0i64..256 {
                col.append_value(&Value::Int64(i)).unwrap();
            }
            assert_eq!(col.num_values, 256);
            col.flush().unwrap();
            col.save_metadata().unwrap();
        }

        // Phase 2: Drop everything, create a fresh column and BufferManager
        drop(bm);
        let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
        let config = BufferManagerConfig::default();
        let bm2 = Arc::new(Mutex::new(crate::buffer_manager::BufferManager::new(
            db_path.clone(),
            mm,
            config,
        )));

        // Phase 3: Recreate column, load metadata, read back data
        {
            let mut col = Column::new(LogicalTypeID::Int64, 0, 0, &db_path, bm2.clone(), DEFAULT_PAGE_SIZE);
            let loaded = col.load_metadata().unwrap();
            assert!(loaded, "metadata should exist from Phase 1");
            assert_eq!(col.num_values, 256);
            assert!(col.num_pages > 0, "should have pages on disk");

            // Read back all values from disk
            for i in 0i64..256 {
                let v = col.get_value(i as u64).unwrap();
                assert_eq!(v, Value::Int64(i), "data mismatch at row {} after restart", i);
            }
        }
    }

    #[test]
    fn test_checkpoint_with_column_write_to_disk() {
        use crate::column::Column;
        use crate::page::DEFAULT_PAGE_SIZE;
        use akar_common::types::{LogicalTypeID, Value};

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

        // Write data through Column
        let mut col = Column::new(LogicalTypeID::Int64, 0, 0, dir.path(), bm.clone(), DEFAULT_PAGE_SIZE);
        for i in 0i64..50 {
            col.append_value(&Value::Int64(i * 10)).unwrap();
        }

        // Log ColumnWrite to WAL
        for page_idx in 0..col.num_pages {
            if let Ok(page_data) = col.read_value_bytes(page_idx * 256) {
                wal.log_column_write(0, 0, page_idx, &page_data);
            }
        }
        wal.append(WALRecord::Commit { transaction_id: 1 });

        // Checkpoint: flush WAL + dirty pages
        let result = checkpoint(&mut wal, &bm).unwrap();
        assert!(result.success);
        assert!(result.pages_flushed > 0);

        // Save column metadata so it can be reconstructed
        col.save_metadata().unwrap();

        // Verify column files exist on disk
        let col_file = dir.path().join("col_0_0");
        assert!(col_file.exists(), "column data file should exist after checkpoint");
        let col_meta = dir.path().join("col_0_0.meta");
        assert!(col_meta.exists(), "column metadata file should exist");

        // Verify data is still readable
        for i in 0i64..50 {
            let v = col.get_value(i as u64).unwrap();
            assert_eq!(v, Value::Int64(i * 10));
        }
    }

    #[test]
    fn test_wal_replay_with_column_write_records() {
        use crate::column::Column;
        use crate::wal_replayer::WALReplayer;
        use akar_common::types::Value;

        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("wal.log");

        // Phase 1: Write ColumnWrite records to WAL
        {
            let mut wal = WAL::new(wal_path.clone());
            for i in 0i64..5 {
                let val = Value::Int64(i * 100);
                let raw = Column::serialize_value(&val);
                wal.log_column_write(0, 0, 0, &raw);
            }
            wal.append(WALRecord::Commit { transaction_id: 10 });
            wal.flush_to_disk().unwrap();
        }

        // Phase 2: Replay the WAL
        let mut replayed_writes = Vec::new();
        let result = WALReplayer::replay(&wal_path, |record| {
            if let WALRecord::ColumnWrite {
                table_id,
                col_id,
                page_id,
                data,
            } = record
            {
                replayed_writes.push((*table_id, *col_id, *page_id, data.clone()));
            }
            Ok(())
        })
        .unwrap();

        assert_eq!(result.records_replayed, 5, "should replay 5 ColumnWrite records");
        assert_eq!(replayed_writes.len(), 5);

        // Verify the data in replayed records
        for (idx, (tid, cid, pid, data)) in replayed_writes.iter().enumerate() {
            assert_eq!(*tid, 0);
            assert_eq!(*cid, 0);
            assert_eq!(*pid, 0);
            // Each record should start with TAG_INT64 = 2
            assert_eq!(data[0], 2, "should be Int64 tag");
            let val = i64::from_le_bytes(data[1..9].try_into().unwrap());
            assert_eq!(val, (idx as i64) * 100);
        }
    }

    #[test]
    fn test_multi_column_checkpoint_persistence() {
        use crate::column::Column;
        use crate::page::DEFAULT_PAGE_SIZE;
        use akar_common::types::{LogicalTypeID, Value};

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().to_path_buf();
        let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
        let config = BufferManagerConfig::default();
        let bm = Arc::new(Mutex::new(crate::buffer_manager::BufferManager::new(
            db_path.clone(),
            mm,
            config,
        )));

        // Create two columns (simulating a table with id + name)
        let mut col_id = Column::new(LogicalTypeID::Int64, 1, 0, &db_path, bm.clone(), DEFAULT_PAGE_SIZE);
        let mut col_name = Column::new(LogicalTypeID::Int64, 1, 1, &db_path, bm.clone(), DEFAULT_PAGE_SIZE);

        // Write data
        for i in 0i64..100 {
            col_id.append_value(&Value::Int64(i)).unwrap();
            col_name.append_value(&Value::Int64(i * 1000)).unwrap();
        }

        // Flush both columns
        col_id.flush().unwrap();
        col_name.flush().unwrap();

        // Save metadata for both
        col_id.save_metadata().unwrap();
        col_name.save_metadata().unwrap();

        // Drop everything
        drop(bm);
        drop(col_id);
        drop(col_name);

        // Create fresh BufferManager and Columns
        let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
        let config = BufferManagerConfig::default();
        let bm2 = Arc::new(Mutex::new(crate::buffer_manager::BufferManager::new(
            db_path.clone(),
            mm,
            config,
        )));

        let mut col_id2 = Column::new(LogicalTypeID::Int64, 1, 0, &db_path, bm2.clone(), DEFAULT_PAGE_SIZE);
        let mut col_name2 = Column::new(LogicalTypeID::Int64, 1, 1, &db_path, bm2.clone(), DEFAULT_PAGE_SIZE);

        col_id2.load_metadata().unwrap();
        col_name2.load_metadata().unwrap();

        assert_eq!(col_id2.num_values, 100);
        assert_eq!(col_name2.num_values, 100);

        // Read back all values from both columns
        for i in 0i64..100 {
            assert_eq!(col_id2.get_value(i as u64).unwrap(), Value::Int64(i));
            assert_eq!(col_name2.get_value(i as u64).unwrap(), Value::Int64(i * 1000));
        }
    }
}
