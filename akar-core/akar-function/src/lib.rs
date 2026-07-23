//! Built-in function registry, evaluation, and type system.

pub mod aggregate;
pub mod registry;
pub mod scalar;

pub use aggregate::{AggValueState, evaluate_aggregate};
pub use registry::{
    AggregateFunction, ArithmeticOp, ArrayOp, BooleanOp, CastTarget, ComparisonOp, DateOp, FunctionRegistry, ListOp,
    MapOp, ResolvedFunction, ScalarFunction, SchemaOp, StringOp, StructOp, TableFunction, UtilityOp,
};
pub use scalar::evaluate_scalar;

// Re-export types needed for custom function callbacks
pub use akar_common::types::Value;
pub use akar_common::vector::DataChunk;
