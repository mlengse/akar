//! Database — the main entry point for Kuzu.

use kuzu_catalog::Catalog;
use kuzu_common::memory::MemoryManager;
use kuzu_common::task_system::TaskSystem;
use kuzu_extension::{ExtensionRegistry, ExtensionContext};
use kuzu_function::FunctionRegistry;
use kuzu_storage::StorageManager;
use kuzu_transaction::TransactionManager;
use std::path::PathBuf;
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
        }
    }
}

/// The main database instance.
#[allow(dead_code)]
pub struct Database {
    pub(crate) storage_manager: Arc<StorageManager>,
    pub(crate) catalog: Arc<Mutex<Catalog>>,
    pub(crate) transaction_manager: Arc<TransactionManager>,
    pub(crate) function_registry: Arc<Mutex<FunctionRegistry>>,
    pub(crate) task_system: Arc<TaskSystem>,
    pub(crate) memory_manager: Arc<MemoryManager>,
    pub(crate) extension_registry: Mutex<ExtensionRegistry>,
}

impl Database {
    pub fn new(db_path: impl Into<PathBuf>, config: SystemConfig) -> Result<Self, String> {
        let db_path = db_path.into();
        let memory_manager = Arc::new(MemoryManager::new(config.max_db_size));
        let task_system = Arc::new(TaskSystem::new(config.max_num_threads as usize));
        let catalog = Arc::new(Mutex::new(Catalog::new()));
        let transaction_manager = Arc::new(TransactionManager::new());
        let function_registry = Arc::new(Mutex::new(FunctionRegistry::new()));
        let storage_manager = Arc::new(StorageManager::new(db_path, memory_manager.clone()));

        let mut db = Self {
            storage_manager,
            catalog,
            transaction_manager,
            function_registry,
            task_system,
            memory_manager,
            extension_registry: Mutex::new(ExtensionRegistry::new()),
        };

        // Load built-in extensions
        db.register_builtin_extensions();

        // Load all registered extensions
        {
            let mut ext_registry = db.extension_registry.lock().unwrap();
            let context = ExtensionContext::new(
                db.function_registry.clone(),
                db.catalog.clone(),
            );
            for result in ext_registry.load_all(&context) {
                match result {
                    (name, Ok(())) => tracing::info!("Extension '{name}' loaded successfully"),
                    (name, Err(e)) => tracing::warn!("Extension '{name}' failed to load: {e}"),
                }
            }
        }

        Ok(db)
    }

    /// Register built-in extensions (JSON, FTS, etc.).
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
    }
}
