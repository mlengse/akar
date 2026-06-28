//! Kuzu -- in-process property graph database.
//!
//! By default, this crate uses a pure Rust implementation via kuzu-core.

#[cfg(feature = "native")]
mod native;

#[cfg(feature = "native")]
pub use native::*;

#[cfg(not(feature = "native"))]
include!("lib_ffi.rs");
