//! Physical operator modules.

pub mod types;
pub mod common;
pub mod scan_filter;
pub mod order_aggregate;
pub mod join_ops;
pub mod write_ops;
pub mod batch_insert;
pub mod index_lookup;
pub mod missing_ops;

pub use types::{
    HashJoinBucket, HashJoinTable, NodeSemiMask, OperatorResult, PhysicalOperatorExec, PhysicalSemiMasker,
};
pub use scan_filter::*;
pub use order_aggregate::*;
pub use join_ops::*;
pub use write_ops::*;
pub use batch_insert::*;
pub use index_lookup::*;
pub use missing_ops::*;
