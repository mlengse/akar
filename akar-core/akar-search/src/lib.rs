//! Search fusion and hybrid recall for Akar.
//!
//! Provides Reciprocal Rank Fusion (RRF) for merging multiple ranked
//! result sets, hybrid search (vector + FTS), and multi-perspective
//! recall with automatic RRF deduplication.

pub mod hybrid;
pub mod multi;
pub mod rrf;

pub use hybrid::hybrid_search;
pub use multi::multi_perspective_recall_with_id;
pub use rrf::rrf_fuse_ref;
