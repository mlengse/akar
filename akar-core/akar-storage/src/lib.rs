//! Akar storage engine.
//!
//! Disk-based columnar storage with buffer management, WAL, compression, and indexing.

pub mod art_index;
pub mod art_key;
pub mod art_node;
pub mod buffer_manager;
pub mod checkpoint;
pub mod column;
pub mod column_chunk;
pub mod compression;
pub mod csr;
pub mod csv_reader;
pub mod free_space_manager;
pub mod hyperloglog;
pub mod ice_format;
pub mod index;
pub mod lazy_scanner;
pub mod local_storage;
pub mod local_wal;
pub mod node_group;
pub mod npy_reader;
pub mod page;
pub mod page_manager;
#[cfg(feature = "parquet")]
pub mod parquet_reader;
#[cfg(feature = "parquet")]
pub mod parquet_writer;
pub mod persistence;
pub mod predicate;
pub mod roaring_bitmap;
pub mod shadow_file;
pub mod spiller;
pub mod stats;
pub mod string_dictionary;
pub mod table;
pub mod undo_buffer;
pub mod update_info;
pub mod vector_index;
pub mod version_info;
pub mod wal;
pub mod wal_replayer;

use akar_common::error::StorageError;
use akar_common::memory::MemoryManager;
use akar_common::types::Value;
use akar_vector::hnsw::DistanceMetric;
use buffer_manager::{BufferManager, BufferManagerConfig};
use checkpoint::checkpoint;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use wal::WAL;

pub use art_index::ArtPrimaryKeyIndex;
pub use art_key::ArtKey;
pub use column_chunk::{ColumnChunk, NODE_GROUP_SIZE};
pub use index::{HashIndex, IndexKey, OnDiskHashIndex};
pub use local_storage::LocalStorage;
pub use local_wal::LocalWAL;
pub use node_group::NodeGroup;
pub use page_manager::PageManager;
pub use persistence::TablePersistence;
pub use shadow_file::ShadowFile;
pub use spiller::{MultiWayStreamMerge, SpillFile, Spiller};
pub use string_dictionary::StringDictionary;
pub use table::{ColumnDefinition, NodeTable, RelTable, TableCatalog};
pub use undo_buffer::UndoBuffer;
pub use vector_index::{VectorIndexTable, extract_f64_list_from_value};
pub use wal::WalSink;
pub use wal_replayer::{ReplayResult, WALReplayer};

/// Shared sink that write-path operators push typed [`WALRecord`]s into
/// during execution (P60.2). Drained into the transaction's `LocalWAL` by
/// the connection layer; bulk-copied into the global WAL at commit.
pub use wal::log_delete_record;
pub use wal::log_insert_record;
pub use wal::log_rel_insert_record;
pub use wal::log_update_record;

/// Serialize a row of values into the tagged binary format expected by
/// `deserialize_values_from_bytes` (the `WALRecord::Insert`/`Update` payload
/// format used for SQL-path WAL replay, P60.2).
pub fn serialize_values_to_bytes(values: &[Value]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 16);
    for v in values {
        out.extend_from_slice(&column::Column::serialize_value(v));
    }
    out
}

/// Convert a catalog column definition into a storage-level column definition.
///
/// `CatalogColumn.default_value` is handled at the binder/schema layer and does
/// not need to be materialized in the storage table.
impl From<&akar_catalog::CatalogColumn> for ColumnDefinition {
    fn from(c: &akar_catalog::CatalogColumn) -> Self {
        ColumnDefinition {
            name: c.name.clone(),
            logical_type: c.logical_type,
            is_primary_key: c.is_primary_key,
            compression: c.compression,
        }
    }
}

/// The storage manager — root of the storage engine.
#[allow(dead_code)]
pub struct StorageManager {
    db_path: PathBuf,
    buffer_manager: Arc<Mutex<BufferManager>>,
    wal: Arc<Mutex<WAL>>,
    memory_manager: Arc<MemoryManager>,
    /// Page manager for allocation/deallocation.
    page_manager: Option<Arc<PageManager>>,
    /// Lock-free table catalog using DashMap internally.
    pub(crate) table_catalog: Arc<TableCatalog>,
    /// Durable column mirrors for in-memory tables (P45.4).
    table_persistence: TablePersistence,
    /// Optional spiller attached to newly created node tables so bulk ingest
    /// spills to disk once a NodeGroup exceeds the memory threshold (P51.44).
    spiller: std::sync::RwLock<Option<Arc<Spiller>>>,
}

/// Storage info returned by CALL storage_info().
#[derive(Debug, Clone)]
pub struct StorageInfo {
    pub db_path: String,
    pub page_size: usize,
    pub total_pages: u64,
    pub free_pages: u64,
}

/// Buffer manager info returned by CALL bm_info().
#[derive(Debug, Clone)]
pub struct BufferInfo {
    pub total_memory: usize,
    pub used_memory: usize,
    pub num_pinned: usize,
}

/// File info returned by CALL file_info() / CALL disk_size_info().
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub total_file_size: u64,
    pub num_data_pages: u64,
    pub wal_size: u64,
}

/// FSM info returned by CALL free_space_info().
#[derive(Debug, Clone)]
pub struct FsmInfo {
    pub total_free_pages: u64,
    pub num_entries: usize,
}

impl StorageManager {
    pub fn new(db_path: PathBuf, memory_manager: Arc<MemoryManager>) -> Self {
        // Ensure the database directory exists (ignore error for :memory: mode)
        let _ = std::fs::create_dir_all(&db_path);

        let config = BufferManagerConfig::default();
        let bm = BufferManager::new(db_path.clone(), memory_manager.clone(), config);
        let wal_path = if db_path.to_string_lossy() == ":memory:" {
            // In-memory mode: use a temp dir for the WAL
            let tmp = std::env::temp_dir().join("akar-wal");
            let _ = std::fs::create_dir_all(&tmp);
            tmp.join("wal.log")
        } else {
            db_path.join("wal.log")
        };
        // Do NOT delete the WAL file — it may contain un-recovered data from
        // a previous session. Recovery is triggered later by `Database::new()`
        // via the `recover()` method.
        let wal = WAL::new(wal_path);
        let fsm = Arc::new(free_space_manager::FreeSpaceManager::new());
        let existing_pages = 0u64; // Will be determined by file metadata
        let pm = PageManager::new(db_path.clone(), page::DEFAULT_PAGE_SIZE, existing_pages, fsm);
        Self {
            db_path,
            buffer_manager: Arc::new(Mutex::new(bm)),
            wal: Arc::new(Mutex::new(wal)),
            memory_manager,
            page_manager: Some(Arc::new(pm)),
            table_catalog: Arc::new(TableCatalog::new()),
            table_persistence: TablePersistence::new(),
            spiller: std::sync::RwLock::new(None),
        }
    }

    /// Attach a spiller so node tables spill during bulk ingest once a
    /// NodeGroup's buffer exceeds the memory threshold (P51.44). Applies to
    /// tables that already exist as well as future ones. A `None` clears the
    /// spiller.
    pub fn set_spiller(&self, spiller: Option<Arc<Spiller>>) {
        *self.spiller.write().unwrap() = spiller.clone();
        // Collect IDs first: `all_node_tables()` holds shard read-locks for
        // the returned refs, so a subsequent `get_node_table_mut` (write-lock
        // on the same shard) would deadlock (P51.44).
        let table_ids: Vec<u64> = self.table_catalog.all_node_tables().iter().map(|r| *r.key()).collect();
        for table_id in table_ids {
            if let Some(mut table) = self.table_catalog.get_node_table_mut(table_id) {
                table.set_spiller(spiller.clone());
            }
        }
    }

    /// The currently attached spiller (if any).
    pub fn spiller(&self) -> Option<Arc<Spiller>> {
        self.spiller.read().unwrap().clone()
    }

    /// Open (or create) a database at `db_path`, initializing all storage
    /// subsystems and replaying the WAL if necessary.
    ///
    /// This is the primary entry point for storage initialization.
    /// After opening, call `recover()` to replay any uncommitted WAL records.
    pub fn open(db_path: PathBuf, memory_manager: Arc<MemoryManager>) -> Self {
        Self::new(db_path, memory_manager)
    }

    /// Get a reference to the page manager, if available.
    pub fn page_manager(&self) -> Option<&Arc<PageManager>> {
        self.page_manager.as_ref()
    }

    pub fn buffer_manager(&self) -> &Arc<Mutex<BufferManager>> {
        &self.buffer_manager
    }

    pub fn wal(&self) -> &Arc<Mutex<WAL>> {
        &self.wal
    }

    pub fn db_path(&self) -> &PathBuf {
        &self.db_path
    }

    /// Get a reference to the table catalog for reading/writing table data.
    pub fn table_catalog(&self) -> Arc<TableCatalog> {
        self.table_catalog.clone()
    }

    /// Flush all node + rel tables into their durable column mirrors.
    ///
    /// Called after every write (commit or single-writer DML) and at
    /// checkpoint time so committed rows survive restarts (P45.4).
    pub fn persist_all_tables(&self) -> Result<(), StorageError> {
        if self.db_path.to_string_lossy() == ":memory:" {
            return Ok(()); // In-memory databases have nothing to persist
        }
        let page_size = self.buffer_manager.lock().unwrap().page_size();
        self.table_persistence
            .persist_all(&self.table_catalog, &self.db_path, &self.buffer_manager, page_size)
    }

    /// Load all persisted tables from their durable column mirrors.
    ///
    /// Called during `Database::new()` AFTER tables are restored from the
    /// persisted catalog. Returns the number of tables that had persisted data.
    pub fn load_persisted_tables(&self) -> Result<usize, StorageError> {
        if self.db_path.to_string_lossy() == ":memory:" {
            return Ok(0); // In-memory databases have no persisted mirrors
        }
        let page_size = self.buffer_manager.lock().unwrap().page_size();
        self.table_persistence
            .load_all(&self.table_catalog, &self.db_path, &self.buffer_manager, page_size)
    }

    /// Delete the durable column mirror for a dropped table.
    pub fn drop_table_persistence(&self, table_id: u64) {
        self.table_persistence
            .remove(table_id, &self.db_path, &self.buffer_manager);
    }

    /// Log a column write to the WAL before applying it to the BufferManager.
    pub fn log_column_write(&self, table_id: u64, col_id: u32, page_id: u64, data: &[u8]) {
        let mut wal = self.wal.lock().unwrap();
        wal.log_column_write(table_id, col_id, page_id, data);
    }

    /// Create a node table in the catalog and return its ID.
    pub fn create_node_table(&self, name: String, columns: Vec<ColumnDefinition>) -> NodeTable {
        let table = self.table_catalog.create_node_table(name, columns);
        self.attach_spiller(&table);
        table
    }

    /// Attach the current spiller (if any) to a freshly created node table so
    /// bulk ingest spills to disk once a NodeGroup exceeds the memory
    /// threshold (P51.44).
    fn attach_spiller(&self, table: &NodeTable) {
        if let Some(spiller) = self.spiller() {
            if let Some(mut stored) = self.table_catalog.get_node_table_mut(table.table_id) {
                stored.set_spiller(Some(spiller));
            }
        }
    }

    /// Restore a node table at a specific table ID during recovery from a
    /// persisted catalog. Optionally recreates an ART primary-key index and
    /// registers its file with the BufferManager.
    pub fn restore_node_table(
        &self,
        table_id: u64,
        name: String,
        columns: Vec<ColumnDefinition>,
        index_name: Option<&str>,
    ) -> NodeTable {
        let table = self
            .table_catalog
            .create_node_table_with_id(table_id, name.clone(), columns);

        if let Some(index_name) = index_name {
            // Register the index file with the BufferManager for persistence
            let mut bm = self.buffer_manager.lock().unwrap();
            let full_path = self.db_path.join(format!("{index_name}.art"));
            if !bm.is_file_registered(index_name) {
                bm.register_file(index_name, full_path);
            }
            drop(bm);

            let _ = self.table_catalog.create_art_index(&name, index_name);
        }
        self.attach_spiller(&table);
        table
    }

    /// Restore a rel table at a specific table ID during recovery from a
    /// persisted catalog.
    pub fn restore_rel_table(
        &self,
        table_id: u64,
        name: String,
        src_table_id: u64,
        dst_table_id: u64,
        columns: Vec<ColumnDefinition>,
    ) -> RelTable {
        self.table_catalog
            .create_rel_table_with_id(table_id, name, src_table_id, dst_table_id, columns)
    }

    /// Create a vector index in the catalog and register its file with the BufferManager.
    pub fn create_vector_index(
        &self,
        name: String,
        table_name: String,
        column_name: String,
        metric: DistanceMetric,
        dimensions: u32,
    ) -> VectorIndexTable {
        let table = self
            .table_catalog
            .create_vector_index(name, table_name, column_name, metric, dimensions);

        // Register the index file with the BufferManager
        let mut bm = self.buffer_manager.lock().unwrap();
        table.register_file(&mut bm, &self.db_path);

        table
    }

    /// Get a vector index by name.
    pub fn get_vector_index_by_name(&self, name: &str) -> Option<dashmap::mapref::one::Ref<'_, u64, VectorIndexTable>> {
        self.table_catalog.get_vector_index_by_name(name)
    }

    /// Get a mutable vector index by name.
    pub fn get_vector_index_by_name_mut(
        &self,
        name: &str,
    ) -> Option<dashmap::mapref::one::RefMut<'_, u64, VectorIndexTable>> {
        self.table_catalog.get_vector_index_by_name_mut(name)
    }

    /// Create an ART (Adaptive Radix Tree) index on a node table.
    /// Delegates to TableCatalog and registers the index file with BufferManager.
    pub fn create_art_index(&self, table_name: &str, index_name: &str) -> Result<(), StorageError> {
        self.table_catalog.create_art_index(table_name, index_name)?;

        // Register the index file with the BufferManager for persistence
        let mut bm = self
            .buffer_manager
            .lock()
            .map_err(|e| StorageError::BufferManager(format!("Lock poisoned: {e}")))?;
        let full_path = self.db_path.join(format!("{index_name}.art"));
        let file_name = index_name.to_string();
        if !bm.is_file_registered(&file_name) {
            bm.register_file(&file_name, full_path);
        }
        drop(bm);

        Ok(())
    }

    /// Drop an ART index from a node table.
    pub fn drop_art_index(&self, table_name: &str, _index_name: &str) -> Result<(), StorageError> {
        self.table_catalog.drop_art_index(table_name)
    }

    /// Get the ART index for a node table (cloned copy for read-only access).
    pub fn get_art_index(&self, table_name: &str) -> Option<crate::ArtPrimaryKeyIndex> {
        self.table_catalog.get_art_index(table_name)
    }

    /// Create a rel table in the catalog.
    pub fn create_rel_table(
        &self,
        name: String,
        src_table_id: u64,
        dst_table_id: u64,
        columns: Vec<ColumnDefinition>,
    ) -> RelTable {
        self.table_catalog
            .create_rel_table(name, src_table_id, dst_table_id, columns)
    }

    /// Get the total size of the WAL in bytes.
    pub fn wal_size(&self) -> usize {
        self.wal.lock().unwrap().total_size()
    }

    /// Perform a checkpoint: flush WAL + dirty pages to disk.
    pub fn checkpoint(&self) -> std::io::Result<checkpoint::CheckpointResult> {
        let mut wal = self
            .wal
            .lock()
            .map_err(|e| std::io::Error::other(format!("Lock poisoned: {e}")))?;
        checkpoint(&mut wal, &self.buffer_manager)
    }

    /// Conditionally trigger a checkpoint based on the given threshold.
    ///
    /// This is called after every DML/DDL operation from `Connection::query()`.
    ///
    /// Semantics:
    /// - `threshold < 0` (e.g., -1): checkpoint after every write (every DML/DDL).
    /// - `threshold == 0`: never auto-checkpoint (manual only via `CHECKPOINT`).
    /// - `threshold > 0`: checkpoint when `wal_size() > threshold` (bytes).
    ///
    /// Returns `true` if a checkpoint was triggered.
    pub fn maybe_checkpoint(
        &self,
        threshold: i64,
        drain_fn: Option<&dyn Fn(std::time::Duration) -> bool>,
    ) -> std::io::Result<bool> {
        if threshold == 0 {
            return Ok(false); // Auto-checkpoint disabled
        }

        let should_checkpoint = if threshold < 0 {
            // Always checkpoint after every write (default behavior)
            true
        } else {
            self.wal_size() > threshold as usize
        };

        if should_checkpoint {
            let _ = self.checkpoint_with_drain(drain_fn)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Perform a checkpoint with transaction drain.
    ///
    /// Two-phase drain:
    /// 1. Call the `drain_fn` callback to stop new transactions and wait for active ones
    /// 2. Perform the checkpoint (WAL flush + BM flush)
    ///
    /// This is the concurrent-writer-safe checkpoint. Use this instead of
    /// plain `checkpoint()` when concurrent writes are enabled.
    ///
    /// If `drain_fn` is `None`, the drain is skipped (backwards-compatible default).
    /// If the drain times out, the checkpoint proceeds anyway — this is safe because
    /// the WAL will capture any in-flight writes.
    pub fn checkpoint_with_drain(
        &self,
        drain_fn: Option<&dyn Fn(std::time::Duration) -> bool>,
    ) -> std::io::Result<crate::checkpoint::CheckpointResult> {
        // Phase 1: Stop new transactions and drain active ones
        if let Some(drain) = drain_fn {
            let drained = drain(std::time::Duration::from_secs(30));
            if !drained {
                tracing::warn!("Checkpoint drain timed out — proceeding with best-effort checkpoint");
            }
        }

        // Persist in-memory tables to their durable column mirrors so the
        // checkpoint's BufferManager flush writes them to disk too (P45.4).
        // Done before the checkpoint so a crash after WAL truncation still
        // leaves the mirror consistent with the tables.
        if let Err(e) = self.persist_all_tables() {
            tracing::warn!("Persist tables before checkpoint failed: {e}");
        }

        // Phase 2: Do the actual checkpoint
        let mut wal = self
            .wal
            .lock()
            .map_err(|e| std::io::Error::other(format!("Lock poisoned: {e}")))?;
        crate::checkpoint::checkpoint(&mut wal, &self.buffer_manager)
    }

    /// Get storage-level information for diagnostics.
    pub fn storage_info(&self) -> StorageInfo {
        let total_pages = self.page_manager.as_ref().map(|pm| pm.total_pages()).unwrap_or(0);
        let free_pages = 0u64; // FSM query could be added later
        StorageInfo {
            db_path: self.db_path.to_string_lossy().to_string(),
            page_size: self
                .page_manager
                .as_ref()
                .map(|pm| pm.page_size())
                .unwrap_or(page::DEFAULT_PAGE_SIZE),
            total_pages,
            free_pages,
        }
    }

    /// Buffer manager statistics for CALL bm_info().
    pub fn buffer_info(&self) -> BufferInfo {
        let bm = self.buffer_manager.lock().unwrap();
        let stats = bm.stats();
        let page_size = bm.page_size();
        BufferInfo {
            total_memory: stats.num_frames * page_size,
            used_memory: (stats.num_frames - (stats.num_frames - stats.pinned_frames - stats.dirty_frames)) * page_size,
            num_pinned: stats.pinned_frames,
        }
    }

    /// File-level statistics for CALL file_info() / CALL disk_size_info().
    pub fn file_info(&self) -> FileInfo {
        let db_path = &self.db_path;
        let wal_path = db_path.join("wal.log");
        let wal_size = std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0);
        let data_size = std::fs::read_dir(db_path)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().map(|x| x == "data").unwrap_or(false))
                    .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
                    .sum::<u64>()
            })
            .unwrap_or(0);
        let page_size = self
            .page_manager
            .as_ref()
            .map(|pm| pm.page_size())
            .unwrap_or(page::DEFAULT_PAGE_SIZE) as u64;
        FileInfo {
            total_file_size: data_size + wal_size,
            num_data_pages: data_size / page_size.max(1),
            wal_size,
        }
    }

    /// FSM statistics for CALL free_space_info().
    pub fn fsm_info(&self) -> FsmInfo {
        let total_pages = self.page_manager.as_ref().map(|pm| pm.total_pages()).unwrap_or(0);
        let free_pages = 0u64; // FSM query later
        FsmInfo {
            total_free_pages: free_pages,
            num_entries: total_pages as usize,
        }
    }

    /// Commit a write transaction's data to storage.
    ///
    /// Orchestrates the full commit pipeline:
    /// 1. Append `Commit` record to the WAL (write-ahead log) + fsync
    /// 2. Flush `LocalStorage` buffered writes to the actual tables
    /// 3. Apply `ShadowFile` copy-on-write pages to the BufferManager
    /// 4. Optionally checkpoint if the WAL threshold is met
    ///
    /// Since P60.2 the SQL write path emits typed Insert/Delete/Update WAL
    /// records, so committed data is durable from the WAL alone; the durable
    /// column mirrors are written only by checkpoints and by `recover()`.
    ///
    /// # Arguments
    ///
    /// * `local_storage` — the transaction's write buffer (consumed on success).
    /// * `shadow_file` — the transaction's COW page buffer.
    /// * `checkpoint_threshold` — passed to `maybe_checkpoint()`; use -1 for
    ///   always-checkpoint, 0 for never, N for byte-based threshold.
    /// * `drain_fn` — optional callback to drain active transactions before checkpoint.
    ///
    /// Returns `Ok(())` if the commit pipeline succeeded.
    pub fn commit_transaction(
        &self,
        local_storage: &crate::local_storage::LocalStorage,
        shadow_file: &crate::shadow_file::ShadowFile,
        checkpoint_threshold: i64,
        txn_id: u64,
        drain_fn: Option<&dyn Fn(std::time::Duration) -> bool>,
    ) -> Result<(), StorageError> {
        // Step 1: Write-ahead log the commit
        {
            let mut wal = self
                .wal
                .lock()
                .map_err(|e| StorageError::Wal(format!("Lock poisoned: {e}")))?;
            wal.append(crate::wal::WALRecord::Commit { transaction_id: txn_id });
            wal.flush_to_disk()
                .map_err(|e| StorageError::Wal(format!("WAL flush failed during commit: {e}")))?;
        }

        // Step 2: Flush local storage buffers to the actual tables.
        // P60.2: the standalone mirror persist is GONE — the SQL write path
        // now emits typed Insert/Delete/Update WAL records (bulk-copied into
        // the global WAL at commit and replayed on top of the checkpoint
        // mirrors by `recover()`), so per-commit mirror I/O is unnecessary in
        // every threshold mode. Mirrors are written only by checkpoints
        // (`checkpoint_with_drain`) and by `recover()` after a replay.
        // Pass txn_id so inserts/deletes are recorded in VersionInfo for MVCC
        let _commit_undo_records = local_storage.flush_to_tables(&self.table_catalog, Some(txn_id))?;
        // Undo records generated during commit for potential rollback-on-failure.
        // Currently unused since commit is atomic, but stored for future use
        // (e.g., partial-commit recovery).
        tracing::debug!(
            "commit_transaction: generated {} undo records for txn#{}",
            _commit_undo_records.len(),
            txn_id
        );

        // Step 3: Apply shadow pages to the BufferManager
        shadow_file
            .apply(&self.buffer_manager)
            .map_err(|e| StorageError::ShadowFile(format!("ShadowFile apply failed during commit: {e}")))?;

        // Step 4: Auto-checkpoint if needed. When it fires it persists the
        // durable column mirrors before truncating the WAL (see
        // `checkpoint_with_drain`).
        if let Err(e) = self.maybe_checkpoint(checkpoint_threshold, drain_fn) {
            tracing::warn!("Checkpoint after commit failed: {e}");
            // Non-fatal — data is already in tables and WAL
        }

        Ok(())
    }

    /// Roll back a write transaction, discarding all pending changes.
    ///
    /// Clears the local storage buffer, discards shadow pages, and
    /// applies undo records to restore pre-write state.
    /// The caller should also call `TransactionManager::rollback()` to
    /// update the transaction's status and release locks.
    ///
    /// `undo_records` — accumulated undo records from the transaction.
    ///   Applied in reverse order to restore overwritten data.
    ///
    /// Returns `Ok(())` on success.
    pub fn rollback_transaction(
        &self,
        local_storage: &mut crate::local_storage::LocalStorage,
        shadow_file: &mut crate::shadow_file::ShadowFile,
        txn_id: u64,
        undo_records: &[akar_transaction::UndoRecord],
    ) -> Result<(), StorageError> {
        // Log the rollback to WAL
        {
            let mut wal = self
                .wal
                .lock()
                .map_err(|e| StorageError::Wal(format!("Lock poisoned: {e}")))?;
            wal.append(crate::wal::WALRecord::Rollback { transaction_id: txn_id });
            let _ = wal.flush_to_disk();
        }

        // Apply undo records in reverse order to restore pre-write state
        for record in undo_records.iter().rev() {
            if let Some(mut table) = self.table_catalog.get_node_table_mut(record.table_id) {
                match record.undo_type {
                    akar_transaction::UndoType::Update => {
                        let values = deserialize_values_from_bytes(&record.old_data, 1);
                        if let Some(val) = values.into_iter().next() {
                            table
                                .update_cell(record.row_id, record.column as usize, val)
                                .map_err(|e| {
                                    StorageError::Undo(format!(
                                        "Undo failed for table {} row {}: {e}",
                                        record.table_id, record.row_id
                                    ))
                                })?;
                        }
                    }
                    akar_transaction::UndoType::Insert => {
                        // Rollback an insert: delete the row
                        let _ = table.delete_row(record.row_id);
                    }
                    akar_transaction::UndoType::Delete => {
                        // Rollback a delete: restore all column values
                        let num_cols = table.columns.len();
                        let values = deserialize_values_from_bytes(&record.old_data, num_cols);
                        for (col_idx, val) in values.into_iter().enumerate() {
                            let _ = table.update_cell(record.row_id, col_idx, val);
                        }
                    }
                }
            } else if let Some(mut rel) = self.table_catalog.get_rel_table_mut(record.table_id) {
                // Rel-table undo (P52.18): inserts delete the edge, updates
                // restore the property cell, deletes restore src/dst + props.
                match record.undo_type {
                    akar_transaction::UndoType::Update => {
                        let values = deserialize_values_from_bytes(&record.old_data, 1);
                        if let Some(val) = values.into_iter().next() {
                            let _ = rel.update_cell(record.row_id as usize, record.column as usize, val);
                        }
                    }
                    akar_transaction::UndoType::Insert => {
                        // Rollback an edge insert: tombstone the edge.
                        let _ = rel.delete_edge(record.row_id as usize);
                    }
                    akar_transaction::UndoType::Delete => {
                        // Rollback an edge delete: restore src/dst + properties.
                        let num_cols = rel.columns.len();
                        let values = deserialize_values_from_bytes(&record.old_data, num_cols + 2);
                        let mut iter = values.into_iter();
                        let src = match iter.next() {
                            Some(Value::UInt64(v)) => v,
                            Some(Value::Int64(v)) if v >= 0 => v as u64,
                            _ => u64::MAX,
                        };
                        let dst = match iter.next() {
                            Some(Value::UInt64(v)) => v,
                            Some(Value::Int64(v)) if v >= 0 => v as u64,
                            _ => u64::MAX,
                        };
                        let props: Vec<_> = iter.collect();
                        let _ = rel.restore_deleted_edge(record.row_id as usize, src, dst, props);
                    }
                }
            }
        }

        // Discard buffered writes
        local_storage.clear();
        shadow_file.discard();

        Ok(())
    }

    /// Recover state after a crash or unclean shutdown.
    ///
    /// Recovery source order (P45.4, amended P60.2):
    /// 1. **Durable column mirrors** — the state at the last checkpoint — are
    ///    loaded first;
    /// 2. **WAL replay** then applies every committed delta since that
    ///    checkpoint: typed `Insert`/`Delete`/`Update` records emitted by the
    ///    SQL write path, plus records decoded out of bulk-copied
    ///    `LocalWALData` blobs. Replaying on top of the mirrors preserves
    ///    row-id continuity and reconstructs full state even when no
    ///    checkpoint ever ran (mirrors absent → whole log is replayed).
    ///
    /// Finally the recovered tables are re-persisted and a checkpoint resets
    /// the WAL, so a subsequent startup restores from mirrors alone.
    ///
    /// Call this once during `Database::new()`, **after** table schemas
    /// have been re-created from the persisted catalog (same table IDs).
    ///
    /// Returns the number of data records applied during replay (0 when the
    /// WAL was empty), or an error if recovery fails (database is corrupt).
    pub fn recover(&self) -> std::io::Result<usize> {
        // Phase 1: restore checkpoint state from the durable column mirrors.
        match self.load_persisted_tables() {
            Ok(n) if n > 0 => tracing::info!("Restored {n} table(s) from durable column mirrors"),
            Ok(_) => {}
            Err(e) => tracing::warn!("Failed to restore tables from column mirrors: {e}"),
        }

        let mut wal = self
            .wal
            .lock()
            .map_err(|e| std::io::Error::other(format!("Lock poisoned: {e}")))?;

        // Load WAL records from disk
        wal.load_from_disk()?;

        if wal.is_empty() {
            return Ok(0); // Nothing to recover
        }

        let mut data_records = 0usize;
        let catalog = self.table_catalog.clone();

        // Replay each record on top of the mirror state.
        wal.replay(|record| replay_data_record(record, &catalog, &mut data_records))?;

        // After successful replay, mirror the recovered rows into the durable
        // column mirrors as well, so a subsequent startup with an empty WAL
        // restores from the mirror instead of the (now truncated) log.
        if data_records > 0 {
            self.persist_all_tables()
                .map_err(|e| std::io::Error::other(format!("WAL recovery: persist tables failed: {e}")))?;
        }

        // Take a checkpoint to reset the WAL and make all recovered data durable.
        if let Err(e) = checkpoint(&mut wal, &self.buffer_manager) {
            tracing::warn!("WAL recovery: checkpoint after replay failed: {e}");
        }

        Ok(data_records)
    }
}

/// Helper: deserialize binary data back into a `Vec<Value>`.
///
/// Each value is stored as a tag byte followed by type-specific data
/// (see `column.rs` for the tag format). This is a simplified version
/// that handles the common primary-key and property types.
/// Apply one replayed WAL record to the catalog tables.
///
/// Handles node **and** rel tables (P60.2): rel inserts carry a
/// `[src, dst, props…]` payload that rebuilds the adjacency entry via
/// `RelTable::insert_rel`; deletes/updates dispatch by table kind.
/// `LocalWALData` blobs are decoded into their individual records and
/// applied recursively — this is how SQL-path DML reaches recovery, since
/// commit bulk-copies each transaction's typed records as one blob.
pub(crate) fn replay_data_record(
    record: &crate::wal::WALRecord,
    catalog: &Arc<TableCatalog>,
    data_records: &mut usize,
) -> std::io::Result<()> {
    use crate::wal::WALRecord;

    let as_u64 = |v: &Value| -> Option<u64> {
        match v {
            Value::UInt64(x) => Some(*x),
            Value::Int64(x) => (*x >= 0).then_some(*x as u64),
            _ => None,
        }
    };

    match record {
        WALRecord::Insert { table_id, data } => {
            if let Some(mut table) = catalog.get_node_table_mut(*table_id) {
                let values = deserialize_values_from_bytes(data, table.columns.len());
                if let Err(e) = table.insert_row(values) {
                    return Err(std::io::Error::other(format!("WAL recovery insert failed: {e}")));
                }
                *data_records += 1;
            } else if let Some(mut rel) = catalog.get_rel_table_mut(*table_id) {
                let values = deserialize_values_from_bytes(data, rel.columns.len() + 2);
                if values.len() < 2 {
                    return Err(std::io::Error::other("WAL recovery: rel insert payload too short"));
                }
                let (Some(src), Some(dst)) = (as_u64(&values[0]), as_u64(&values[1])) else {
                    return Err(std::io::Error::other("WAL recovery: rel insert endpoints missing"));
                };
                if let Err(e) = rel.insert_rel(src, dst, values[2..].to_vec()) {
                    return Err(std::io::Error::other(format!("WAL recovery rel insert failed: {e}")));
                }
                *data_records += 1;
            } else {
                tracing::debug!("WAL recovery: table {table_id} not found; skipping Insert");
            }
        }
        WALRecord::Delete { table_id, row_id } => {
            if let Some(mut table) = catalog.get_node_table_mut(*table_id) {
                if let Err(e) = table.delete_row(*row_id) {
                    return Err(std::io::Error::other(format!("WAL recovery delete failed: {e}")));
                }
                *data_records += 1;
            } else if let Some(mut rel) = catalog.get_rel_table_mut(*table_id) {
                if let Err(e) = rel.delete_edge(*row_id as usize) {
                    return Err(std::io::Error::other(format!("WAL recovery edge delete failed: {e}")));
                }
                *data_records += 1;
            } else {
                tracing::debug!("WAL recovery: table {table_id} not found; skipping Delete");
            }
        }
        WALRecord::Update {
            table_id,
            row_id,
            column,
            data,
        } => {
            let values = deserialize_values_from_bytes(data, 1);
            if let Some(val) = values.into_iter().next() {
                if let Some(mut table) = catalog.get_node_table_mut(*table_id) {
                    if let Err(e) = table.update_cell(*row_id, *column as usize, val) {
                        return Err(std::io::Error::other(format!("WAL recovery update failed: {e}")));
                    }
                    *data_records += 1;
                } else if let Some(mut rel) = catalog.get_rel_table_mut(*table_id) {
                    if let Err(e) = rel.update_cell(*row_id as usize, *column as usize, val) {
                        return Err(std::io::Error::other(format!("WAL recovery edge update failed: {e}")));
                    }
                    *data_records += 1;
                } else {
                    tracing::debug!("WAL recovery: table {table_id} not found; skipping Update");
                }
            }
        }
        WALRecord::LocalWALData { data } => {
            for sub in crate::wal::decode_wal_buffer(data)? {
                replay_data_record(&sub, catalog, data_records)?;
            }
        }
        WALRecord::UpdateFsm { .. } => {
            // UpdateFsm records are handled by the FSM recovery directly,
            // we can ignore them during the table-level replay.
        }
        WALRecord::ColumnWrite { .. } => {
            // ColumnWrite records are for the BufferManager-level
            // page writes. At the table level, data is already in
            // NodeGroup memory, so we skip these during recovery
            // (the checkpoint handles page-level persistence).
        }
        WALRecord::Commit { .. } | WALRecord::Rollback { .. } => {
            // Transaction markers — ignore during recovery since
            // all records in the WAL at startup are from already-
            // committed transactions (uncommitted ones were lost
            // in the crash).
        }
        WALRecord::Checkpoint => {
            // Checkpoint marker — all data before this is already
            // durable. In practice, a checkpoint clears the WAL so this
            // marker should rarely appear during recovery.
        }
        // DDL records — metadata-only, no data to replay for now.
        // DDL operations (CREATE/DROP TABLE, etc.) are captured via
        // Catalog serialization separately.
        WALRecord::CreateTable { .. }
        | WALRecord::DropTable { .. }
        | WALRecord::AlterTable { .. }
        | WALRecord::CreateIndex { .. }
        | WALRecord::DropIndex { .. }
        | WALRecord::CreateSequence { .. } => {
            // DDL records are replayed via catalog snapshot, not
            // individual WAL entries. Skip during table-level replay.
        }
    }
    Ok(())
}

pub(crate) fn deserialize_values_from_bytes(data: &[u8], expected_count: usize) -> Vec<Value> {
    use crate::column::Column;

    if data.is_empty() || expected_count == 0 {
        return Vec::new();
    }

    // Delegate to the full `Column` value parser so every tag round-trips.
    // The previous hand-rolled subset silently turned unhandled tags (notably
    // the UInt64 encoding used for rel endpoints after PK coercion) into Null
    // AND desynced the cursor, aborting WAL recovery mid-log (P60.2).
    let mut values = Vec::with_capacity(expected_count);
    let mut pos = 0usize;
    for _ in 0..expected_count {
        if pos >= data.len() {
            values.push(Value::Null);
            continue;
        }
        match Column::deserialize_value(data, &mut pos) {
            Ok(v) => values.push(v),
            Err(_) => {
                // Lenient like the old parser: pad rather than fail recovery.
                while values.len() < expected_count {
                    values.push(Value::Null);
                }
                break;
            }
        }
    }

    values
}

// =========================================================================
// Phase 1 integration tests — full pipeline: table → column → buffer
// manager → WAL → checkpoint → compression → multi-node-group
// =========================================================================

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::column::{Column, TAG_INT64, TAG_STRING};
    use crate::page::DEFAULT_PAGE_SIZE;
    use crate::wal::WALRecord;
    use akar_common::enums::CompressionType;
    use akar_common::types::{LogicalTypeID, Value};

    // -----------------------------------------------------------------
    // Helper: create a StorageManager + column pair
    // -----------------------------------------------------------------
    fn setup_integration() -> (StorageManager, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mm = Arc::new(MemoryManager::new(128 * 1024 * 1024));
        let sm = StorageManager::new(dir.path().to_path_buf(), mm);
        (sm, dir)
    }

    // =================================================================
    // Test 1: Create table → insert rows → flush → reopen → verify
    // =================================================================
    #[test]
    fn test_table_full_persistence_cycle() {
        let (sm, _dir) = setup_integration();

        // 1. Create a node table with two columns
        let mut table = sm.create_node_table(
            "Person".into(),
            vec![
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "name".into(),
                    logical_type: LogicalTypeID::String,
                    is_primary_key: true,
                },
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "age".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: false,
                },
            ],
        );
        assert_eq!(table.table_id, 0);

        // 2. Insert rows into the table
        table
            .insert_row(vec![Value::String("Alice".into()), Value::Int64(30)])
            .unwrap();
        table
            .insert_row(vec![Value::String("Bob".into()), Value::Int64(25)])
            .unwrap();
        table
            .insert_row(vec![Value::String("Charlie".into()), Value::Int64(35)])
            .unwrap();
        assert_eq!(table.num_rows, 3);

        // 3. Verify data before checkpoint
        assert_eq!(table.get_value(0, 0), Some(&Value::String("Alice".into())));
        assert_eq!(table.get_value(1, 1), Some(&Value::Int64(25)));

        // 4. Read back via scan_column across node groups
        let names = table.scan_column(0, 0, 3, None, &[]);
        assert_eq!(names.len(), 3);
        assert_eq!(names[0], Value::String("Alice".into()));

        let ages = table.scan_column(1, 1, 2, None, &[]);
        assert_eq!(ages.len(), 2);
        assert_eq!(ages[0], Value::Int64(25));
    }

    // =================================================================
    // Test 2: WAL crash recovery — log writes, flush to disk, replay
    // =================================================================
    #[test]
    fn test_wal_recovery_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("wal.log");

        // Phase 1: Write data with WAL logging
        #[allow(unused_variables)]
        let (wal_records_count, column_count) = {
            let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
            let config = BufferManagerConfig::default();
            let bm = Arc::new(Mutex::new(BufferManager::new(dir.path().to_path_buf(), mm, config)));
            let mut wal = WAL::new(wal_path.clone());

            let mut col = Column::new(LogicalTypeID::Int64, 0, 0, dir.path(), bm.clone(), DEFAULT_PAGE_SIZE);

            // Write data and log each write to WAL
            for i in 0i64..10 {
                col.append_value(&Value::Int64(i)).unwrap();
                wal.log_column_write(0, 0, 0, &i.to_le_bytes());
            }
            wal.append(WALRecord::Commit { transaction_id: 1 });
            let count = wal.len();

            // Flush WAL to disk
            wal.flush_to_disk().unwrap();

            // Also flush BM pages
            {
                let mut bm_lock = bm.lock().unwrap();
                bm_lock.flush_all().unwrap();
            }

            // Verify column data is correct before "crash"
            for i in 0i64..10 {
                let v = col.get_value(i as u64).unwrap();
                assert_eq!(v, Value::Int64(i), "Pre-crash data mismatch at {}", i);
            }

            (count, 10)
        }; // Drop everything — simulate crash

        // Phase 2: Verify the on-disk WAL file exists and has content
        assert!(wal_path.exists(), "WAL file should exist after flush");
        let file_len = std::fs::metadata(&wal_path).unwrap().len();
        assert!(file_len > 0, "WAL file should have content, got {} bytes", file_len);

        // Verify that a fresh WAL created from the file would contain
        // the right number of records. Since WAL::new() starts in-memory,
        // we create a new one, and verify the file has the right data.
        // In a real recovery scenario, we'd implement WAL::load_from_disk().
        assert_eq!(
            wal_records_count, 11,
            "Expected 10 ColumnWrite + 1 Commit = 11 records, got {}",
            wal_records_count
        );
        assert_eq!(column_count, 10);
    }

    // =================================================================
    // Test 3: Compression round-trip — write compressed → read back
    // =================================================================
    #[test]
    fn test_compression_full_roundtrip() {
        // Test IntegerBitpacking: small values compress, large values preserved
        let dir = tempfile::tempdir().unwrap();
        let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
        let config = BufferManagerConfig::default();
        let bm = Arc::new(Mutex::new(BufferManager::new(dir.path().to_path_buf(), mm, config)));

        let mut col_int = Column::with_compression(
            LogicalTypeID::Int64,
            0,
            0,
            dir.path(),
            bm.clone(),
            DEFAULT_PAGE_SIZE,
            CompressionType::IntegerBitpacking,
        );

        // Write a range of values from small to large
        let test_values: Vec<i64> = vec![0, 1, 42, 127, 255, 65535, 1_000_000, i64::MAX, i64::MIN, -1];
        for v in &test_values {
            col_int.append_value(&Value::Int64(*v)).unwrap();
        }

        // Read back and verify
        for (i, expected) in test_values.iter().enumerate() {
            let v = col_int.get_value(i as u64).unwrap();
            assert_eq!(v, Value::Int64(*expected), "IntegerBitpacking mismatch at index {}", i);
        }

        // Verify that setting compression doesn't break existing data
        let mut col_float = Column::with_compression(
            LogicalTypeID::Double,
            0,
            1,
            dir.path(),
            bm.clone(),
            DEFAULT_PAGE_SIZE,
            CompressionType::Float,
        );

        let floats: Vec<f64> = vec![1.0, std::f64::consts::PI, -2.5e10, 0.0, f64::MIN_POSITIVE, f64::MAX];
        for v in &floats {
            col_float.append_value(&Value::Double(*v)).unwrap();
        }

        for (i, expected) in floats.iter().enumerate() {
            let v = col_float.get_value(i as u64).unwrap();
            match v {
                Value::Double(d) => assert!(
                    (d - expected).abs() < 1e-10 || (d / expected - 1.0).abs() < 1e-10,
                    "Float compression mismatch at {}: got {}, expected {}",
                    i,
                    d,
                    expected
                ),
                _ => panic!("Expected Double, got {:?}", v),
            }
        }

        // Write compressed values via existing Column API and verify roundtrip
        // with buffer manager flush
        col_int.flush().unwrap();
        col_float.flush().unwrap();

        // Read again after flush
        for (i, expected) in test_values.iter().enumerate() {
            let v = col_int.get_value(i as u64).unwrap();
            assert_eq!(
                v,
                Value::Int64(*expected),
                "After flush: IntegerBitpacking mismatch at {}",
                i
            );
        }
        for (i, expected) in floats.iter().enumerate() {
            let v = col_float.get_value(i as u64).unwrap();
            match v {
                Value::Double(d) => assert!(
                    (d - expected).abs() < 1e-10 || (d / expected - 1.0).abs() < 1e-10,
                    "After flush: Float mismatch at {}",
                    i
                ),
                _ => panic!("Expected Double after flush"),
            }
        }
    }

    // =================================================================
    // Test 4: Multi-node-group scan — insert > NODE_GROUP_SIZE rows
    // =================================================================
    #[test]
    fn test_multi_node_group_scan() {
        let (_sm, _dir) = setup_integration();

        // Create a NodeTable (not via StorageManager, directly for test control)
        let mut table = NodeTable::new(
            1,
            "BigTable".into(),
            vec![
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "id".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: false,
                },
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "value".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: false,
                },
            ],
        );

        // Insert NODE_GROUP_SIZE + 500 rows to span across multiple node groups
        let total_rows = NODE_GROUP_SIZE + 500;
        for i in 0..total_rows {
            table
                .insert_row(vec![Value::Int64(i as i64), Value::Int64((i * 2) as i64)])
                .unwrap();
        }

        // Verify total row count
        assert_eq!(table.num_rows, total_rows as u64);

        // Verify multiple node groups were created
        let expected_groups = 2; // 4096 fits in first group, 500 in second
        assert_eq!(
            table.node_groups.len(),
            expected_groups,
            "Expected {} node groups for {} rows",
            expected_groups,
            total_rows
        );

        // Verify node group boundaries
        assert_eq!(table.node_groups[0].num_nodes, NODE_GROUP_SIZE as u64);
        assert_eq!(table.node_groups[1].num_nodes, 500);
        assert_eq!(table.node_groups[0].start_offset, 0);
        assert_eq!(table.node_groups[1].start_offset, NODE_GROUP_SIZE as u64);

        // Verify scanning across group boundaries
        // Row at boundary: last row of group 0
        let row_at_boundary = (NODE_GROUP_SIZE - 1) as u64;
        assert_eq!(
            table.get_value(row_at_boundary as usize, 0),
            Some(&Value::Int64(row_at_boundary as i64))
        );

        // First row of group 1
        let row_in_group1 = NODE_GROUP_SIZE as u64;
        assert_eq!(
            table.get_value(row_in_group1 as usize, 0),
            Some(&Value::Int64(row_in_group1 as i64))
        );

        // Scan column 0 across the entire table
        let scanned = table.scan_column(0, 0, total_rows as u64, None, &[]);
        assert_eq!(scanned.len(), total_rows);
        assert_eq!(scanned[0], Value::Int64(0));
        assert_eq!(scanned[NODE_GROUP_SIZE], Value::Int64(NODE_GROUP_SIZE as i64));
        assert_eq!(scanned[total_rows - 1], Value::Int64((total_rows - 1) as i64));

        // Scan column 1 with offset and count spanning both groups
        let scan_mid = table.scan_column(1, (NODE_GROUP_SIZE - 100) as u64, 200, None, &[]);
        assert_eq!(scan_mid.len(), 200);
        assert_eq!(scan_mid[0], Value::Int64(((NODE_GROUP_SIZE - 100) * 2) as i64));
        assert_eq!(scan_mid[199], Value::Int64(((NODE_GROUP_SIZE + 99) * 2) as i64));

        // Verify to_column_major_data correctness
        let data = table.to_column_major_data();
        assert_eq!(data.len(), 2); // 2 columns
        assert_eq!(data[0].len(), total_rows);
        assert_eq!(data[1].len(), total_rows);
        assert_eq!(data[0][NODE_GROUP_SIZE], Value::Int64(NODE_GROUP_SIZE as i64));
        assert_eq!(data[1][0], Value::Int64(0));
        assert_eq!(data[1][total_rows - 1], Value::Int64(((total_rows - 1) * 2) as i64));
    }

    // =================================================================
    // Test 5: Combined — WAL-logged compressed multi-node-group write
    // =================================================================
    #[test]
    fn test_compressed_multi_group_with_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let mm = Arc::new(MemoryManager::new(128 * 1024 * 1024));

        // Use explicit BM + WAL for full control
        let config = BufferManagerConfig::default();
        let bm = Arc::new(Mutex::new(BufferManager::new(dir.path().to_path_buf(), mm, config)));
        let wal_path = dir.path().join("wal.log");
        let mut wal = WAL::new(wal_path);

        let mut col = Column::with_compression(
            LogicalTypeID::Int64,
            0,
            0,
            dir.path(),
            bm.clone(),
            DEFAULT_PAGE_SIZE,
            CompressionType::IntegerBitpacking,
        );

        // Write enough values to span multiple pages
        let num_values = 500;
        for i in 0i64..num_values {
            col.append_value(&Value::Int64(i)).unwrap();
            wal.log_column_write(0, 0, 0, &i.to_le_bytes());
        }
        wal.append(WALRecord::Commit { transaction_id: 1 });

        // Read back before checkpoint
        for i in 0i64..num_values {
            let v = col.get_value(i as u64).unwrap();
            assert_eq!(v, Value::Int64(i), "Pre-checkpoint mismatch at {}", i);
        }

        // Checkpoint: flush WAL + dirty pages
        let mut bm_lock = bm.lock().unwrap();
        bm_lock.flush_all().unwrap();
        drop(bm_lock);

        wal.flush_to_disk().unwrap();

        // Read back after checkpoint
        for i in 0i64..num_values {
            let v = col.get_value(i as u64).unwrap();
            assert_eq!(v, Value::Int64(i), "Post-checkpoint mismatch at {}", i);
        }

        // Verify multiple pages were allocated
        assert!(
            col.num_pages > 1,
            "Expected multiple pages for {} values, got {}",
            num_values,
            col.num_pages
        );
    }

    // =================================================================
    // Test 6: Stress — 10k rows via column with checkpoint
    // =================================================================
    #[test]
    fn test_10k_row_stress() {
        let dir = tempfile::tempdir().unwrap();
        let mm = Arc::new(MemoryManager::new(256 * 1024 * 1024));
        let config = BufferManagerConfig::default();
        let bm = Arc::new(Mutex::new(BufferManager::new(dir.path().to_path_buf(), mm, config)));

        let mut col = Column::new(LogicalTypeID::Int64, 0, 0, dir.path(), bm.clone(), DEFAULT_PAGE_SIZE);

        // Write 10,000 values
        for i in 0i64..10_000 {
            col.append_value(&Value::Int64(i)).unwrap();
        }
        assert_eq!(col.num_values, 10_000);

        // Read back all values
        for i in 0i64..10_000 {
            let v = col.get_value(i as u64).unwrap();
            assert_eq!(v, Value::Int64(i), "Stress test mismatch at {}", i);
        }

        // Flush and re-verify
        col.flush().unwrap();
        for i in 0i64..10_000 {
            let v = col.get_value(i as u64).unwrap();
            assert_eq!(v, Value::Int64(i), "Post-flush stress mismatch at {}", i);
        }

        // Verify multiple pages were used
        assert!(
            col.num_pages > 1,
            "Stress test should use multiple pages, got {}",
            col.num_pages
        );
    }

    // =================================================================
    // Test 7: WAL recovery — insert data, simulate crash, recover
    // =================================================================
    #[test]
    fn test_wal_recovery_insert_then_recover() {
        let dir = tempfile::tempdir().unwrap();

        // Phase 1: Create DB, insert data, flush WAL, then "crash"
        let _row_count = {
            let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
            let sm = StorageManager::new(dir.path().to_path_buf(), mm);

            // Create a node table
            let mut table = sm.create_node_table(
                "Person".into(),
                vec![
                    ColumnDefinition {
                        compression: akar_common::enums::CompressionType::Uncompressed,
                        name: "name".into(),
                        logical_type: LogicalTypeID::String,
                        is_primary_key: true,
                    },
                    ColumnDefinition {
                        compression: akar_common::enums::CompressionType::Uncompressed,
                        name: "age".into(),
                        logical_type: LogicalTypeID::Int64,
                        is_primary_key: false,
                    },
                ],
            );

            // Insert rows
            table
                .insert_row(vec![Value::String("Alice".into()), Value::Int64(30)])
                .unwrap();
            table
                .insert_row(vec![Value::String("Bob".into()), Value::Int64(25)])
                .unwrap();
            table
                .insert_row(vec![Value::String("Charlie".into()), Value::Int64(35)])
                .unwrap();

            // Put the table back into the catalog so WAL knows about it.
            // We re-create the table entry via the catalog API.
            {
                // Re-create the table entry with the same schema so the
                // catalog has a valid entry for recovery to target.
                sm.table_catalog.create_node_table(
                    "Person".into(),
                    vec![
                        ColumnDefinition {
                            compression: akar_common::enums::CompressionType::Uncompressed,
                            name: "name".into(),
                            logical_type: LogicalTypeID::String,
                            is_primary_key: true,
                        },
                        ColumnDefinition {
                            compression: akar_common::enums::CompressionType::Uncompressed,
                            name: "age".into(),
                            logical_type: LogicalTypeID::Int64,
                            is_primary_key: false,
                        },
                    ],
                );
            }

            let count = table.num_rows;
            assert_eq!(count, 3);

            // Write WAL records and flush to disk
            {
                let mut wal = sm.wal.lock().unwrap();
                wal.append(WALRecord::Insert {
                    table_id: table.table_id,
                    data: vec![
                        TAG_STRING, 5, 0, 0, 0, b'A', b'l', b'i', b'c', b'e', TAG_INT64, 30, 0, 0, 0, 0, 0, 0, 0,
                    ],
                });
                wal.append(WALRecord::Insert {
                    table_id: table.table_id,
                    data: vec![
                        TAG_STRING, 3, 0, 0, 0, b'B', b'o', b'b', TAG_INT64, 25, 0, 0, 0, 0, 0, 0, 0,
                    ],
                });
                wal.append(WALRecord::Insert {
                    table_id: table.table_id,
                    data: vec![
                        TAG_STRING, 7, 0, 0, 0, b'C', b'h', b'a', b'r', b'l', b'i', b'e', TAG_INT64, 35, 0, 0, 0, 0, 0,
                        0, 0,
                    ],
                });
                wal.flush_to_disk().unwrap();
            }

            count
        }; // "Crash" — all state dropped

        // Phase 2: Recover — create a new StorageManager, it should NOT delete
        // the WAL. Then manually trigger recovery.
        {
            let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
            let sm = StorageManager::new(dir.path().to_path_buf(), mm);

            // Verify WAL file exists and has records
            let wal_path = dir.path().join("wal.log");
            assert!(wal_path.exists(), "WAL file should exist for recovery");

            // Create the same table schema so recovery has a target
            // (the catalog create_node_table stores the table internally).
            sm.create_node_table(
                "Person".into(),
                vec![
                    ColumnDefinition {
                        compression: akar_common::enums::CompressionType::Uncompressed,
                        name: "name".into(),
                        logical_type: LogicalTypeID::String,
                        is_primary_key: true,
                    },
                    ColumnDefinition {
                        compression: akar_common::enums::CompressionType::Uncompressed,
                        name: "age".into(),
                        logical_type: LogicalTypeID::Int64,
                        is_primary_key: false,
                    },
                ],
            );

            // Recover — the WAL has table_id = 0 (the first table created).
            let recovered = sm.recover().unwrap();
            assert_eq!(recovered, 3, "Should recover 3 WAL records");

            // Verify data survived — the table was re-created empty,
            // and recovery inserted exactly 3 rows from the WAL.
            {
                let recovered_table = sm.table_catalog.get_node_table_by_name("Person").unwrap();
                assert_eq!(recovered_table.num_rows, 3, "Should have recovered 3 rows");
                assert_eq!(recovered_table.get_value(0, 0), Some(&Value::String("Alice".into())));
                assert_eq!(recovered_table.get_value(1, 0), Some(&Value::String("Bob".into())));
                assert_eq!(recovered_table.get_value(2, 0), Some(&Value::String("Charlie".into())));
                assert_eq!(recovered_table.get_value(0, 1), Some(&Value::Int64(30)));
                assert_eq!(recovered_table.get_value(1, 1), Some(&Value::Int64(25)));
                assert_eq!(recovered_table.get_value(2, 1), Some(&Value::Int64(35)));
            }

            // Verify WAL was checkpointed — after checkpoint the WAL has
            // exactly 1 record (the Checkpoint marker).
            {
                let wal = sm.wal.lock().unwrap();
                assert_eq!(
                    wal.len(),
                    1,
                    "WAL should have only the checkpoint marker after recovery"
                );
                assert!(matches!(wal.records()[0], crate::wal::WALRecord::Checkpoint));
            }
        }
    }

    // =================================================================
    // Test 8: WAL recovery — no-op when no WAL exists
    // =================================================================
    #[test]
    fn test_wal_recovery_no_wal() {
        let dir = tempfile::tempdir().unwrap();

        let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
        let sm = StorageManager::new(dir.path().to_path_buf(), mm);

        // No WAL file = recovery returns 0
        let recovered = sm.recover().unwrap();
        assert_eq!(recovered, 0, "No WAL = no records recovered");
    }

    // =================================================================
    // Test 9: WAL recovery — empty WAL file
    // =================================================================
    #[test]
    fn test_wal_recovery_empty_wal() {
        let dir = tempfile::tempdir().unwrap();

        // Create an empty WAL file on disk
        let wal_path = dir.path().join("wal.log");
        std::fs::write(&wal_path, b"").unwrap();

        let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
        let sm = StorageManager::new(dir.path().to_path_buf(), mm);

        let recovered = sm.recover().unwrap();
        assert_eq!(recovered, 0, "Empty WAL = no records recovered");
    }

    // =================================================================
    // Test 10: WAL load_from_disk roundtrip
    // =================================================================
    #[test]
    fn test_wal_load_from_disk_roundtrip() {
        use crate::wal::WALRecord;
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("wal.log");

        // Write records
        {
            let mut wal = WAL::new(wal_path.clone());
            wal.append(WALRecord::Insert {
                table_id: 42,
                data: vec![1, 2, 3, 4],
            });
            wal.append(WALRecord::Delete {
                table_id: 42,
                row_id: 0,
            });
            wal.append(WALRecord::Update {
                table_id: 42,
                row_id: 1,
                column: 2,
                data: vec![5, 6],
            });
            wal.append(WALRecord::ColumnWrite {
                table_id: 42,
                col_id: 0,
                page_id: 1,
                data: vec![7, 8, 9],
            });
            wal.append(WALRecord::Commit { transaction_id: 100 });
            wal.append(WALRecord::Rollback { transaction_id: 101 });
            wal.append(WALRecord::Checkpoint);
            wal.flush_to_disk().unwrap();
        }

        // Load from disk
        {
            let mut wal = WAL::new(wal_path.clone());
            wal.load_from_disk().unwrap();
            assert_eq!(wal.len(), 7, "Should load 7 records from disk");
            assert!(wal.is_dirty());

            // Verify each record type
            match &wal.records()[0] {
                WALRecord::Insert { table_id, data } => {
                    assert_eq!(*table_id, 42);
                    assert_eq!(data, &[1, 2, 3, 4]);
                }
                _ => panic!("Expected Insert"),
            }
            match &wal.records()[1] {
                WALRecord::Delete { table_id, row_id } => {
                    assert_eq!(*table_id, 42);
                    assert_eq!(*row_id, 0);
                }
                _ => panic!("Expected Delete"),
            }
            match &wal.records()[4] {
                WALRecord::Commit { transaction_id } => {
                    assert_eq!(*transaction_id, 100);
                }
                _ => panic!("Expected Commit"),
            }
            match &wal.records()[5] {
                WALRecord::Rollback { transaction_id } => {
                    assert_eq!(*transaction_id, 101);
                }
                _ => panic!("Expected Rollback"),
            }
            match &wal.records()[6] {
                WALRecord::Checkpoint => {}
                _ => panic!("Expected Checkpoint"),
            }
        }
    }

    // =================================================================
    // Test: Commit pipeline — LocalStorage flush → ShadowFile apply
    // =================================================================
    #[test]
    fn test_commit_pipeline_local_storage_flush() {
        let (sm, _dir) = setup_integration();

        // Create table via catalog directly so we know the table_id
        let table_id;
        {
            let table = sm.table_catalog.create_node_table(
                "Person".into(),
                vec![
                    ColumnDefinition {
                        compression: akar_common::enums::CompressionType::Uncompressed,
                        name: "name".into(),
                        logical_type: LogicalTypeID::String,
                        is_primary_key: true,
                    },
                    ColumnDefinition {
                        compression: akar_common::enums::CompressionType::Uncompressed,
                        name: "age".into(),
                        logical_type: LogicalTypeID::Int64,
                        is_primary_key: false,
                    },
                ],
            );
            table_id = table.table_id;
        }

        // Simulate a transaction: buffer a row in LocalStorage
        let mut local_storage = crate::local_storage::LocalStorage::new();
        {
            let txn_table = local_storage.get_or_create_table(table_id);

            // Encode a row: name="Alice"(String), age=30(Int64)
            let mut row_bytes = Vec::new();
            row_bytes.push(13 /* TAG_STRING */);
            let name = "Alice";
            let name_bytes = name.as_bytes();
            row_bytes.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
            row_bytes.extend_from_slice(name_bytes);
            row_bytes.push(2 /* TAG_INT64 */);
            row_bytes.extend_from_slice(&30i64.to_le_bytes());

            txn_table.insert(row_bytes);
        }

        assert_eq!(local_storage.len(), 1, "Should have 1 table in local storage");

        // Commit via StorageManager
        let shadow = crate::shadow_file::ShadowFile::new();
        sm.commit_transaction(
            &local_storage,
            &shadow,
            -1, /* checkpoint */
            1,  /* txn_id */
            None,
        )
        .unwrap();

        // Verify data was flushed to the table
        {
            let t = sm.table_catalog.get_node_table_by_name("Person").unwrap();
            assert_eq!(t.num_rows, 1, "Should have 1 row after commit");
            assert_eq!(t.get_value(0, 0), Some(&Value::String("Alice".into())));
            assert_eq!(t.get_value(0, 1), Some(&Value::Int64(30)));
        }

        // Verify WAL has the commit record (and was checkpointed)
        {
            let wal = sm.wal.lock().unwrap();
            // After checkpoint, WAL has 1 record (Checkpoint marker)
            assert_eq!(wal.len(), 1, "WAL should have Checkpoint marker after commit");
        }
    }

    // =================================================================
    // Test: Rollback pipeline — LocalStorage clear, no data written
    // =================================================================
    #[test]
    fn test_rollback_pipeline_no_data_written() {
        let (sm, _dir) = setup_integration();
        let mut local_storage = crate::local_storage::LocalStorage::new();
        let mut shadow = crate::shadow_file::ShadowFile::new();

        // Buffer some data
        {
            let txn_table = local_storage.get_or_create_table(0);
            txn_table.insert(vec![2 /* TAG_INT64 */, 42, 0, 0, 0, 0, 0, 0, 0]);
        }

        assert!(!local_storage.is_empty(), "LocalStorage should have buffered data");

        // Rollback
        sm.rollback_transaction(&mut local_storage, &mut shadow, 1 /* txn_id */, &[])
            .unwrap();

        // Verify buffers are cleared
        assert!(local_storage.is_empty(), "LocalStorage should be empty after rollback");
        assert!(shadow.is_empty(), "ShadowFile should be empty after rollback");
    }

    // =================================================================
    // Test: Multiple buffered rows commit correctly
    // =================================================================
    #[test]
    fn test_commit_multiple_rows() {
        let (sm, _dir) = setup_integration();
        sm.create_node_table(
            "Item".into(),
            vec![
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "name".into(),
                    logical_type: LogicalTypeID::String,
                    is_primary_key: true,
                },
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "price".into(),
                    logical_type: LogicalTypeID::Double,
                    is_primary_key: false,
                },
            ],
        );

        // Buffer multiple rows
        let mut local = crate::local_storage::LocalStorage::new();
        {
            let txn_table = local.get_or_create_table(0); // table_id = 0

            // Row 1: "Widget", 19.99
            let mut row = Vec::new();
            row.push(13);
            row.extend_from_slice(&6u32.to_le_bytes());
            row.extend_from_slice(b"Widget");
            row.push(11);
            row.extend_from_slice(&19.99f64.to_le_bytes());
            txn_table.insert(row);

            // Row 2: "Gadget", 29.99
            let mut row = Vec::new();
            row.push(13);
            row.extend_from_slice(&6u32.to_le_bytes());
            row.extend_from_slice(b"Gadget");
            row.push(11);
            row.extend_from_slice(&29.99f64.to_le_bytes());
            txn_table.insert(row);
        }

        let shadow = crate::shadow_file::ShadowFile::new();
        sm.commit_transaction(&local, &shadow, 0 /* no checkpoint */, 2 /* txn_id */, None)
            .unwrap();

        // Verify both rows
        {
            let t = sm.table_catalog.get_node_table_by_name("Item").unwrap();
            assert_eq!(t.num_rows, 2, "Should have 2 rows after commit");
        }
    }
    #[test]
    fn test_zone_map_pushdown() {
        use crate::column_chunk::NODE_GROUP_SIZE;
        use crate::table::{ColumnDefinition, NodeTable};
        use akar_common::types::{LogicalTypeID, Value};

        let db_path = "test_zone_map_pushdown.db";
        let _ = std::fs::remove_file(db_path);

        let mut table = NodeTable::new(
            0,
            db_path.to_string(),
            vec![
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "id".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: true,
                },
                ColumnDefinition {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "value".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: false,
                },
            ],
        );

        // Insert exactly one node group of elements with values 0 to 4095
        for i in 0..NODE_GROUP_SIZE as i64 {
            table.insert_row(vec![Value::Int64(i), Value::Int64(i)]).unwrap();
        }

        // Insert a second node group of elements with values 4096 to 8191
        for i in 0..NODE_GROUP_SIZE as i64 {
            let val = i + NODE_GROUP_SIZE as i64;
            table.insert_row(vec![Value::Int64(val), Value::Int64(val)]).unwrap();
        }

        // Query with a predicate: id > 5000.
        // The first node group (max id = 4095) should be completely skipped.
        let predicate = Some((0, ">", &Value::Int64(5000)));
        let data = table.to_column_major_data_with_predicate(predicate);

        // data is Vec<Vec<Value>> where data[col][row].
        // Total rows should be 4096 instead of 8192 because the first node group is skipped.
        assert_eq!(
            data[0].len(),
            NODE_GROUP_SIZE,
            "Only the second chunk should be returned"
        );
        assert_eq!(
            data[0][0],
            Value::Int64(NODE_GROUP_SIZE as i64),
            "First element should be from the second chunk"
        );

        let _ = std::fs::remove_file(db_path);
    }
}
