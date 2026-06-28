//! Binder — semantic analysis, symbol resolution, catalog lookup, type checking.

pub mod bound_statement;
pub mod binder;

pub use binder::Binder;
