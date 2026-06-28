//! Query optimizer — applies optimization passes to logical plans.

pub mod passes;
pub mod optimizer;
pub mod join_order;

pub use optimizer::Optimizer;
