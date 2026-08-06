//! Tree optimization passes — operate on the operator tree in-place.

pub mod acc_hash_join;
pub mod agg_key_dep;
pub mod cardinality;
pub mod factorization;
pub mod foreign_join;
pub mod subquery_unnesting;

pub use acc_hash_join::AccHashJoinOptimization;
pub use agg_key_dep::AggKeyDependency;
pub use cardinality::CardinalityEstimation;
pub use factorization::FactorizationRewriting;
pub use foreign_join::ForeignJoinPushDown;
pub use subquery_unnesting::CorrelatedSubqueryUnnesting;
