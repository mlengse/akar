//! Binder — semantic analysis, symbol resolution, catalog lookup, type checking.

pub mod binder;
pub mod bound_statement;

#[cfg(test)]
mod binder_test;

pub use binder::Binder;
