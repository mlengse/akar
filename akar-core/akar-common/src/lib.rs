//! Akar common types and utilities.
//!
//! This crate contains the foundational types used by all other Akar modules:
//! - Type system (LogicalType, PhysicalType, Value)
//! - Vector and DataChunk (columnar data units)
//! - Serialization primitives
//! - Task system / thread pool
//! - Memory management
//! - File system abstraction

pub mod arrow_vector;
pub mod data_chunk;
pub mod enums;
pub mod file_system;
pub mod gzip_file_system;
pub mod memory;
pub mod progress_bar;
pub mod selection;
pub mod serialization;
pub mod task_system;
pub mod types;
pub mod vector;
