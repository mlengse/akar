//! Kuzu public API — Database, Connection, QueryResult, PreparedStatement.

pub mod connection;
pub mod database;
pub mod prepared_statement;
pub mod query_result;

#[cfg(feature = "adbc")]
pub mod adbc;

#[cfg(test)]
mod connection_test;

pub use connection::Connection;
pub use database::{Database, SystemConfig};
pub use prepared_statement::PreparedStatement;
pub use query_result::QueryResult;
