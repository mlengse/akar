//! Query optimizer — applies optimization passes to logical plans.

pub mod passes;
pub mod optimizer;

pub use optimizer::Optimizer;
