//! Flat optimization passes — operate on `&[LogicalOperator]`.

pub mod aggregate_detection;
pub mod art_range_scan;
pub mod constant_folding;
pub mod filter_pushdown;
pub mod join_optimization;
pub mod ladybug;
pub mod projection_pushdown;
pub mod scan_ops;
pub mod top_k;
pub mod vector_similarity;

pub use aggregate_detection::AggregateDetection;
pub use art_range_scan::ArtRangeScanDetection;
pub use constant_folding::ConstantFolding;
pub use filter_pushdown::FilterPushDown;
pub use join_optimization::JoinOptimization;
pub use ladybug::{CountRelTable, OrderByPushDown, UnwindDedup};
pub use projection_pushdown::ProjectionPushDown;
pub use scan_ops::{CommonSubexpressionElimination, LimitPushDown, RemoveUnnecessaryOperators};
pub use top_k::TopKOptimization;
pub use vector_similarity::VectorSimilarityDetection;
