//! Extension framework for Akar.
//!
//! Provides the `Extension` trait, `ExtensionRegistry`, and `ExtensionContext`
//! for registering external functionality (custom types, functions, table functions)
//! into the database engine.

pub mod context;
pub mod registry;

pub use context::ExtensionContext;
pub use registry::ExtensionRegistry;

/// The base trait for all Akar extensions.
///
/// Extensions are loaded at database initialization time and can register:
/// - Custom logical types (e.g., JSON)
/// - Scalar functions (e.g., `json_extract`, `stem`)
/// - Aggregate functions
/// - Table functions (e.g., `JSON_SCAN`, `QUERY_FTS_INDEX`)
pub trait Extension: Send + Sync {
    /// The unique name of this extension (e.g., "JSON", "FTS").
    fn name(&self) -> &'static str;

    /// Load this extension into the database context.
    ///
    /// Called during `Database::new()` after core initialization.
    fn load(&self, context: &ExtensionContext) -> Result<(), String>;
}
