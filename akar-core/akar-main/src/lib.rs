//! Akar public API — Database, Connection, QueryResult, PreparedStatement.

pub mod connection;
pub mod database;
pub mod prepared_statement;
pub mod query_result;
pub mod remote;
pub mod storage_driver;

#[cfg(feature = "adbc")]
pub mod adbc;

#[cfg(all(feature = "ml-extension", not(akar_wasm)))]
pub mod ml;

#[cfg(test)]
mod connection_test;

/// Test helpers — shared setup/teardown utilities for all Akar tests.
/// Always compiled (not cfg(test)-gated) so integration tests in `tests/` can use them.
pub mod test_helpers;

pub use connection::Connection;
pub use database::{Database, SystemConfig};
pub use prepared_statement::PreparedStatement;
pub use query_result::QueryResult;
pub use remote::RemoteDatabase;
pub use storage_driver::StorageDriver;
