//! Flat optimization passes — operate on `&[LogicalOperator]`.

pub mod aggregate_detection;
pub mod aggregate_fusion;
pub mod art_range_scan;
pub mod constant_folding;
pub mod expression_inline;
pub mod filter_pushdown;
pub mod join_optimization;
pub mod ladybug;
pub mod predicate_pushdown;
pub mod projection_pushdown;
pub mod scan_ops;
pub mod sort_elision;
pub mod top_k;
pub mod vector_similarity;

pub use aggregate_detection::AggregateDetection;
pub use aggregate_fusion::AggregateFusion;
pub use art_range_scan::ArtRangeScanDetection;
pub use constant_folding::ConstantFolding;
pub use expression_inline::ExpressionInline;
pub use filter_pushdown::FilterPushDown;
pub use join_optimization::JoinOptimization;
pub use ladybug::{CountRelTable, OrderByPushDown, UnwindDedup};
pub use predicate_pushdown::PredicatePushDown;
pub use projection_pushdown::ProjectionPushDown;
pub use scan_ops::{CommonSubexpressionElimination, LimitPushDown, RemoveUnnecessaryOperators};
pub use sort_elision::SortElision;
pub use top_k::TopKOptimization;
pub use vector_similarity::VectorSimilarityDetection;
