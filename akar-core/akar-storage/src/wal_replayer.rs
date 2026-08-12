//! WAL Replayer — reads the WAL file and applies records to recover state after a crash.
//!
//! On database open, if a non-empty WAL file exists, the replayer reads
//! all records sequentially and applies them to the StorageManager.
//! Transactions that were rolled back are skipped; only committed
//! transactions' writes are applied.
//!
//! Ported from C++ `src/storage/wal/wal_replayer.cpp` and
//! Ladybug `ladybug/src/storage/wal/records/`.

use crate::wal::WALRecord;
use std::collections::HashSet;
use std::path::Path;

/// Result of replaying the WAL.
#[derive(Debug)]
pub struct ReplayResult {
    /// Number of records replayed.
    pub records_replayed: usize,
    /// Number of records skipped (rolled-back txns).
    pub records_skipped: usize,
    /// Set of committed transaction IDs found in the WAL.
    pub committed_txns: HashSet<u64>,
}

/// Reads and applies WAL records for crash recovery.
pub struct WALReplayer;

impl WALReplayer {
    /// Replay the WAL located at `wal_path`, applying each record via `apply_fn`.
    ///
    /// The caller is responsible for providing an `apply_fn` that knows how
    /// to apply each `WALRecord` variant to the storage engine.
    ///
    /// Rolled-back transactions' records are skipped.
    pub fn replay<F>(wal_path: &Path, mut apply_fn: F) -> std::io::Result<ReplayResult>
    where
        F: FnMut(&WALRecord) -> std::io::Result<()>,
    {
        use std::io::{BufReader, Read};

        if !wal_path.exists() {
            return Ok(ReplayResult {
                records_replayed: 0,
                records_skipped: 0,
                committed_txns: HashSet::new(),
            });
        }

        let file = std::fs::File::open(wal_path)?;
        let mut reader = BufReader::new(file);

        let mut buffer = Vec::new();
        reader.read_to_end(&mut buffer)?;

        if buffer.is_empty() {
            return Ok(ReplayResult {
                records_replayed: 0,
                records_skipped: 0,
                committed_txns: HashSet::new(),
            });
        }

        // Deserialize via the real WAL reader (single pass — the previous
        // manual byte-scanning "first pass" was dead code: it collected an
        // `all_records` vector that was never used, P52.52).
        let mut wal = crate::wal::WAL::new(wal_path.to_path_buf());
        wal.load_from_disk()?;

        let mut committed_txns: HashSet<u64> = HashSet::new();
        let mut records_replayed = 0usize;
        let mut records_skipped = 0usize;
        // Data records for the current txn batch. They are applied only when
        // the batch ends with a Commit marker; a Rollback marker discards them,
        // so a rolled-back insert is no longer replayed (P52.52).
        let mut pending: Vec<&WALRecord> = Vec::new();

        for record in wal.records() {
            match record {
                WALRecord::Commit { transaction_id } => {
                    committed_txns.insert(*transaction_id);
                    for r in pending.drain(..) {
                        apply_fn(r)?;
                        records_replayed += 1;
                    }
                }
                WALRecord::Rollback { .. } => {
                    records_skipped += pending.len();
                    pending.clear();
                }
                WALRecord::Checkpoint => {
                    // Everything before a checkpoint is already durable — flush
                    // the pending batch as committed.
                    for r in pending.drain(..) {
                        apply_fn(r)?;
                        records_replayed += 1;
                    }
                }
                _ => {
                    pending.push(record);
                }
            }
        }

        // Trailing data records with no closing marker are treated as committed.
        for r in pending.drain(..) {
            apply_fn(r)?;
            records_replayed += 1;
        }

        Ok(ReplayResult {
            records_replayed,
            records_skipped,
            committed_txns,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wal::{WAL, WALRecord};
    use tempfile::TempDir;

    #[test]
    fn test_replay_empty_wal() {
        let dir = TempDir::new().unwrap();
        let wal_path = dir.path().join("wal.log");

        let mut count = 0;
        let result = WALReplayer::replay(&wal_path, |_| {
            count += 1;
            Ok(())
        })
        .unwrap();

        assert_eq!(result.records_replayed, 0);
        assert_eq!(count, 0);
    }

    #[test]
    fn test_replay_with_records() {
        let dir = TempDir::new().unwrap();
        let wal_path = dir.path().join("wal.log");

        // Create a WAL with known records
        let mut wal = WAL::new(wal_path.clone());
        wal.append(WALRecord::Insert {
            table_id: 1,
            data: vec![1, 2, 3],
        });
        wal.append(WALRecord::Commit { transaction_id: 1 });
        wal.flush_to_disk().unwrap();

        let mut replayed = Vec::new();
        let result = WALReplayer::replay(&wal_path, |rec| {
            replayed.push(format!("{:?}", rec));
            Ok(())
        })
        .unwrap();

        assert!(result.records_replayed > 0);
    }

    #[test]
    fn test_replay_skips_rolled_back_batch() {
        // P52.52: a rolled-back batch's data records must be discarded, not
        // replayed after recovery.
        let dir = TempDir::new().unwrap();
        let wal_path = dir.path().join("wal.log");

        let mut wal = WAL::new(wal_path.clone());
        wal.append(WALRecord::Insert {
            table_id: 1,
            data: vec![1, 2, 3],
        });
        wal.append(WALRecord::Rollback { transaction_id: 7 });
        wal.append(WALRecord::Insert {
            table_id: 1,
            data: vec![4, 5, 6],
        });
        wal.append(WALRecord::Commit { transaction_id: 8 });
        wal.flush_to_disk().unwrap();

        let mut replayed = Vec::new();
        let result = WALReplayer::replay(&wal_path, |rec| {
            replayed.push(rec.clone());
            Ok(())
        })
        .unwrap();

        assert_eq!(result.records_replayed, 1, "only the committed insert is replayed");
        assert_eq!(replayed.len(), 1);
        assert!(matches!(&replayed[0], WALRecord::Insert { data, .. } if data == &vec![4, 5, 6]));
    }
}
