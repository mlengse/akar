//! Undo Buffer — records old data before writes for transaction rollback.
//!
//! Each write transaction accumulates `UndoRecord`s in its `UndoBuffer`.
//! On rollback, the records are applied in reverse order to restore
//! the pre-write state. On commit, the buffer is cleared.
//!
//! Ported from C++ `src/storage/undo_buffer.cpp`.

use akar_common::error::StorageError;
use akar_transaction::UndoRecord;

/// Accumulates undo records for a single write transaction.
#[derive(Debug, Default)]
pub struct UndoBuffer {
    records: Vec<UndoRecord>,
}

impl UndoBuffer {
    pub fn new() -> Self {
        Self { records: Vec::new() }
    }

    /// Record the old value of a cell before it is overwritten.
    pub fn record(&mut self, table_id: u64, row_id: u64, column: u32, old_data: Vec<u8>) {
        self.records.push(UndoRecord::update(table_id, row_id, column, old_data));
    }

    /// Number of undo records accumulated.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Clear all records (called on successful commit).
    pub fn clear(&mut self) {
        self.records.clear();
    }

    /// Drain all records out of the buffer (consuming).
    /// Used when the caller needs to take ownership of the records
    /// for rollback application.
    pub fn drain(&mut self) -> Vec<UndoRecord> {
        std::mem::take(&mut self.records)
    }

    /// Apply all undo records in reverse order.
    ///
    /// The `apply_fn` callback receives each undo record and should
    /// write `old_data` back to the appropriate table/row/column.
    /// Records are applied in **reverse** order (LIFO) so that the
    /// last write is undone first — preserving intermediate states.
    pub fn rollback<F>(&mut self, mut apply_fn: F) -> Result<(), StorageError>
    where
        F: FnMut(&UndoRecord) -> Result<(), StorageError>,
    {
        for record in self.records.iter().rev() {
            apply_fn(record)?;
        }
        self.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_buffer() {
        let buf = UndoBuffer::new();
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn test_record_and_drain() {
        let mut buf = UndoBuffer::new();
        buf.record(1, 100, 0, vec![1, 2, 3]);
        buf.record(1, 200, 1, vec![4, 5, 6]);
        assert_eq!(buf.len(), 2);

        let drained = buf.drain();
        assert_eq!(drained.len(), 2);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_rollback_applies_reverse_order() {
        let mut buf = UndoBuffer::new();
        buf.record(1, 100, 0, vec![1]); // first write
        buf.record(1, 100, 0, vec![2]); // second write (overwrites first)

        let mut applied = Vec::new();
        buf.rollback(|rec| {
            applied.push(rec.old_data[0]);
            Ok(())
        })
        .unwrap();

        // Rollback applies in reverse: last write undone first
        assert_eq!(applied, vec![2, 1]);
    }

    #[test]
    fn test_rollback_clears_buffer() {
        let mut buf = UndoBuffer::new();
        buf.record(1, 100, 0, vec![1]);

        buf.rollback(|_| Ok(())).unwrap();
        assert!(buf.is_empty());
    }
}
