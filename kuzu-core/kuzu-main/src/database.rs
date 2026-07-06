//! Database — the main entry point for Kuzu.

use kuzu_catalog::Catalog;
use kuzu_common::file_system::VirtualFileSystemRegistry;
use kuzu_common::memory::MemoryManager;
use kuzu_common::task_system::TaskSystem;
use kuzu_extension::{ExtensionContext, ExtensionRegistry};
use kuzu_function::FunctionRegistry;
use kuzu_storage::StorageManager;
use kuzu_storage::stats::StatsStore;
use kuzu_transaction::TransactionManager;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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
/// use kuzu_main::database::{Database, SystemConfig};
/// use kuzu_main::connection::Connection;
///
/// let db = Database::new("./my_db", SystemConfig::default())?;
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
    pub fn spiller(&self) -> Option<std::sync::Arc<kuzu_storage::Spiller>> {
        let threshold = self.effective_spill_threshold();
        if threshold == 0 {
            return None;
        }
        let spill_dir = self.storage_manager.db_path().join("spill");
        Some(std::sync::Arc::new(kuzu_storage::Spiller::new(spill_dir, threshold)))
    }

    /// Get a reference to the table catalog for programmatic data access.
    pub fn catalog(&self) -> Arc<Mutex<Catalog>> {
        self.catalog.clone()
    }

    pub fn table_catalog(&self) -> Arc<kuzu_storage::TableCatalog> {
        self.storage_manager.table_catalog()
    }
    pub fn new(db_path: impl Into<PathBuf>, config: SystemConfig) -> Result<Self, String> {
        let db_path = db_path.into();
        let memory_manager = Arc::new(MemoryManager::new(config.max_db_size));
        let task_system = Arc::new(TaskSystem::new(config.max_num_threads as usize));
        let catalog = Arc::new(Mutex::new(Catalog::new()));
        let transaction_manager = {
            let tx_config = kuzu_transaction::TransactionManagerConfig {
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
            config,
        };

        // Load built-in extensions
        db.register_builtin_extensions();

        // Load all registered extensions
        {
            let mut ext_registry = db.extension_registry.lock().unwrap();
            let context = ExtensionContext::new(db.function_registry.clone(), db.catalog.clone(), db.vfs.clone());
            for result in ext_registry.load_all(&context) {
                match result {
                    (name, Ok(())) => tracing::info!("Extension '{name}' loaded successfully"),
                    (name, Err(e)) => tracing::warn!("Extension '{name}' failed to load: {e}"),
                }
            }
        }

        // Register built-in sequence functions (nextval/currval)
        // These use CustomScalar closures that capture the catalog
        {
            use kuzu_common::types::Value;
            use kuzu_function::registry::ScalarFunction;
            use std::sync::Arc;

            let catalog_seq = db.catalog.clone();
            let reg = db.function_registry.clone();
            let mut reg = reg.lock().unwrap();

            // currval(seq_name: string) -> int64
            let curr_catalog = catalog_seq.clone();
            reg.register_scalar(
                "currval",
                ScalarFunction::CustomScalar {
                    name: "currval".into(),
                    execute: Arc::new(move |args: &[Value]| -> Result<Value, String> {
                        if args.is_empty() {
                            return Err("currval requires a sequence name argument".into());
                        }
                        let seq_name = match &args[0] {
                            Value::String(s) => s.clone(),
                            other => return Err(format!("currval expects a string, got {:?}", other.logical_type())),
                        };
                        let cat = curr_catalog.lock().map_err(|e| format!("Catalog lock error: {e}"))?;
                        let seq = cat
                            .get_sequence(&seq_name)
                            .ok_or_else(|| format!("Sequence '{}' not found", seq_name))?;
                        Ok(Value::Int64(seq.curr_val()))
                    }),
                },
            );

            // nextval(seq_name: string) -> int64
            let next_catalog = catalog_seq.clone();
            reg.register_scalar(
                "nextval",
                ScalarFunction::CustomScalar {
                    name: "nextval".into(),
                    execute: Arc::new(move |args: &[Value]| -> Result<Value, String> {
                        if args.is_empty() {
                            return Err("nextval requires a sequence name argument".into());
                        }
                        let seq_name = match &args[0] {
                            Value::String(s) => s.clone(),
                            other => return Err(format!("nextval expects a string, got {:?}", other.logical_type())),
                        };
                        let mut cat = next_catalog.lock().map_err(|e| format!("Catalog lock error: {e}"))?;
                        let seq = cat
                            .get_sequence_mut(&seq_name)
                            .ok_or_else(|| format!("Sequence '{}' not found", seq_name))?;
                        let result = seq.next_k_val(1);
                        Ok(Value::Int64(result))
                    }),
                },
            );
        }

        // Attempt WAL recovery from a previous session
        if let Err(e) = db.storage_manager.recover() {
            tracing::warn!(
                "WAL recovery failed (database may need manual repair): {e}. \
                 Starting with fresh state."
            );
            // Do not fail — allow read-only or empty-state start
        }

        Ok(db)
    }

    /// Register built-in extensions (JSON, FTS, Vector, HTTPFS, DuckDB).
    fn register_builtin_extensions(&mut self) {
        #[cfg(feature = "json-extension")]
        {
            let ext = Box::new(kuzu_json::JsonExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(feature = "fts-extension")]
        {
            let ext = Box::new(kuzu_fts::FtsExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(feature = "vector-extension")]
        {
            let ext = Box::new(kuzu_vector::VectorExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(all(feature = "httpfs-extension", not(kuzu_wasm)))]
        {
            let ext = Box::new(kuzu_httpfs::HttpfsExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(all(feature = "duckdb-extension", not(kuzu_wasm)))]
        {
            let ext = Box::new(kuzu_duckdb::DuckDbExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(feature = "algo-extension")]
        {
            let ext = Box::new(kuzu_algo::AlgoExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(feature = "neo4j-extension")]
        {
            let ext = Box::new(kuzu_neo4j::Neo4jExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(feature = "llm-extension")]
        {
            let ext = Box::new(kuzu_llm::LlmExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(all(feature = "sqlite-extension", not(kuzu_wasm)))]
        {
            let ext = Box::new(kuzu_sqlite::SqliteExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(feature = "delta-extension")]
        {
            let ext = Box::new(kuzu_delta::DeltaExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(feature = "iceberg-extension")]
        {
            let ext = Box::new(kuzu_iceberg::IcebergExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(feature = "azure-extension")]
        {
            let ext = Box::new(kuzu_azure::AzureExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(all(feature = "postgres-extension", not(kuzu_wasm)))]
        {
            let ext = Box::new(kuzu_postgres::PostgresExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
        #[cfg(feature = "unity-catalog-extension")]
        {
            let ext = Box::new(kuzu_unity_catalog::UnityCatalogExtension::new());
            if let Ok(mut reg) = self.extension_registry.lock() {
                reg.register(ext);
            }
        }
    }
}
