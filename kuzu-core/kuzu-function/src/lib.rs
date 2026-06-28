//! Built-in function registry, evaluation, and type system.

pub mod registry;
pub mod scalar;

pub use registry::{
    FunctionRegistry, ResolvedFunction,
    ScalarFunction, AggregateFunction, TableFunction,
    ArithmeticOp, ComparisonOp, StringOp, DateOp, ListOp,
    MapOp, StructOp, BooleanOp, UtilityOp, CastTarget,
};
pub use scalar::evaluate_scalar;
