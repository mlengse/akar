//! Built-in function registry, evaluation, and type system.

pub mod registry;
pub mod scalar;

pub use registry::{
    AggregateFunction, ArithmeticOp, BooleanOp, CastTarget, ComparisonOp, DateOp, FunctionRegistry, ListOp, MapOp,
    ResolvedFunction, ScalarFunction, StringOp, StructOp, TableFunction, UtilityOp,
};
pub use scalar::{evaluate_aggregate, evaluate_scalar};

// Re-export types needed for custom function callbacks
pub use kuzu_common::types::Value;
pub use kuzu_common::vector::DataChunk;
