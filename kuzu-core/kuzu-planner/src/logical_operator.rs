//! Logical operator types for query planning.

use kuzu_common::types::LogicalTypeID;
use kuzu_parser::ast::Expression;

/// A logical operator in the query plan.
#[derive(Debug, Clone)]
pub enum LogicalOperator {
    ScanNode(LogicalScanNode),
    ScanRel(LogicalScanRel),
    Filter(LogicalFilter),
    Projection(LogicalProjection),
    HashJoin(LogicalHashJoin),
    CrossProduct(LogicalCrossProduct),
    OrderBy(LogicalOrderBy),
    Limit(LogicalLimit),
    Aggregate(LogicalAggregate),
    Union(LogicalUnion),
}

#[derive(Debug, Clone)]
pub struct LogicalScanNode {
    pub table_name: String,
    pub table_id: u64,
    pub alias: Option<String>,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LogicalScanRel {
    pub table_name: String,
    pub table_id: u64,
    pub direction: kuzu_parser::ast::EdgeDirection,
}

#[derive(Debug, Clone)]
pub struct LogicalFilter {
    pub expression: Expression,
    pub children: Vec<LogicalOperator>,
}

#[derive(Debug, Clone)]
pub struct LogicalProjection {
    pub expressions: Vec<(Expression, Option<String>, LogicalTypeID)>,
    pub children: Vec<LogicalOperator>,
}

#[derive(Debug, Clone)]
pub struct LogicalHashJoin {
    pub join_keys: Vec<Expression>,
    pub build_side: Box<LogicalOperator>,
    pub probe_side: Box<LogicalOperator>,
}

#[derive(Debug, Clone)]
pub struct LogicalCrossProduct {
    pub left: Box<LogicalOperator>,
    pub right: Box<LogicalOperator>,
}

#[derive(Debug, Clone)]
pub struct LogicalOrderBy {
    pub sort_keys: Vec<(Expression, bool)>, // (expression, ascending)
    pub children: Vec<LogicalOperator>,
}

#[derive(Debug, Clone)]
pub struct LogicalLimit {
    pub limit: u64,
    pub offset: u64,
    pub children: Vec<LogicalOperator>,
}

#[derive(Debug, Clone)]
pub struct LogicalAggregate {
    pub group_by: Vec<Expression>,
    pub aggregates: Vec<(String, Vec<Expression>)>, // (function_name, args)
    pub children: Vec<LogicalOperator>,
}

#[derive(Debug, Clone)]
pub struct LogicalUnion {
    pub left: Box<LogicalOperator>,
    pub right: Box<LogicalOperator>,
}
