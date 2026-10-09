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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_aggregate_function_standard_uppercase() {
        assert!(matches!(
            parse_aggregate_function("COUNT"),
            AggregateFunction::Count
        ));
        assert!(matches!(
            parse_aggregate_function("COUNT(*)"),
            AggregateFunction::CountStar
        ));
        assert!(matches!(
            parse_aggregate_function("SUM"),
            AggregateFunction::Sum
        ));
        assert!(matches!(
            parse_aggregate_function("AVG"),
            AggregateFunction::Avg
        ));
        assert!(matches!(
            parse_aggregate_function("MIN"),
            AggregateFunction::Min
        ));
        assert!(matches!(
            parse_aggregate_function("MAX"),
            AggregateFunction::Max
        ));
        assert!(matches!(
            parse_aggregate_function("COLLECT"),
            AggregateFunction::Collect
        ));
        assert!(matches!(
            parse_aggregate_function("STDDEV"),
            AggregateFunction::StdDev
        ));
        assert!(matches!(
            parse_aggregate_function("VARIANCE"),
            AggregateFunction::Variance
        ));
        assert!(matches!(
            parse_aggregate_function("PERCENTILE_DISC"),
            AggregateFunction::PercentileDisc { percentile } if (percentile - 0.5).abs() < f64::EPSILON
        ));
        assert!(matches!(
            parse_aggregate_function("PERCENTILE_CONT"),
            AggregateFunction::PercentileCont { percentile } if (percentile - 0.5).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn test_parse_aggregate_function_case_insensitivity() {
        assert!(matches!(
            parse_aggregate_function("count"),
            AggregateFunction::Count
        ));
        assert!(matches!(
            parse_aggregate_function("cOuNt(*)"),
            AggregateFunction::CountStar
        ));
        assert!(matches!(
            parse_aggregate_function("Sum"),
            AggregateFunction::Sum
        ));
        assert!(matches!(
            parse_aggregate_function("aVg"),
            AggregateFunction::Avg
        ));
        assert!(matches!(
            parse_aggregate_function("min"),
            AggregateFunction::Min
        ));
        assert!(matches!(
            parse_aggregate_function("mAx"),
            AggregateFunction::Max
        ));
        assert!(matches!(
            parse_aggregate_function("collect"),
            AggregateFunction::Collect
        ));
        assert!(matches!(
            parse_aggregate_function("stddev"),
            AggregateFunction::StdDev
        ));
        assert!(matches!(
            parse_aggregate_function("Variance"),
            AggregateFunction::Variance
        ));
        assert!(matches!(
            parse_aggregate_function("percentile_disc"),
            AggregateFunction::PercentileDisc { percentile } if (percentile - 0.5).abs() < f64::EPSILON
        ));
        assert!(matches!(
            parse_aggregate_function("Percentile_Cont"),
            AggregateFunction::PercentileCont { percentile } if (percentile - 0.5).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn test_parse_aggregate_function_fallback() {
        assert!(matches!(
            parse_aggregate_function("UNKNOWN_FUNC"),
            AggregateFunction::Count
        ));
        assert!(matches!(
            parse_aggregate_function(""),
            AggregateFunction::Count
        ));
        assert!(matches!(
            parse_aggregate_function("  123  "),
            AggregateFunction::Count
        ));
    }

    #[test]
    fn test_physical_aggregate_operator_type_and_execution() {
        let op = PhysicalAggregate {
            group_by_cols: vec![],
            aggregate_functions: vec!["COUNT".to_string(), "sum".to_string()],
        };
        assert_eq!(op.operator_type(), "aggregate");

        let result = op.execute(vec![]);
        assert!(result.is_ok());
    }
}
