//! Search fusion and hybrid recall for Akar.
//!
//! Provides Reciprocal Rank Fusion (RRF) for merging multiple ranked
//! result sets, hybrid search (vector + FTS), and hierarchical
//! authority-weighted recall.

pub mod fused;
pub mod hierarchical;
pub mod hybrid;
pub mod hybrid_scan;
pub mod native_bm25;
pub mod rrf;

pub use fused::{FusedSearchConfig, fuse_vector_and_bm25, to_bm25_results, to_vector_results};
pub use hierarchical::{
    AuthorityConfig, HierarchicalRrfConfig, apply_authority, authority_multiplier, fuse_hierarchical,
    fuse_hierarchical_with_authority, tag_channel,
};
pub use hybrid::hybrid_search;
pub use hybrid_scan::{HybridScan, HybridScanConfig};
pub use native_bm25::{Bm25Params, NativeBm25Index, tokenize};
pub use rrf::rrf_fuse_ref;
