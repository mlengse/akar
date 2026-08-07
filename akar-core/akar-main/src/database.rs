//! Database — the main entry point for Akar.

use akar_catalog::Catalog;
use akar_common::file_system::VirtualFileSystemRegistry;
use akar_common::memory::MemoryManager;
use akar_common::task_system::TaskSystem;
use akar_extension::{ExtensionContext, ExtensionRegistry};
use akar_function::FunctionRegistry;
use akar_storage::StorageManager;
use akar_storage::stats::StatsStore;
use akar_storage::table::ColumnDefinition;
use akar_transaction::TransactionManager;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Name of the file that holds the serialized system catalog.
///
/// The catalog file is the source of truth for DDL (schema changes survive
/// restarts via this file); WAL records only carry DML.
pub const CATALOG_FILE_NAME: &str = "catalog.json";

/// Name of the file holding the cross-process lock.
///
/// The lock file is created with an exclusive lock by the first process to
/// open a database directory and prevents a second process from opening the
/// same directory concurrently (P45.4). Read-only opens take a shared lock.
pub const LOCK_FILE_NAME: &str = "akar.lock";

/// Configuration for the database.
#[derive(Debug, Clone)]
pub struct SystemConfig {
    pub buffer_pool_size: u64,
    pub max_num_threads: u64,
    pub enable_compression: bool,
    pub read_only: bool,
    pub max_db_size: u64,
    pub auto_checkpoint: bool,
    pub checkpoint_threshold: i64,
    /// When true, multiple write transactions can run concurrently.
    /// When false, only one write transaction at a time is allowed.
    pub concurrent_writes: bool,
    /// Memory threshold (in bytes) for triggering disk spilling during
    /// bulk ingest (COPY FROM, large batch inserts).
    ///
    /// When a NodeGroup's estimated in-memory size exceeds this value,
    /// its data is spilled to a temp file and the in-memory buffer is
    /// cleared. After all rows are ingested, spilled files are merged
    /// back into persistent storage.
    ///
    /// Default: 80% of `buffer_pool_size`. Set to 0 to disable spilling.
    pub spill_threshold: u64,
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            buffer_pool_size: 0,
            max_num_threads: 0,
            enable_compression: true,
            read_only: false,
            max_db_size: u64::from(u32::MAX),
            auto_checkpoint: true,
            checkpoint_threshold: -1,
            concurrent_writes: true,
            // Default: 80% of buffer_pool_size, or 0 if not set
            spill_threshold: 0,
        }
    }
}

/// The main database instance.
///
/// Manages the storage engine, catalog, transaction manager, and all subsystem
/// instances. Create via [`Database::new`] with a path and [`SystemConfig`].
///
/// # Examples
///
/// ```no_run
/// use akar_main::database::{Database, SystemConfig};
/// use akar_main::connection::Connection;
///
/// let db = std::sync::Arc::new(Database::new("./my_db", SystemConfig::default())?);
/// let conn = Connection::new(&db);
/// conn.query("CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name))")?;
/// # Ok::<(), String>(())
/// ```
#[allow(dead_code)]
pub struct Database {
    pub(crate) storage_manager: Arc<StorageManager>,
    pub(crate) catalog: Arc<Mutex<Catalog>>,
    pub(crate) transaction_manager: Arc<TransactionManager>,
    pub(crate) function_registry: Arc<Mutex<FunctionRegistry>>,
    pub(crate) task_system: Arc<TaskSystem>,
    pub(crate) memory_manager: Arc<MemoryManager>,
    pub(crate) extension_registry: Mutex<ExtensionRegistry>,
    pub(crate) stats_store: Arc<Mutex<StatsStore>>,
    pub(crate) vfs: Arc<VirtualFileSystemRegistry>,
    /// Configuration used at database creation time.
    pub(crate) config: SystemConfig,
    /// Runtime-overridable spill threshold via `SET spill_threshold`.
    /// 0 means "use config default".
    spill_threshold_override: AtomicU64,
    /// Cross-process lock file handle (kept alive for the lifetime of the
    /// database so the OS lock is released only on `Database` drop).
    _lock_file: Option<std::fs::File>,
}

impl Database {
    /// Override the spill threshold at runtime (via `SET spill_threshold`).
    pub fn set_spill_threshold(&self, bytes: u64) {
        self.spill_threshold_override.store(bytes, Ordering::Relaxed);
    }

    /// Get the effective spill threshold in bytes.
    ///
    /// Priority:
    /// 1. Runtime override (via `SET spill_threshold`)
    /// 2. `config.spill_threshold`
    /// 3. 80% of `buffer_pool_size`
    /// 4. 0 (disabled)
    pub fn effective_spill_threshold(&self) -> u64 {
        let override_val = self.spill_threshold_override.load(Ordering::Relaxed);
        if override_val > 0 {
            return override_val;
        }
        if self.config.spill_threshold > 0 {
            return self.config.spill_threshold;
        }
        if self.config.buffer_pool_size > 0 {
            return (self.config.buffer_pool_size as f64 * 0.8) as u64;
        }
        0
    }

    /// Create a `Spiller` instance using the current database configuration.
    ///
    /// Returns `None` if spilling is disabled (threshold is 0).
    pub fn spiller(&self) -> Option<std::sync::Arc<akar_storage::Spiller>> {
        let threshold = self.effective_spill_threshold();
        if threshold == 0 {
            return None;
        }
        let spill_dir = self.storage_manager.db_path().join("spill");
        Some(std::sync::Arc::new(akar_storage::Spiller::new(spill_dir, threshold)))
    }

    /// Create a [`StorageDriver`] for programmatic storage-level access
    /// (page counts, buffer stats, file sizes, table counts) without Cypher.
    pub fn storage_driver(&self) -> crate::storage_driver::StorageDriver {
        crate::storage_driver::StorageDriver::new(self.storage_manager.clone(), self.catalog.clone(), self.vfs.clone())
    }

    /// Get a reference to the schema catalog for programmatic metadata access.
    pub fn catalog(&self) -> Arc<Mutex<Catalog>> {
        self.catalog.clone()
    }

    /// Get the data table catalog for programmatic data access.
    ///
    /// Prefer using the unified DDL methods on `Database` instead of
    /// accessing the table catalog directly.
    pub fn table_catalog(&self) -> Arc<akar_storage::TableCatalog> {
        self.storage_manager.table_catalog()
    }

    // ── Unified DDL operations ──────────────────────────────────────────
    //
    // These methods ensure storage-level table creation/deletion is atomic.
    // Schema entries are managed by the binder (via `Catalog`) during the
    // bind phase.  These methods handle the data-level side:
    // storage table creation, serial sequences, ART indexes.

    /// Create a node table: data table + serial sequences + ART index.
    ///
    /// The schema entry is created by the binder during `bind()`.
    /// This method creates the storage-level table and associated resources.
    pub fn create_node_table(&self, name: String, columns: Vec<akar_catalog::CatalogColumn>) -> Result<u64, String> {
        // 1. Create the data-level table
        let storage_columns: Vec<akar_storage::table::ColumnDefinition> = columns
            .iter()
            .map(|c| akar_storage::table::ColumnDefinition {
                name: c.name.clone(),
                logical_type: c.logical_type,
                is_primary_key: c.is_primary_key,
                compression: c.compression,
            })
            .collect();
        let node_table = self.storage_manager.create_node_table(name.clone(), storage_columns);
        let table_id = node_table.table_id;

        // 2. Auto-create backing sequences for SERIAL columns
        {
            let mut cat = self.catalog.lock().map_err(|e| format!("Lock poisoned: {e}"))?;
            for col in &columns {
                if col.logical_type == akar_common::types::LogicalTypeID::Serial {
                    if let akar_catalog::CatalogResult::Created { .. } = cat.create_serial_sequence(&name, &col.name) {
                        tracing::info!("Created serial sequence for {name}.{}", col.name);
                    }
                }
            }
        }

        // 3. Auto-create ART index for primary key
        if columns.iter().any(|c| c.is_primary_key) {
            let index_name = format!("{name}_pk_idx");
            self.storage_manager
                .create_art_index(&name, &index_name)
                .map_err(|e| format!("Failed to create ART PK index for table '{name}': {e}"))?;
        }

        tracing::info!("Created node table '{name}'");
        Ok(table_id)
    }

    /// Create a rel table: data table.
    ///
    /// The schema entry is created by the binder during `bind()`.
    /// This method creates the storage-level table.
    pub fn create_rel_table(
        &self,
        name: String,
        src_table_id: u64,
        dst_table_id: u64,
        columns: Vec<akar_catalog::CatalogColumn>,
    ) -> Result<u64, String> {
        // 1. Create the data-level table
        let storage_columns: Vec<akar_storage::table::ColumnDefinition> = columns
            .iter()
            .map(|c| akar_storage::table::ColumnDefinition {
                name: c.name.clone(),
                logical_type: c.logical_type,
                is_primary_key: c.is_primary_key,
                compression: c.compression,
            })
            .collect();
        let rel_table =
            self.storage_manager
                .create_rel_table(name.clone(), src_table_id, dst_table_id, storage_columns);
        let table_id = rel_table.table_id;

        tracing::info!("Created rel table '{name}'");
        Ok(table_id)
    }

    /// Drop a table: serial sequences + data table + schema entry.
    pub fn drop_table(&self, name: &str) -> Result<(), String> {
        // 1. Drop auto-created serial sequences
        {
            let mut cat = self.catalog.lock().map_err(|e| format!("Lock poisoned: {e}"))?;
            let serial_seqs: Vec<String> = cat
                .sequences()
                .iter()
                .filter(|s| s.name.ends_with("_serial"))
                .filter(|s| s.name.starts_with(&format!("{name}_")))
                .map(|s| s.name.clone())
                .collect();
            for seq_name in serial_seqs {
                if let akar_catalog::CatalogResult::Dropped { .. } = cat.drop_sequence(&seq_name) {
                    tracing::info!("Dropped serial sequence '{seq_name}'");
                }
            }
            // Drop schema entry
            cat.drop_table(name);
        }

        // 2. Drop the data table
        let table_catalog = self.storage_manager.table_catalog();
        let node_tid = table_catalog.get_node_table_by_name(name).map(|t| t.table_id);
        let rel_tid = table_catalog.get_rel_table_by_name(name).map(|t| t.table_id);
        table_catalog.drop_node_table(name);
        table_catalog.drop_rel_table(name);
        if let Some(tid) = node_tid {
            self.storage_manager.drop_table_persistence(tid);
        }
        if let Some(tid) = rel_tid {
            self.storage_manager.drop_table_persistence(tid);
        }

        tracing::info!("Dropped table '{name}'");
        Ok(())
    }

    /// Create a vector index: data index + auto-populate.
    ///
    /// The schema entry is created by the binder during `bind()`.
    #[cfg(feature = "vector-extension")]
    pub fn create_vector_index(
        &self,
        index_name: String,
        table_name: String,
        column_name: String,
        metric: akar_vector::hnsw::DistanceMetric,
        dimensions: u32,
    ) -> Result<(), String> {
        // 1. Create the data-level index
        self.storage_manager.create_vector_index(
            index_name.clone(),
            table_name.clone(),
            column_name.clone(),
            metric,
            dimensions,
        );

        // 2. Auto-populate from existing table data
        let table_catalog = self.storage_manager.table_catalog();
        if let Some(table) = table_catalog.get_node_table_by_name(&table_name) {
            let col_idx = table.columns.iter().position(|c| c.name == column_name);
            if let Some(col_idx) = col_idx {
                for row_id in 0..table.num_rows as usize {
                    if let Some(val) = table.get_value(row_id, col_idx) {
                        if let Ok(vec) = akar_storage::extract_f64_list_from_value(val) {
                            if let Some(mut vi) = table_catalog.get_vector_index_by_name_mut(&index_name) {
                                vi.hnsw_mut().insert(vec, row_id);
                            }
                        }
                    }
                }
            }
        }

        tracing::info!("Created vector index '{index_name}'");
        Ok(())
    }

    /// Create an ART index on a node table: data index.
    ///
    /// The schema entry is created by the binder during `bind()`.
    pub fn create_art_index(&self, table_name: &str, index_name: &str) -> Result<(), String> {
        self.storage_manager.create_art_index(table_name, index_name)?;
        Ok(())
    }

    /// Drop an ART index from a node table: data index + schema entry.
    pub fn drop_art_index(&self, table_name: &str, _index_name: &str) -> Result<(), String> {
        self.storage_manager.drop_art_index(table_name, _index_name)?;

        // Update the schema entry
        {
            let mut cat = self.catalog.lock().map_err(|e| format!("Lock poisoned: {e}"))?;
            if let Some(entry) = cat.get_entry_by_name_mut(table_name) {
                if let akar_catalog::CatalogEntry::NodeTable(t) = entry {
                    t.index_type = None;
                    t.index_name = None;
                }
            }
        }

        Ok(())
    }

    /// Get the number of rows in a table by name.
    pub fn table_num_rows(&self, name: &str) -> u64 {
        self.storage_manager.table_catalog().node_table_num_rows(name)
    }

    /// Get table IDs for write operations (used by transaction locking).
    pub fn get_table_id(&self, name: &str) -> Option<u64> {
        self.catalog.lock().ok()?.get_table_id(name)
    }

    /// Path of the persisted catalog file for this database.
    pub fn catalog_file_path(&self) -> PathBuf {
        self.storage_manager.db_path().join(CATALOG_FILE_NAME)
    }

    /// Connect to a remote Akar server (embedded server mode, P47).
    ///
    /// The server process owns the [`Database`] instance and its exclusive file
    /// lock; remote clients only talk to the server over TCP and never open the
    /// database directory themselves. Multiple processes can therefore access
    /// the same database concurrently: one writer via the server plus any
    /// number of read-only clients (and additional writers when
    /// `concurrent_writes` is enabled).
    ///
    /// See [`crate::remote::RemoteDatabase`] for the returned handle's API.
    pub fn connect_tcp(addr: impl Into<String>) -> Result<crate::remote::RemoteDatabase, String> {
        crate::remote::RemoteDatabase::connect_tcp(addr)
    }

    /// Returns `true` when the database runs fully in-memory (`:memory:`),
    /// in which case catalog persistence is skipped.
    pub fn is_in_memory(&self) -> bool {
        self.storage_manager.db_path().to_string_lossy() == ":memory:"
    }

    /// Persist the system catalog to disk.
    ///
    /// Called after every DDL statement so schema changes survive restarts.
    /// The write is atomic (temp file + rename) and no-ops for `:memory:`
    /// databases.
    pub fn persist_catalog(&self) -> Result<(), String> {
        if self.is_in_memory() {
            return Ok(());
        }
        let catalog = self.catalog.lock().map_err(|e| format!("Lock poisoned: {e}"))?;
        catalog
            .save_to_path(&self.catalog_file_path())
            .map_err(|e| format!("Failed to persist catalog: {e}"))
    }

    /// Restore storage-level tables from the loaded catalog.
    ///
    /// Called during `Database::new` after the catalog file is loaded so that
    /// WAL DML replay and subsequent queries reference the same table IDs that
    /// were in use when the database was shut down.
    fn restore_storage_from_catalog(&self) {
        let catalog = match self.catalog.lock() {
            Ok(c) => c,
            Err(_) => return,
        };
        for entry in catalog.all_entries() {
            match entry {
                akar_catalog::CatalogEntry::NodeTable(t) => {
                    let columns: Vec<_> = t.columns.iter().map(ColumnDefinition::from).collect();
                    let index_name = if t.has_art_index() {
                        t.index_name.as_deref()
                    } else {
                        None
                    };
                    self.storage_manager
                        .restore_node_table(t.table_id, t.name.clone(), columns, index_name);
                }
                akar_catalog::CatalogEntry::RelTable(t) => {
                    let columns: Vec<_> = t.columns.iter().map(ColumnDefinition::from).collect();
                    self.storage_manager.restore_rel_table(
                        t.table_id,
                        t.name.clone(),
                        t.src_table_id,
                        t.dst_table_id,
                        columns,
                    );
                }
                _ => {}
            }
        }
    }

    /// Open or create a database at the given path.
    ///
    /// # Arguments
    /// * `db_path` — filesystem path where database files are stored.
    /// * `config` — buffer pool size, thread count, etc.
    ///
    /// # Errors
    /// Returns `Err` if the path is not writable or if existing data is corrupt.
    pub fn new(db_path: impl Into<PathBuf>, config: SystemConfig) -> Result<Self, String> {
        let db_path = db_path.into();
        let is_memory = db_path.to_string_lossy() == ":memory:";

        // Multi-process guard: take a file lock on <db_path>/akar.lock so two
        // processes cannot open the same database directory concurrently.
        // Read-only opens take a shared lock (multiple readers allowed); write
        // opens take an exclusive lock. The handle is kept alive for the
        // lifetime of the Database to hold the lock.
        let lock_file = if is_memory {
            None
        } else {
            std::fs::create_dir_all(&db_path)
                .map_err(|e| format!("Failed to create database directory '{}': {e}", db_path.display()))?;
            let lock_path = db_path.join(LOCK_FILE_NAME);
            let file = std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .open(&lock_path)
                .map_err(|e| format!("Failed to open lock file '{}': {e}", lock_path.display()))?;
            let result = if config.read_only {
                file.try_lock_shared()
            } else {
                file.try_lock()
            };
            result.map_err(|_| format!("Database '{}' is already open by another process", db_path.display()))?;
            Some(file)
        };

        let memory_manager = Arc::new(MemoryManager::new(config.max_db_size));
        let task_system = Arc::new(TaskSystem::new(config.max_num_threads as usize));

        // Load a previously-persisted catalog (if any). Fresh databases and
        // databases created before catalog persistence start with an empty
        // catalog, preserving backward compatibility.
        let catalog_file = db_path.join(CATALOG_FILE_NAME);
        let catalog = Arc::new(Mutex::new(
            akar_catalog::Catalog::load_from_path(&catalog_file)
                .map_err(|e| format!("Failed to load persisted catalog: {e}"))?
                .unwrap_or_default(),
        ));
        let transaction_manager = {
            let tx_config = akar_transaction::TransactionManagerConfig {
                concurrent_writes: config.concurrent_writes,
            };
            Arc::new(TransactionManager::new_with_config(tx_config))
        };
        let function_registry = Arc::new(Mutex::new(FunctionRegistry::new()));
        let storage_manager = Arc::new(StorageManager::new(db_path.clone(), memory_manager.clone()));
        let stats_store = Arc::new(Mutex::new(StatsStore::new()));
        let vfs = Arc::new(VirtualFileSystemRegistry::new());

        let mut db = Self {
            storage_manager,
            catalog,
            transaction_manager,
            function_registry,
            task_system,
            memory_manager,
            extension_registry: Mutex::new(ExtensionRegistry::new()),
            stats_store,
            vfs,
            spill_threshold_override: AtomicU64::new(0),
            _lock_file: lock_file,
            config,
        };

        // Recreate storage-level tables from the restored catalog (if any) so
        // WAL DML replay below operates on the same table IDs.
        db.restore_storage_from_catalog();

        // Load built-in extensions
        db.register_builtin_extensions();

        // Load all registered extensions
        {
            let mut ext_registry = db
                .extension_registry
                .lock()
                .map_err(|e| format!("Lock poisoned: {e}"))?;
            let context = ExtensionContext::new(db.function_registry.clone(), db.catalog.clone(), db.vfs.clone());
            for result in ext_registry.load_all(&context) {
                match result {
                    (name, Ok(())) => tracing::info!("Extension '{name}' loaded successfully"),
                    (name, Err(e)) => tracing::warn!("Extension '{name}' failed to load: {e}"),
                }
            }
        }

        // Register built-in sequence functions (nextval/currval)
        {
            let mut reg = db.function_registry.lock().map_err(|e| format!("Lock poisoned: {e}"))?;
            crate::connection::utils::register_sequence_scalars(&mut reg, db.catalog.clone());
        }

        // Restore durable column mirrors when the WAL has nothing to replay.
        //
        // `recover()` returns the number of data records it replayed. When it
        // returns 0 (fresh database, clean shutdown, or last checkpoint), the
        // committed rows live in the durable column mirrors and are restored
        // here. When it replayed WAL records, the mirrors are re-persisted by
        // recover(), so they are only consulted when they are authoritative —
        // this avoids double-applying committed rows (P45.4).
        let recovered = match db.storage_manager.recover() {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(
                    "WAL recovery failed (database may need manual repair): {e}. \
                     Starting with fresh state."
                );
                0
            }
        };
        if recovered == 0 {
            match db.storage_manager.load_persisted_tables() {
                Ok(n) => {
                    if n > 0 {
                        tracing::info!("Restored {n} table(s) from durable column mirrors");
                    }
                }
                Err(e) => tracing::warn!("Failed to restore tables from column mirrors: {e}"),
            }
        }

        Ok(db)
    }

    /// Register built-in extensions (JSON, FTS, Vector, HTTPFS, DuckDB).
    fn register_builtin_extensions(&mut self) {
        #[cfg(feature = "json-extension")]
        {
            let ext = Box::new(akar_json::JsonExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(feature = "fts-extension")]
        {
            let ext = Box::new(akar_fts::FtsExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(feature = "vector-extension")]
        {
            let ext = Box::new(akar_vector::VectorExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(all(feature = "httpfs-extension", not(akar_wasm)))]
        {
            let ext = Box::new(akar_httpfs::HttpfsExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(all(feature = "duckdb-extension", not(akar_wasm)))]
        {
            let ext = Box::new(akar_duckdb::DuckDbExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(feature = "algo-extension")]
        {
            let ext = Box::new(akar_algo::AlgoExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(feature = "neo4j-extension")]
        {
            let ext = Box::new(akar_neo4j::Neo4jExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(feature = "llm-extension")]
        {
            let ext = Box::new(akar_llm::LlmExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(all(feature = "sqlite-extension", not(akar_wasm)))]
        {
            let ext = Box::new(akar_sqlite::SqliteExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(any(feature = "delta-extension", feature = "delta-native"))]
        {
            let ext = Box::new(akar_delta::DeltaExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(any(feature = "iceberg-extension", feature = "iceberg-native"))]
        {
            let ext = Box::new(akar_iceberg::IcebergExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(any(feature = "azure-extension", feature = "azure-native"))]
        {
            let ext = Box::new(akar_azure::AzureExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(all(feature = "postgres-extension", not(akar_wasm)))]
        {
            let ext = Box::new(akar_postgres::PostgresExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(any(feature = "unity-catalog-extension", feature = "unity-catalog-native"))]
        {
            let ext = Box::new(akar_unity_catalog::UnityCatalogExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
    }
}
