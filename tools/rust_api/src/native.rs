//! Native Rust backend for Kuzu (using kuzu-core).
//!
//! This module provides the same public API as the C++ FFI backend,
//! but implemented entirely in Rust via `kuzu-main` and `kuzu-common`.

/// Re-export core types from kuzu-core.
pub use kuzu_main::{Database, SystemConfig, Connection, QueryResult, PreparedStatement};

/// Re-export common value types.
pub use kuzu_common::types::{Value, InternalID, LogicalTypeID};

/// The version of the Kuzu library.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Get the storage version number.
pub fn get_storage_version() -> u64 {
    0 // Native Rust storage version
}
