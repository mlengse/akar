//! Binder — semantic analysis, symbol resolution, catalog lookup, type checking.

pub mod binder;
pub mod bound_statement;
pub mod confidential_statement_analyzer;

#[cfg(test)]
mod binder_test;

pub use binder::Binder;
