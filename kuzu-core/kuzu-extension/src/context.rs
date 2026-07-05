//! Extension context — the API surface available to extensions during loading.
//!
//! Provides access to register functions, types, and other database components.

use kuzu_common::file_system::{FileSystem, VirtualFileSystemRegistry};
use kuzu_catalog::Catalog;
use kuzu_function::registry::{AggregateFunction, FunctionRegistry, ScalarFunction, TableFunction};
use std::sync::{Arc, Mutex};

/// Context passed to `Extension::load()` with hooks into the database engine.
pub struct ExtensionContext {
    pub(crate) function_registry: Arc<Mutex<FunctionRegistry>>,
    pub(crate) catalog: Arc<Mutex<Catalog>>,
    pub(crate) vfs: Arc<VirtualFileSystemRegistry>,
}

impl ExtensionContext {
    /// Create a new extension context.
    pub fn new(function_registry: Arc<Mutex<FunctionRegistry>>, catalog: Arc<Mutex<Catalog>>, vfs: Arc<VirtualFileSystemRegistry>) -> Self {
        Self {
            function_registry,
            catalog,
            vfs,
        }
    }

    /// Register a scalar function with the function registry.
    pub fn register_scalar_function(&self, name: &str, func: ScalarFunction) {
        if let Ok(mut reg) = self.function_registry.lock() {
            reg.register_scalar(name, func);
            tracing::debug!("Extension registered scalar function: {}", name);
        }
    }

    /// Register an aggregate function with the function registry.
    pub fn register_aggregate_function(&self, name: &str, func: AggregateFunction) {
        if let Ok(mut reg) = self.function_registry.lock() {
            reg.register_aggregate(name, func);
            tracing::debug!("Extension registered aggregate function: {}", name);
        }
    }

    /// Register a table function with the function registry.
    pub fn register_table_function(&self, name: &str, func: TableFunction) {
        if let Ok(mut reg) = self.function_registry.lock() {
            reg.register_table(name, func);
            tracing::debug!("Extension registered table function: {}", name);
        }
    }

    /// Get a reference to the catalog.
    pub fn catalog(&self) -> &Arc<Mutex<Catalog>> {
        &self.catalog
    }

    /// Get a reference to the function registry.
    pub fn function_registry(&self) -> &Arc<Mutex<FunctionRegistry>> {
        &self.function_registry
    }

    /// Register a virtual file system.
    pub fn register_file_system(&self, fs: Box<dyn FileSystem>) {
        self.vfs.register_file_system(fs);
        tracing::debug!("Extension registered file system");
    }
}
