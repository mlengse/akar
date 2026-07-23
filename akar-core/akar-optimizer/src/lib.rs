//! Query optimizer — applies optimization passes to logical plans.

pub mod join_order;
pub mod optimizer;
pub mod passes;

#[cfg(test)]
mod passes_test;

pub use optimizer::Optimizer;
