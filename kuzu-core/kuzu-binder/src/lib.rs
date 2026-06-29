//! Binder — semantic analysis, symbol resolution, catalog lookup, type checking.

pub mod binder;
pub mod bound_statement;

pub use binder::Binder;
