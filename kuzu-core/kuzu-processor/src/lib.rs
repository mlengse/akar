//! Query processor — executes physical query plans and produces results.

pub mod physical_operator;
pub mod processor;

pub use processor::QueryProcessor;
