//! Cypher query parser — converts query text to AST.
//!
//! Will replace the existing ANTLR4-based C++ parser with a pest.rs PEG parser.

pub mod ast;
pub mod parser;

pub use parser::parse;
