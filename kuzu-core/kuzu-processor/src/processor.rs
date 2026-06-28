//! Query processor — maps logical operators to physical operators and executes them.
//!
//! Pipeline execution model:
//! 1. Scan operators produce raw DataChunks
//! 2. Filter removes non-matching rows
//! 3. Projection selects/transforms columns
//! 4. Limit/OrderBy/Aggregate are applied last

use crate::physical_operator::*;
use kuzu_common::types::PhysicalTypeID;
use kuzu_common::vector::{DataChunk, ValueVector};
use kuzu_planner::logical_operator::LogicalOperator;

/// The query processor executes a physical plan and produces result chunks.
pub struct QueryProcessor;

impl QueryProcessor {
    pub fn new() -> Self {
        Self
    }

    /// Execute a sequence of logical operators by mapping them to physical operators.
    pub fn execute(
        &self,
        operators: &[LogicalOperator],
    ) -> Result<Vec<DataChunk>, String> {
        if operators.is_empty() {
            return Ok(vec![DataChunk {
                fields: vec![],
                size: 0,
            }]);
        }

        // Map logical operators to physical and execute in pipeline
        let current = Vec::new();

        // Execute each logical operator
        let mut intermediate_result: Option<Vec<DataChunk>> = None;

        for op in operators {
            match op {
                LogicalOperator::ScanNode(s) => {
                    let scan = PhysicalScan {
                        table_name: s.table_name.clone(),
                        table_id: s.table_id,
                        column_ids: Vec::new(),
                        estimated_cardinality: 1000,
                    };
                    let result = scan.execute(current.clone())?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::ScanRel(s) => {
                    let scan = PhysicalScan {
                        table_name: s.table_name.clone(),
                        table_id: s.table_id,
                        column_ids: Vec::new(),
                        estimated_cardinality: 500,
                    };
                    let result = scan.execute(current.clone())?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Filter(f) => {
                    let filter = PhysicalFilter {
                        expression: f.expression.clone(),
                    };
                    let input = intermediate_result.take().unwrap_or_default();
                    let result = filter.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Projection(p) => {
                    let proj = PhysicalProjection {
                        column_indices: (0..p.expressions.len()).collect(),
                    };
                    let input = intermediate_result.take().unwrap_or_default();
                    let result = proj.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Limit(l) => {
                    let limit = PhysicalLimit {
                        limit: l.limit,
                        offset: l.offset,
                    };
                    let input = intermediate_result.take().unwrap_or_default();
                    let result = limit.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::OrderBy(o) => {
                    let order = PhysicalOrderBy {
                        sort_column: 0,
                        ascending: o.sort_keys.first().map(|s| s.1).unwrap_or(true),
                    };
                    let input = intermediate_result.take().unwrap_or_default();
                    let result = order.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::Aggregate(a) => {
                    let agg = PhysicalAggregate {
                        group_by_cols: Vec::new(),
                        aggregate_functions: a.aggregates.iter().map(|(n, _)| n.clone()).collect(),
                    };
                    let input = intermediate_result.take().unwrap_or_default();
                    let result = agg.execute(input)?;
                    intermediate_result = Some(result);
                }
                LogicalOperator::HashJoin(_) | LogicalOperator::CrossProduct(_)
                | LogicalOperator::Union(_) => {
                    intermediate_result = Some(vec![]);
                }
            }
        }

        Ok(intermediate_result.unwrap_or_default())
    }

    /// Execute a single expression against a DataChunk and return a ValueVector of results.
    pub fn evaluate_expression(
        _expr: &kuzu_parser::ast::Expression,
        _chunk: &DataChunk,
    ) -> Result<ValueVector, String> {
        // Placeholder: return a dummy Int64 vector
        let size = _chunk.size;
        let mut v = ValueVector::new(PhysicalTypeID::Int64, size);
        for i in 0..size {
            v.set_i64(i, 0);
        }
        v.resize(size);
        Ok(v)
    }
}

impl Default for QueryProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuzu_binder::bound_statement::BoundExpression;
    use kuzu_common::types::LogicalTypeID;
    use kuzu_parser::ast::{Constant, Expression};

    fn make_scan_op() -> LogicalOperator {
        LogicalOperator::ScanNode(kuzu_planner::logical_operator::LogicalScanNode {
            table_name: "Person".into(),
            table_id: 0,
            alias: Some("a".into()),
            columns: vec![],
        })
    }

    fn make_filter_op() -> LogicalOperator {
        LogicalOperator::Filter(kuzu_planner::logical_operator::LogicalFilter {
            expression: Expression::Constant(Constant::Bool(true)),
            children: vec![],
        })
    }

    fn make_proj_op() -> LogicalOperator {
        LogicalOperator::Projection(kuzu_planner::logical_operator::LogicalProjection {
            expressions: vec![BoundExpression {
                expression: Expression::Variable("a".into()),
                resolved_type: LogicalTypeID::Any,
                is_constant: false,
            }],
            children: vec![],
        })
    }

    fn make_limit_op() -> LogicalOperator {
        LogicalOperator::Limit(kuzu_planner::logical_operator::LogicalLimit {
            limit: 10,
            offset: 0,
            children: vec![],
        })
    }

    #[test]
    fn test_empty_plan() {
        let proc = QueryProcessor::new();
        let result = proc.execute(&[]).unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_scan_only() {
        let proc = QueryProcessor::new();
        let result = proc.execute(&[make_scan_op()]).unwrap();
        assert!(!result.is_empty());
        assert!(result[0].num_fields() > 0);
    }

    #[test]
    fn test_scan_filter_projection() {
        let proc = QueryProcessor::new();
        let plan = vec![make_scan_op(), make_filter_op(), make_proj_op()];
        let result = proc.execute(&plan).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_scan_filter_limit() {
        let proc = QueryProcessor::new();
        let plan = vec![make_scan_op(), make_filter_op(), make_limit_op()];
        let result = proc.execute(&plan).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_filter_true_passthrough() {
        let filter = PhysicalFilter {
            expression: Expression::Constant(Constant::Bool(true)),
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        for i in 0..5 {
            v.set_i64(i, i as i64);
        }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = filter.execute(input).unwrap();
        assert!(!result.is_empty());
        assert_eq!(result[0].size, 5); // All rows pass through
    }

    #[test]
    fn test_filter_false_removes_all() {
        let filter = PhysicalFilter {
            expression: Expression::Constant(Constant::Bool(false)),
        };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 5);
        for i in 0..5 {
            v.set_i64(i, i as i64);
        }
        v.resize(5);
        let input = vec![DataChunk::new(vec![v])];
        let result = filter.execute(input).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_limit() {
        let limit = PhysicalLimit { limit: 3, offset: 0 };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 10);
        for i in 0..10 {
            v.set_i64(i, i as i64);
        }
        v.resize(10);
        let input = vec![DataChunk::new(vec![v])];
        let result = limit.execute(input).unwrap();
        assert_eq!(result[0].size, 3);
    }

    #[test]
    fn test_limit_with_offset() {
        let limit = PhysicalLimit { limit: 2, offset: 5 };
        let mut v = ValueVector::new(PhysicalTypeID::Int64, 10);
        for i in 0..10 {
            v.set_i64(i, i as i64);
        }
        v.resize(10);
        let input = vec![DataChunk::new(vec![v])];
        let result = limit.execute(input).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_projection() {
        let proj = PhysicalProjection {
            column_indices: vec![0],
        };
        let mut v1 = ValueVector::new(PhysicalTypeID::Int64, 5);
        let mut v2 = ValueVector::new(PhysicalTypeID::Int64, 5);
        for i in 0..5 {
            v1.set_i64(i, i as i64);
            v2.set_i64(i, (i * 10) as i64);
        }
        v1.resize(5);
        v2.resize(5);
        let input = vec![DataChunk::new(vec![v1, v2])];
        let result = proj.execute(input).unwrap();
        assert_eq!(result[0].num_fields(), 1); // Only first column
    }
}
