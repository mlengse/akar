//! Built-in function registry, evaluation, and type system.

pub mod registry;
pub mod scalar;

pub use registry::{
    FunctionRegistry, ResolvedFunction,
    ScalarFunction, AggregateFunction, TableFunction,
    ArithmeticOp, ComparisonOp, StringOp, DateOp, ListOp,
    MapOp, StructOp, BooleanOp, UtilityOp, CastTarget,
};
pub use scalar::{evaluate_aggregate, evaluate_scalar};

// Re-export types needed for custom function callbacks
pub use kuzu_common::types::Value;
pub use kuzu_common::vector::DataChunk;
