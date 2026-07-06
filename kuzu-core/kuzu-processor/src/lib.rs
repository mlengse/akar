//! Query processor — executes physical query plans and produces results.

pub mod expression_evaluator;
pub mod physical;
pub mod physical_operator;
pub mod processor;

pub use expression_evaluator::ExpressionEvaluator;
pub use physical_operator::*;
pub use processor::QueryProcessor;
