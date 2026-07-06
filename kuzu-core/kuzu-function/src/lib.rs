//! Built-in function registry, evaluation, and type system.

pub mod registry;
pub mod scalar;
pub mod aggregate;

pub use registry::{
    AggregateFunction, ArithmeticOp, ArrayOp, BooleanOp, CastTarget, ComparisonOp, DateOp, FunctionRegistry, ListOp,
    MapOp, ResolvedFunction, ScalarFunction, SchemaOp, StringOp, StructOp, TableFunction, UtilityOp,
};
pub use scalar::evaluate_scalar;
pub use aggregate::{evaluate_aggregate, AggValueState};

// Re-export types needed for custom function callbacks
pub use kuzu_common::types::Value;
pub use kuzu_common::vector::DataChunk;

