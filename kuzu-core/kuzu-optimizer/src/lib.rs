//! Query optimizer — applies optimization passes to logical plans.

pub mod join_order;
pub mod optimizer;
pub mod passes;

pub use optimizer::Optimizer;
