//! Logical operator types for query planning.

use kuzu_binder::bound_statement::BoundExpression;
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
    Flatten(LogicalFlatten),
    TableFunctionCall(LogicalTableFunctionCall),
    CopyFrom(LogicalCopyFrom),
    Delete(LogicalDelete),
    Set(LogicalSet),
    OptionalMatch(LogicalOptionalMatch),
    Unwind(LogicalUnwind),
}

impl LogicalOperator {
    /// Get the estimated cardinality for this operator.
    pub fn cardinality(&self) -> u64 {
        match self {
            LogicalOperator::ScanNode(s) => s.cardinality,
            LogicalOperator::ScanRel(s) => s.cardinality,
            LogicalOperator::Filter(s) => s.cardinality,
            LogicalOperator::Projection(s) => s.cardinality,
            LogicalOperator::HashJoin(s) => s.cardinality,
            LogicalOperator::CrossProduct(s) => s.cardinality,
            LogicalOperator::OrderBy(s) => s.cardinality,
            LogicalOperator::Limit(s) => s.cardinality,
            LogicalOperator::Aggregate(s) => s.cardinality,
            LogicalOperator::Union(s) => s.cardinality,
            LogicalOperator::Flatten(s) => s.cardinality,
            LogicalOperator::TableFunctionCall(s) => s.cardinality,
            LogicalOperator::CopyFrom(s) => s.cardinality,
            LogicalOperator::Delete(s) => s.cardinality,
            LogicalOperator::Set(s) => s.cardinality,
            LogicalOperator::OptionalMatch(s) => s.cardinality,
            LogicalOperator::Unwind(s) => s.cardinality,
        }
    }

    /// Set the estimated cardinality for this operator.
    pub fn set_cardinality(&mut self, card: u64) {
        match self {
            LogicalOperator::ScanNode(s) => s.cardinality = card,
            LogicalOperator::ScanRel(s) => s.cardinality = card,
            LogicalOperator::Filter(s) => s.cardinality = card,
            LogicalOperator::Projection(s) => s.cardinality = card,
            LogicalOperator::HashJoin(s) => s.cardinality = card,
            LogicalOperator::CrossProduct(s) => s.cardinality = card,
            LogicalOperator::OrderBy(s) => s.cardinality = card,
            LogicalOperator::Limit(s) => s.cardinality = card,
            LogicalOperator::Aggregate(s) => s.cardinality = card,
            LogicalOperator::Union(s) => s.cardinality = card,
            LogicalOperator::Flatten(s) => s.cardinality = card,
            LogicalOperator::TableFunctionCall(s) => s.cardinality = card,
            LogicalOperator::CopyFrom(s) => s.cardinality = card,
            LogicalOperator::Delete(s) => s.cardinality = card,
            LogicalOperator::Set(s) => s.cardinality = card,
            LogicalOperator::OptionalMatch(s) => s.cardinality = card,
            LogicalOperator::Unwind(s) => s.cardinality = card,
        }
    }

    /// Recursively visit all operators in the tree bottom-up.
    pub fn visit_bottom_up<F: FnMut(&mut LogicalOperator)>(op: &mut LogicalOperator, f: &mut F) {
        let children = op.children_mut();
        for child in children {
            Self::visit_bottom_up(child, f);
        }
        f(op);
    }

    /// Get mutable references to all direct children of this operator.
    pub fn children_mut(&mut self) -> Vec<&mut LogicalOperator> {
        match self {
            LogicalOperator::Filter(s) => s.children.iter_mut().collect(),
            LogicalOperator::Projection(s) => s.children.iter_mut().collect(),
            LogicalOperator::HashJoin(s) => vec![&mut *s.probe_side, &mut *s.build_side],
            LogicalOperator::CrossProduct(s) => vec![&mut *s.left, &mut *s.right],
            LogicalOperator::OrderBy(s) => s.children.iter_mut().collect(),
            LogicalOperator::Limit(s) => s.children.iter_mut().collect(),
            LogicalOperator::Aggregate(s) => s.children.iter_mut().collect(),
            LogicalOperator::Union(s) => vec![&mut *s.left, &mut *s.right],
            LogicalOperator::Flatten(s) => s.children.iter_mut().collect(),
            LogicalOperator::TableFunctionCall(_) => vec![],
            LogicalOperator::CopyFrom(_)
            | LogicalOperator::Delete(_)
            | LogicalOperator::Set(_)
            | LogicalOperator::OptionalMatch(_)
            | LogicalOperator::Unwind(_) => vec![],
            // Leaf operators have no children
            LogicalOperator::ScanNode(_) | LogicalOperator::ScanRel(_) => vec![],
        }
    }

    /// Get the child operators (immutable).
    pub fn children(&self) -> Vec<&LogicalOperator> {
        match self {
            LogicalOperator::Filter(s) => s.children.iter().collect(),
            LogicalOperator::Projection(s) => s.children.iter().collect(),
            LogicalOperator::HashJoin(s) => vec![&*s.probe_side, &*s.build_side],
            LogicalOperator::CrossProduct(s) => vec![&*s.left, &*s.right],
            LogicalOperator::OrderBy(s) => s.children.iter().collect(),
            LogicalOperator::Limit(s) => s.children.iter().collect(),
            LogicalOperator::Aggregate(s) => s.children.iter().collect(),
            LogicalOperator::Union(s) => vec![&*s.left, &*s.right],
            LogicalOperator::Flatten(s) => s.children.iter().collect(),
            LogicalOperator::TableFunctionCall(_) => vec![],
            LogicalOperator::CopyFrom(_)
            | LogicalOperator::Delete(_)
            | LogicalOperator::Set(_)
            | LogicalOperator::OptionalMatch(_)
            | LogicalOperator::Unwind(_) => vec![],
            LogicalOperator::ScanNode(_) | LogicalOperator::ScanRel(_) => vec![],
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogicalScanNode {
    pub table_name: String,
    pub table_id: u64,
    pub alias: Option<String>,
    pub columns: Vec<String>,
    pub cardinality: u64,
}

#[derive(Debug, Clone)]
pub struct LogicalScanRel {
    pub table_name: String,
    pub table_id: u64,
    pub direction: kuzu_parser::ast::EdgeDirection,
    pub cardinality: u64,
}

#[derive(Debug, Clone)]
pub struct LogicalFilter {
    pub expression: Expression,
    pub children: Vec<LogicalOperator>,
    pub cardinality: u64,
}

#[derive(Debug, Clone)]
pub struct LogicalProjection {
    pub expressions: Vec<BoundExpression>,
    pub children: Vec<LogicalOperator>,
    pub cardinality: u64,
}

#[derive(Debug, Clone)]
pub struct LogicalHashJoin {
    pub join_keys: Vec<Expression>,
    pub build_side: Box<LogicalOperator>,
    pub probe_side: Box<LogicalOperator>,
    pub cardinality: u64,
}

#[derive(Debug, Clone)]
pub struct LogicalCrossProduct {
    pub left: Box<LogicalOperator>,
    pub right: Box<LogicalOperator>,
    pub cardinality: u64,
}

#[derive(Debug, Clone)]
pub struct LogicalOrderBy {
    pub sort_keys: Vec<(Expression, bool)>, // (expression, ascending)
    pub children: Vec<LogicalOperator>,
    pub cardinality: u64,
}

#[derive(Debug, Clone)]
pub struct LogicalLimit {
    pub limit: u64,
    pub offset: u64,
    pub children: Vec<LogicalOperator>,
    pub cardinality: u64,
}

#[derive(Debug, Clone)]
pub struct LogicalAggregate {
    pub group_by: Vec<Expression>,
    pub aggregates: Vec<(String, Vec<Expression>)>, // (function_name, args)
    pub children: Vec<LogicalOperator>,
    pub cardinality: u64,
}

#[derive(Debug, Clone)]
pub struct LogicalUnion {
    pub left: Box<LogicalOperator>,
    pub right: Box<LogicalOperator>,
    pub cardinality: u64,
}

/// A flatten operator that converts a specific factorization group from
/// unflat (list-like) to flat (scalar) representation.
///
/// Inserted by `FactorizationRewriting` to ensure operators like HashJoin
/// receive the correct factorization layout.
#[derive(Debug, Clone)]
pub struct LogicalFlatten {
    pub group_pos: usize,
    pub children: Vec<LogicalOperator>,
    pub cardinality: u64,
}

/// UNWIND operator — expands a list expression into rows.
#[derive(Debug, Clone)]
pub struct LogicalUnwind {
    pub expression: kuzu_parser::ast::Expression,
    pub variable: String,
    pub cardinality: u64,
}

/// OPTIONAL MATCH operator — marks preceding scan as optional (produces NULLs for non-matches).
#[derive(Debug, Clone)]
pub struct LogicalOptionalMatch {
    pub cardinality: u64,
}

/// SET operator — updates properties on matched rows.
#[derive(Debug, Clone)]
pub struct LogicalSet {
    pub table_name: String,
    pub table_id: u64,
    pub column_name: String,
    pub column_idx: usize,
    pub value: kuzu_parser::ast::Expression,
    pub cardinality: u64,
}

/// DELETE operator — removes rows from a table.
#[derive(Debug, Clone)]
pub struct LogicalDelete {
    pub table_name: String,
    pub table_id: u64,
    pub primary_key_column: String,
    pub cardinality: u64,
}

/// COPY FROM operator — loads data from a file into a table.
/// This is a leaf-level DML operator (no children) that the processor
/// resolves into a `PhysicalCopyFrom` for execution.
#[derive(Debug, Clone)]
pub struct LogicalCopyFrom {
    pub table_name: String,
    pub table_id: u64,
    pub file_path: String,
    pub options: std::collections::HashMap<String, String>,
    pub cardinality: u64,
}

/// A table function call operator.
///
/// Invokes a registered table function (e.g., `duckdb_scan`, `delta_scan`)
/// and produces a DataChunk as output. The function is looked up by name
/// in the FunctionRegistry during execution.
#[derive(Debug, Clone)]
pub struct LogicalTableFunctionCall {
    pub function_name: String,
    pub args: Vec<kuzu_parser::ast::Expression>,
    pub cardinality: u64,
}
