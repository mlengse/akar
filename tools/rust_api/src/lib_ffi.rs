// Original C++ FFI backend for Kuzu.
// This file is conditionally included when `native` feature is disabled.

#[path = "ffi-legacy/connection.rs"]
mod connection;
#[path = "ffi-legacy/database.rs"]
mod database;
#[path = "ffi-legacy/error.rs"]
mod error;
#[path = "ffi-legacy/ffi.rs"]
mod ffi;
#[path = "ffi-legacy/logical_type.rs"]
mod logical_type;
#[path = "ffi-legacy/query_result.rs"]
mod query_result;
#[path = "ffi-legacy/value.rs"]
mod value;

pub use connection::{Connection, PreparedStatement};
pub use database::{Database, SystemConfig};
pub use error::Error;
pub use logical_type::LogicalType;
pub use query_result::QueryResult;
pub use value::{InternalID, NodeVal, RelVal, Value};

pub use logical_type::LogicalType as DataType;

#[cfg(feature = "arrow")]
pub use query_result::ArrowIterator;

use std::ffi::CStr;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn get_storage_version() -> u64 {
    unsafe { ffi::ffi::get_storage_version() }
}
