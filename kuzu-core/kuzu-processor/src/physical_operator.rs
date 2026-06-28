//! Physical operator types for query execution.

use kuzu_common::vector::DataChunk;

/// A physical operator in the execution plan.
pub enum PhysicalOperator {
    Scan(PhysicalScan),
    Filter(PhysicalFilter),
    HashJoin(PhysicalHashJoin),
    Projection(PhysicalProjection),
    OrderBy(PhysicalOrderBy),
    Aggregate(PhysicalAggregate),
    Limit(PhysicalLimit),
    Union(PhysicalUnion),
}

pub struct PhysicalScan {
    pub table_id: u64,
    pub column_ids: Vec<u32>,
}

pub struct PhysicalFilter {
    pub predicate: Box<dyn Fn(&DataChunk) -> Vec<bool> + Send>,
}

pub struct PhysicalHashJoin {
    pub build_column_ids: Vec<u32>,
    pub probe_column_ids: Vec<u32>,
}

pub struct PhysicalProjection {
    pub expressions: Vec<usize>, // indices into the input DataChunk
}

pub struct PhysicalOrderBy {
    pub sort_columns: Vec<u32>,
    pub ascending: Vec<bool>,
}

pub struct PhysicalAggregate {
    pub group_by_cols: Vec<u32>,
    pub aggregate_functions: Vec<String>,
}

pub struct PhysicalLimit {
    pub limit: u64,
    pub offset: u64,
}

pub struct PhysicalUnion {
    pub left: Box<PhysicalOperator>,
    pub right: Box<PhysicalOperator>,
}
