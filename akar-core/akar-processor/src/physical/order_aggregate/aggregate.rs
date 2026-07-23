//! Auto-extracted from physical_operator.rs
use crate::physical::order_aggregate::AggregateHashTable;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use akar_common::vector::DataChunk;
use akar_function::AggregateFunction;

// ==================== Aggregate ====================

/// Helper: parse an aggregate function name string into an AggregateFunction enum.
pub fn parse_aggregate_function(name: &str) -> AggregateFunction {
    match name.to_uppercase().as_str() {
        "COUNT" => AggregateFunction::Count,
        "COUNT(*)" => AggregateFunction::CountStar,
        "SUM" => AggregateFunction::Sum,
        "AVG" => AggregateFunction::Avg,
        "MIN" => AggregateFunction::Min,
        "MAX" => AggregateFunction::Max,
        "COLLECT" => AggregateFunction::Collect,
        "STDDEV" => AggregateFunction::StdDev,
        "VARIANCE" => AggregateFunction::Variance,
        "PERCENTILE_DISC" => AggregateFunction::PercentileDisc { percentile: 0.5 },
        "PERCENTILE_CONT" => AggregateFunction::PercentileCont { percentile: 0.5 },
        _ => AggregateFunction::Count,
    }
}

pub struct PhysicalAggregate {
    pub group_by_cols: Vec<u32>,
    pub aggregate_functions: Vec<String>,
}

impl PhysicalOperatorExec for PhysicalAggregate {
    fn operator_type(&self) -> &str {
        "aggregate"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let funcs: Vec<AggregateFunction> = self
            .aggregate_functions
            .iter()
            .map(|name| parse_aggregate_function(name))
            .collect();

        let table = AggregateHashTable::new(funcs, self.group_by_cols.clone(), Vec::new());
        table.aggregate(&input)
    }
}
