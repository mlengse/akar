//! Logical operator types for query planning.

use akar_binder::bound_statement::BoundExpression;
use akar_catalog::CatalogColumn;
use akar_parser::ast::Expression;

/// A logical operator in the query plan.
/// A logical expressions scan operator that reads correlated variables
/// from an outer accumulate (for correlated subquery execution).
///
/// Ported from C++ `LogicalExpressionsScan`.
#[derive(Debug, Clone)]
pub struct LogicalExpressionsScan {
    /// Names of the expressions/variables to scan from the outer context.
    pub expressions: Vec<String>,
    /// Index of the outer accumulate operator in the plan (set by optimizer).
    pub outer_accumulate_idx: Option<usize>,
    /// Estimated cardinality.
    pub cardinality: u64,
}

/// A logical accumulate operator that materializes all input into memory.
///
/// Used as the build-side input for hash joins (via SIP/acc-hash-join)
/// and for correlated subquery execution. Collects all input rows into
/// an in-memory table for later probe.
///
/// Ported from C++ `LogicalAccumulate`.
#[derive(Debug, Clone)]
pub struct LogicalAccumulate {
    /// Accumulate type (Regular or Optional).
    pub accumulate_type: akar_common::enums::AccumulateType,
    /// Expressions to flatten before accumulating.
    pub flat_exprs: Vec<akar_parser::ast::Expression>,
    /// Optional mark expression (for OPTIONAL MATCH).
    pub mark: Option<akar_parser::ast::Expression>,
    /// Child operator.
    pub children: Vec<LogicalOperator>,
    /// Estimated cardinality.
    pub cardinality: u64,
}

/// Logical COUNT on a rel table — optimized via CSR metadata (Ladybug).
#[derive(Debug, Clone)]
pub struct LogicalCountRelTable {
    pub table_name: String,
    pub table_id: u64,
}

#[derive(Debug, Clone)]
pub struct LogicalPartitioner {
    pub children: Vec<LogicalOperator>,
    pub cardinality: u64,
}

#[derive(Debug, Clone)]
pub struct LogicalPathPropertyProbe {
    pub children: Vec<LogicalOperator>,
    pub cardinality: u64,
    pub node_ids_col_idx: usize,
    pub edge_ids_col_idx: Option<usize>,
    pub properties: Vec<(String, bool, Vec<String>)>,
}

#[derive(Debug, Clone)]
pub enum LogicalOperator {
    ScanNode(LogicalScanNode),
    ScanRel(LogicalScanRel),
    VectorSimilarityScan(LogicalVectorSimilarityScan),
    ArtIndexRangeScan(LogicalArtIndexRangeScan),
    Filter(LogicalFilter),
    Projection(LogicalProjection),
    HashJoin(LogicalHashJoin),
    CrossProduct(LogicalCrossProduct),
    OrderBy(LogicalOrderBy),
    Limit(LogicalLimit),
    TopK(LogicalTopK),
    Aggregate(LogicalAggregate),
    Union(LogicalUnion),
    Flatten(LogicalFlatten),
    TableFunctionCall(LogicalTableFunctionCall),
    StandaloneCall(LogicalStandaloneCall),
    CopyFrom(LogicalCopyFrom),
    BatchInsert(LogicalBatchInsert),
    IndexLookup(LogicalIndexLookup),
    Delete(LogicalDelete),
    Set(LogicalSet),
    OptionalMatch(LogicalOptionalMatch),
    OptionalExtend(LogicalOptionalExtend),
    Unwind(LogicalUnwind),
    Foreach(LogicalForeach),
    Merge(LogicalMerge),
    MergeRel(LogicalMergeRel),
    SemiJoin(LogicalSemiJoin),
    AntiJoin(LogicalAntiJoin),
    Intersect(LogicalIntersect),
    Explain(LogicalExplain),
    RecursiveExtend(LogicalRecursiveExtend),
    Accumulate(LogicalAccumulate),
    ExpressionsScan(LogicalExpressionsScan),
    CountRelTable(LogicalCountRelTable),
    Partitioner(LogicalPartitioner),
    PathPropertyProbe(LogicalPathPropertyProbe),
    // DDL operators
    CreateNodeTable(LogicalCreateNodeTable),
    CreateRelTable(LogicalCreateRelTable),
    DropTable(LogicalDropTable),
    AlterTable(LogicalAlterTable),
    CreateIndex(LogicalCreateIndex),
    DropIndex(LogicalDropIndex),
    CreateVectorIndex(LogicalCreateVectorIndex),
    CreateSequence(LogicalCreateSequence),
    DropSequence(LogicalDropSequence),
    CreateDml(LogicalCreateDml),
    CreateNode(LogicalCreateNode),
    CreateRel(LogicalCreateRel),
    Extend(LogicalExtend),
    ExportDatabase(LogicalExportDatabase),
    ImportDatabase(LogicalImportDatabase),
    CreateFtsIndex(LogicalCreateFtsIndex),
    FtsScan(LogicalFtsScan),
    EmptyResult(LogicalEmptyResult),
    MultiplicityReducer(LogicalMultiplicityReducer),
    Skip(LogicalSkip),
    Insert(LogicalInsert),
    ExtensionClause(LogicalExtensionClause),
}

impl LogicalOperator {
    /// Get the estimated cardinality for this operator.
    pub fn cardinality(&self) -> u64 {
        match self {
            LogicalOperator::ScanNode(s) => s.cardinality,
            LogicalOperator::ScanRel(s) => s.cardinality,
            LogicalOperator::VectorSimilarityScan(s) => s.cardinality,
            LogicalOperator::ArtIndexRangeScan(s) => s.cardinality,
            LogicalOperator::Filter(s) => s.cardinality,
            LogicalOperator::Projection(s) => s.cardinality,
            LogicalOperator::HashJoin(s) => s.cardinality,
            LogicalOperator::CrossProduct(s) => s.cardinality,
            LogicalOperator::OrderBy(s) => s.cardinality,
            LogicalOperator::TopK(s) => s.cardinality,
            LogicalOperator::Limit(s) => s.cardinality,
            LogicalOperator::Aggregate(s) => s.cardinality,
            LogicalOperator::Union(s) => s.cardinality,
            LogicalOperator::Flatten(s) => s.cardinality,
            LogicalOperator::TableFunctionCall(s) => s.cardinality,
            LogicalOperator::StandaloneCall(s) => s.cardinality,
            LogicalOperator::CopyFrom(s) => s.cardinality,
            LogicalOperator::BatchInsert(s) => s.cardinality,
            LogicalOperator::IndexLookup(s) => s.cardinality,
            LogicalOperator::Delete(s) => s.cardinality,
            LogicalOperator::Set(s) => s.cardinality,
            LogicalOperator::OptionalMatch(s) => s.cardinality,
            LogicalOperator::OptionalExtend(s) => s.cardinality,
            LogicalOperator::Unwind(s) => s.cardinality,
            LogicalOperator::Foreach(s) => s.cardinality,
            LogicalOperator::Merge(s) => s.cardinality,
            LogicalOperator::MergeRel(s) => s.cardinality,
            LogicalOperator::SemiJoin(s) => s.cardinality,
            LogicalOperator::AntiJoin(s) => s.cardinality,
            LogicalOperator::Intersect(s) => s.cardinality,
            LogicalOperator::Explain(s) => s.cardinality,
            LogicalOperator::RecursiveExtend(s) => s.cardinality,
            LogicalOperator::Accumulate(s) => s.cardinality,
            LogicalOperator::ExpressionsScan(s) => s.cardinality,
            LogicalOperator::CountRelTable(_) => 1,
            LogicalOperator::Partitioner(s) => s.cardinality,
            LogicalOperator::PathPropertyProbe(s) => s.cardinality,
            // DDL operators
            LogicalOperator::CreateNodeTable(s) => s.cardinality,
            LogicalOperator::CreateRelTable(s) => s.cardinality,
            LogicalOperator::DropTable(s) => s.cardinality,
            LogicalOperator::AlterTable(s) => s.cardinality,
            LogicalOperator::CreateIndex(s) => s.cardinality,
            LogicalOperator::DropIndex(s) => s.cardinality,
            LogicalOperator::CreateVectorIndex(s) => s.cardinality,
            LogicalOperator::CreateSequence(s) => s.cardinality,
            LogicalOperator::DropSequence(s) => s.cardinality,
            LogicalOperator::CreateDml(s) => s.cardinality,
            LogicalOperator::CreateNode(s) => s.cardinality,
            LogicalOperator::CreateRel(s) => s.cardinality,
            LogicalOperator::Extend(s) => s.cardinality,
            LogicalOperator::ExportDatabase(s) => s.cardinality,
            LogicalOperator::ImportDatabase(s) => s.cardinality,
            LogicalOperator::CreateFtsIndex(s) => s.cardinality,
            LogicalOperator::FtsScan(s) => s.cardinality,
            LogicalOperator::EmptyResult(s) => s.cardinality,
            LogicalOperator::MultiplicityReducer(s) => s.cardinality,
            LogicalOperator::Skip(s) => s.cardinality,
            LogicalOperator::Insert(s) => s.cardinality,
            LogicalOperator::ExtensionClause(s) => s.cardinality,
        }
    }

    /// Set the estimated cardinality for this operator.
    pub fn set_cardinality(&mut self, card: u64) {
        match self {
            LogicalOperator::ScanNode(s) => s.cardinality = card,
            LogicalOperator::ScanRel(s) => s.cardinality = card,
            LogicalOperator::VectorSimilarityScan(s) => s.cardinality = card,
            LogicalOperator::ArtIndexRangeScan(s) => s.cardinality = card,
            LogicalOperator::Filter(s) => s.cardinality = card,
            LogicalOperator::Projection(s) => s.cardinality = card,
            LogicalOperator::HashJoin(s) => s.cardinality = card,
            LogicalOperator::CrossProduct(s) => s.cardinality = card,
            LogicalOperator::OrderBy(s) => s.cardinality = card,
            LogicalOperator::TopK(s) => s.cardinality = card,
            LogicalOperator::Limit(s) => s.cardinality = card,
            LogicalOperator::Aggregate(s) => s.cardinality = card,
            LogicalOperator::Union(s) => s.cardinality = card,
            LogicalOperator::Flatten(s) => s.cardinality = card,
            LogicalOperator::TableFunctionCall(s) => s.cardinality = card,
            LogicalOperator::StandaloneCall(s) => s.cardinality = card,
            LogicalOperator::CopyFrom(s) => s.cardinality = card,
            LogicalOperator::BatchInsert(s) => s.cardinality = card,
            LogicalOperator::IndexLookup(s) => s.cardinality = card,
            LogicalOperator::Delete(s) => s.cardinality = card,
            LogicalOperator::Set(s) => s.cardinality = card,
            LogicalOperator::OptionalMatch(s) => s.cardinality = card,
            LogicalOperator::OptionalExtend(s) => s.cardinality = card,
            LogicalOperator::Unwind(s) => s.cardinality = card,
            LogicalOperator::Foreach(s) => s.cardinality = card,
            LogicalOperator::Merge(s) => s.cardinality = card,
            LogicalOperator::MergeRel(s) => s.cardinality = card,
            LogicalOperator::SemiJoin(s) => s.cardinality = card,
            LogicalOperator::AntiJoin(s) => s.cardinality = card,
            LogicalOperator::Intersect(s) => s.cardinality = card,
            LogicalOperator::Explain(s) => s.cardinality = card,
            LogicalOperator::RecursiveExtend(s) => s.cardinality = card,
            LogicalOperator::Accumulate(s) => s.cardinality = card,
            LogicalOperator::ExpressionsScan(s) => s.cardinality = card,
            LogicalOperator::CountRelTable(_) => {}
            LogicalOperator::Partitioner(s) => s.cardinality = card,
            LogicalOperator::PathPropertyProbe(s) => s.cardinality = card,
            // DDL operators
            LogicalOperator::CreateNodeTable(s) => s.cardinality = card,
            LogicalOperator::CreateRelTable(s) => s.cardinality = card,
            LogicalOperator::DropTable(s) => s.cardinality = card,
            LogicalOperator::AlterTable(s) => s.cardinality = card,
            LogicalOperator::CreateIndex(s) => s.cardinality = card,
            LogicalOperator::DropIndex(s) => s.cardinality = card,
            LogicalOperator::CreateVectorIndex(s) => s.cardinality = card,
            LogicalOperator::CreateSequence(s) => s.cardinality = card,
            LogicalOperator::DropSequence(s) => s.cardinality = card,
            LogicalOperator::CreateDml(s) => s.cardinality = card,
            LogicalOperator::CreateNode(s) => s.cardinality = card,
            LogicalOperator::CreateRel(s) => s.cardinality = card,
            LogicalOperator::Extend(s) => s.cardinality = card,
            LogicalOperator::ExportDatabase(s) => s.cardinality = card,
            LogicalOperator::ImportDatabase(s) => s.cardinality = card,
            LogicalOperator::CreateFtsIndex(s) => s.cardinality = card,
            LogicalOperator::FtsScan(s) => s.cardinality = card,
            LogicalOperator::EmptyResult(s) => s.cardinality = card,
            LogicalOperator::MultiplicityReducer(s) => s.cardinality = card,
            LogicalOperator::Skip(s) => s.cardinality = card,
            LogicalOperator::Insert(s) => s.cardinality = card,
            LogicalOperator::ExtensionClause(s) => s.cardinality = card,
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
            LogicalOperator::TopK(s) => s.children.iter_mut().collect(),
            LogicalOperator::Limit(s) => s.children.iter_mut().collect(),
            LogicalOperator::Aggregate(s) => s.children.iter_mut().collect(),
            LogicalOperator::Union(s) => vec![&mut *s.left, &mut *s.right],
            LogicalOperator::Flatten(s) => s.children.iter_mut().collect(),
            LogicalOperator::OptionalMatch(s) => vec![&mut *s.left, &mut *s.right],
            LogicalOperator::OptionalExtend(s) => s.children.iter_mut().collect(),
            LogicalOperator::SemiJoin(s) => vec![&mut *s.left, &mut *s.right],
            LogicalOperator::AntiJoin(s) => vec![&mut *s.left, &mut *s.right],
            LogicalOperator::Intersect(s) => vec![&mut *s.left, &mut *s.right],
            LogicalOperator::Explain(s) => vec![&mut *s.inner],
            LogicalOperator::RecursiveExtend(_) => vec![],
            LogicalOperator::Accumulate(s) => s.children.iter_mut().collect(),
            LogicalOperator::Partitioner(s) => s.children.iter_mut().collect(),
            LogicalOperator::PathPropertyProbe(s) => s.children.iter_mut().collect(),
            LogicalOperator::CountRelTable(_) => vec![],
            LogicalOperator::ExpressionsScan(_) => vec![],
            LogicalOperator::TableFunctionCall(_) => vec![],
            LogicalOperator::StandaloneCall(_) => vec![],
            LogicalOperator::CopyFrom(_)
            | LogicalOperator::BatchInsert(_)
            | LogicalOperator::IndexLookup(_)
            | LogicalOperator::Delete(_)
            | LogicalOperator::Set(_)
            | LogicalOperator::Unwind(_)
            | LogicalOperator::Foreach(_)
            | LogicalOperator::Merge(_)
            | LogicalOperator::MergeRel(_) => vec![],
            // Leaf operators have no children
            LogicalOperator::ArtIndexRangeScan(_)
            | LogicalOperator::VectorSimilarityScan(_)
            | LogicalOperator::ScanNode(_)
            | LogicalOperator::ScanRel(_)
            | LogicalOperator::CreateNodeTable(_)
            | LogicalOperator::CreateRelTable(_)
            | LogicalOperator::DropTable(_)
            | LogicalOperator::AlterTable(_)
            | LogicalOperator::CreateIndex(_)
            | LogicalOperator::DropIndex(_)
            | LogicalOperator::CreateVectorIndex(_)
            | LogicalOperator::CreateSequence(_)
            | LogicalOperator::DropSequence(_)
            | LogicalOperator::CreateDml(_)
            | LogicalOperator::CreateNode(_)
            | LogicalOperator::CreateRel(_)
            | LogicalOperator::Extend(_)
            | LogicalOperator::ExportDatabase(_)
            | LogicalOperator::ImportDatabase(_)
            | LogicalOperator::CreateFtsIndex(_)
            | LogicalOperator::FtsScan(_)
            | LogicalOperator::EmptyResult(_)
            | LogicalOperator::Insert(_)
            | LogicalOperator::ExtensionClause(_) => vec![],
            LogicalOperator::MultiplicityReducer(s) => s.children.iter_mut().collect(),
            LogicalOperator::Skip(s) => s.children.iter_mut().collect(),
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
            LogicalOperator::TopK(s) => s.children.iter().collect(),
            LogicalOperator::Limit(s) => s.children.iter().collect(),
            LogicalOperator::Aggregate(s) => s.children.iter().collect(),
            LogicalOperator::Union(s) => vec![&*s.left, &*s.right],
            LogicalOperator::Flatten(s) => s.children.iter().collect(),
            LogicalOperator::OptionalMatch(s) => vec![&*s.left, &*s.right],
            LogicalOperator::OptionalExtend(s) => s.children.iter().collect(),
            LogicalOperator::SemiJoin(s) => vec![&*s.left, &*s.right],
            LogicalOperator::AntiJoin(s) => vec![&*s.left, &*s.right],
            LogicalOperator::Intersect(s) => vec![&*s.left, &*s.right],
            LogicalOperator::Explain(s) => vec![&*s.inner],
            LogicalOperator::RecursiveExtend(_) => vec![],
            LogicalOperator::Accumulate(s) => s.children.iter().collect(),
            LogicalOperator::Partitioner(s) => s.children.iter().collect(),
            LogicalOperator::PathPropertyProbe(s) => s.children.iter().collect(),
            LogicalOperator::CountRelTable(_) => vec![],
            LogicalOperator::ExpressionsScan(_) => vec![],
            LogicalOperator::TableFunctionCall(_) => vec![],
            LogicalOperator::StandaloneCall(_) => vec![],
            LogicalOperator::CopyFrom(_)
            | LogicalOperator::BatchInsert(_)
            | LogicalOperator::IndexLookup(_)
            | LogicalOperator::Delete(_)
            | LogicalOperator::Set(_)
            | LogicalOperator::Unwind(_)
            | LogicalOperator::Foreach(_)
            | LogicalOperator::Merge(_)
            | LogicalOperator::MergeRel(_) => vec![],
            LogicalOperator::ArtIndexRangeScan(_)
            | LogicalOperator::VectorSimilarityScan(_)
            | LogicalOperator::ScanNode(_)
            | LogicalOperator::ScanRel(_)
            | LogicalOperator::CreateNodeTable(_)
            | LogicalOperator::CreateRelTable(_)
            | LogicalOperator::DropTable(_)
            | LogicalOperator::AlterTable(_)
            | LogicalOperator::CreateIndex(_)
            | LogicalOperator::DropIndex(_)
            | LogicalOperator::CreateVectorIndex(_)
            | LogicalOperator::CreateSequence(_)
            | LogicalOperator::DropSequence(_)
            | LogicalOperator::CreateDml(_)
            | LogicalOperator::CreateNode(_)
            | LogicalOperator::CreateRel(_)
            | LogicalOperator::Extend(_)
            | LogicalOperator::ExportDatabase(_)
            | LogicalOperator::ImportDatabase(_)
            | LogicalOperator::CreateFtsIndex(_)
            | LogicalOperator::FtsScan(_)
            | LogicalOperator::EmptyResult(_)
            | LogicalOperator::Insert(_)
            | LogicalOperator::ExtensionClause(_) => vec![],
            LogicalOperator::MultiplicityReducer(s) => s.children.iter().collect(),
            LogicalOperator::Skip(s) => s.children.iter().collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogicalArtIndexRangeScan {
    pub table_name: String,
    pub table_id: u64,
    pub alias: Option<String>,
    pub lower_bound: Option<akar_common::types::Value>,
    pub upper_bound: Option<akar_common::types::Value>,
    pub lower_inclusive: bool,
    pub upper_inclusive: bool,
    pub cardinality: u64,
}

#[derive(Debug, Clone)]
pub struct LogicalVectorSimilarityScan {
    pub table_name: String,
    pub column_name: String,
    pub query_vector: Vec<f64>,
    pub top_k: u64,
    pub cardinality: u64,
}

#[derive(Debug, Clone)]
pub struct LogicalScanNode {
    pub table_name: String,
    pub table_id: u64,
    pub alias: Option<String>,
    pub columns: Vec<String>,
    pub cardinality: u64,
    pub fts_query: Option<LogicalFtsScan>,
    pub predicate: Option<Expression>,
}

#[derive(Debug, Clone)]
pub struct LogicalScanRel {
    pub table_name: String,
    pub table_id: u64,
    pub direction: akar_parser::ast::EdgeDirection,
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
    /// Whether this join is eligible for foreign join push-down optimization.
    /// Set by the ForeignJoinPushDown optimizer pass when all tables in the
    /// pattern belong to the same foreign database.
    pub push_down_eligible: bool,
}

/// Semi-join: returns left rows that have a matching key in the right side.
/// Like HashJoin but only emits left columns for matching rows.
#[derive(Debug, Clone)]
pub struct LogicalSemiJoin {
    pub join_keys: Vec<Expression>,
    pub left: Box<LogicalOperator>,
    pub right: Box<LogicalOperator>,
    pub cardinality: u64,
}

/// Anti-join: returns left rows that have NO matching key in the right side.
/// Like SemiJoin but inverts the match condition.
#[derive(Debug, Clone)]
pub struct LogicalAntiJoin {
    pub join_keys: Vec<Expression>,
    pub left: Box<LogicalOperator>,
    pub right: Box<LogicalOperator>,
    pub cardinality: u64,
}

/// EXPLAIN operator — wraps a child plan and produces a textual plan description.
///
/// Unlike other operators, Explain does not execute its child; instead it
/// serializes the operator tree to a human-readable string.
#[derive(Debug, Clone)]
pub struct LogicalExplain {
    /// The inner operator tree to explain.
    pub inner: Box<LogicalOperator>,
    /// The type of explain output.
    pub explain_type: akar_parser::ast::ExplainType,
    /// Cardinality (always 1 — one row with the plan string).
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

/// A fused ORDER BY + LIMIT operator for Top-K optimization.
///
/// When the optimizer detects a consecutive ORDER BY followed by LIMIT,
/// it fuses them into a single LogicalTopK. This signals the processor
/// to use a BinaryHeap-based TopK execution (O(n log k)) instead of
/// full sort + limit (O(n log n)).
#[derive(Debug, Clone)]
pub struct LogicalTopK {
    pub sort_keys: Vec<(Expression, bool)>,
    pub limit: u64,
    pub offset: u64,
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
    pub all: bool,
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
    pub expression: akar_parser::ast::Expression,
    pub variable: String,
    pub cardinality: u64,
}

/// INTERSECT operator — finds common keys across multiple build sides.
///
/// Used for multi-pattern matching like `MATCH (a)-[:r1]->(b), (a)-[:r2]->(c)`
/// where `a` is the shared key. Intersect probes multiple build hash tables
/// and outputs combined payloads only for keys present in all build sides.
#[derive(Debug, Clone)]
pub struct LogicalIntersect {
    /// Number of build sides (hash tables to probe).
    pub num_build_sides: u32,
    /// Key expressions for each build side.
    pub build_key_exprs: Vec<Expression>,
    /// Probe side — produces the shared key value to look up.
    pub left: Box<LogicalOperator>,
    /// Build side — produces hash tables for each pattern.
    pub right: Box<LogicalOperator>,
    /// Estimated cardinality.
    pub cardinality: u64,
}

/// Variable-length path (recursive extend) operator.
///
/// Corresponds to `MATCH (a)-[e*1..3]->(b)` — traverses the graph
/// up to `upper_bound` hops from source nodes and produces results
/// for each path whose length is between `lower_bound` and `upper_bound`.
///
/// Supports both unweighted BFS and weighted shortest path (Dijkstra)
/// when `weight_property` is specified.
///
/// This is a leaf operator that executes BFS/Dijkstra traversal during query execution.
#[derive(Debug, Clone)]
pub struct LogicalRecursiveExtend {
    /// Source node variable name.
    pub source_var: String,
    /// Source node table ID.
    pub source_table_id: u64,
    /// Edge variable name (optional).
    pub edge_var: Option<String>,
    /// Destination node variable name.
    pub target_var: String,
    /// Relationship table ID(s) to traverse.
    pub rel_table_ids: Vec<u64>,
    /// Relationship label(s).
    pub rel_labels: Vec<String>,
    /// Minimum path length.
    pub lower_bound: u64,
    /// Maximum path length.
    pub upper_bound: u64,
    /// Traversal direction.
    pub direction: akar_common::enums::ExtendDirection,
    /// Path semantic (WALK / TRAIL / ACYCLIC).
    pub semantic: akar_common::enums::PathSemantic,
    /// Optional edge weight property name for weighted shortest path.
    /// When `Some(prop_name)`, traversal uses Dijkstra's algorithm instead of BFS,
    /// and results are sorted by cumulative path cost.
    pub weight_property: Option<String>,
    /// Optional name for the cost output column (e.g., "cost" or "totalWeight").
    /// Only used when `weight_property` is set.
    pub cost_output_name: Option<String>,
    /// Estimated cardinality.
    pub cardinality: u64,
}

/// OPTIONAL MATCH operator — a tree node with required (left) and optional (right) children.
///
/// The required side is executed first. For each resulting row, the optional
/// side is attempted. If the optional side produces a match, the combined
/// row (left + right columns) is emitted. If no match, left columns are
/// emitted with NULLs for right-side columns.
#[derive(Debug, Clone)]
pub struct LogicalOptionalMatch {
    pub left: Box<LogicalOperator>,
    pub right: Box<LogicalOperator>,
    pub cardinality: u64,
}

/// OPTIONAL MATCH over an already-bound pair of node variables.
///
/// Used when both endpoint node variables of the optional pattern are bound by
/// the required side (e.g. `OPTIONAL MATCH (a)-[existing:Connected]-(b)` where
/// `a` and `b` are already in scope). Unlike `LogicalOptionalMatch`, the right
/// side is not a scan: the pattern is probed per input row against the
/// relationship table adjacency (forward and/or reverse), emitting the edge
/// property columns, or NULL-padding them when no edge exists (P53.25).
#[derive(Debug, Clone)]
pub struct LogicalOptionalExtend {
    pub children: Vec<LogicalOperator>,
    /// Name of the relationship table to probe.
    pub rel_table_name: String,
    /// ID of the relationship table.
    pub rel_table_id: u64,
    /// Variable name of the relationship (e.g., "existing"); prefix for the
    /// emitted edge property columns.
    pub rel_var: String,
    /// Variable name of the bound source node (e.g., "a").
    pub src_node_var: String,
    /// Variable name of the bound destination node (e.g., "b"). An empty
    /// string selects fan-out mode (P53.40): the destination is anonymous
    /// (`OPTIONAL MATCH (m)-[r:R]-(:T)`), so every edge incident on the source
    /// becomes one output row.
    pub dst_node_var: String,
    /// Direction of the probe (forward, backward, or both).
    pub direction: akar_parser::ast::EdgeDirection,
    /// Estimated cardinality.
    pub cardinality: u64,
}

/// SET operator — updates properties on matched rows.
#[derive(Debug, Clone)]
pub struct LogicalSet {
    pub table_name: String,
    pub table_id: u64,
    pub is_node: bool,
    /// All assignments of the SET clause, evaluated against the pre-update
    /// snapshot as a group (P53.17). Multiple items were previously emitted as
    /// a chain of single-item operators, so items after the first received the
    /// previous item's count chunk and lost the scan rows.
    pub items: Vec<SetItem>,
    /// True when the SET is the terminal clause (no trailing RETURN): the
    /// operator reports "N rows updated" as column 0. False when a RETURN
    /// follows — an empty match must flow 0 rows, not a phantom count row
    /// (P53.39, kairos Finding P59.1).
    pub emit_count: bool,
    pub cardinality: u64,
}

/// A single `SET n.prop = <expr>` assignment inside a [`LogicalSet`].
#[derive(Debug, Clone)]
pub struct SetItem {
    pub column_name: String,
    pub column_idx: usize,
    pub value: akar_parser::ast::Expression,
}

/// DELETE operator — removes rows from a table.
#[derive(Debug, Clone)]
pub struct LogicalDelete {
    pub table_name: String,
    pub table_id: u64,
    pub primary_key_column: String,
    pub is_node: bool,
    pub detach: bool,
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

/// Batch insert operator — inserts multiple rows/rels in a single operation.
///
/// Unlike `CopyFrom` which reads from a file, `BatchInsert` takes pre-collected
/// data from the plan pipeline (e.g., multiple fused CREATE statements).
/// Uses `NodeTable::insert_rows_batch()` / `RelTable::insert_rels_batch()`.
#[derive(Debug, Clone)]
pub struct LogicalBatchInsert {
    pub table_name: String,
    pub table_id: u64,
    /// Rows to insert: each row is a Vec<Value> matching column order.
    pub rows: Vec<Vec<akar_common::types::Value>>,
    pub cardinality: u64,
}

/// Index lookup operator — point lookup via ART index on a PK column.
#[derive(Debug, Clone)]
pub struct LogicalIndexLookup {
    pub table_name: String,
    pub table_id: u64,
    pub key_value: akar_common::types::Value,
    pub cardinality: u64,
}

/// FOREACH operator — iterates over list elements and executes sub-plans.
#[derive(Debug, Clone)]
pub struct LogicalForeach {
    pub variable: String,
    pub expression: akar_parser::ast::Expression,
    /// Sub-plans to execute for each list element.
    pub sub_plans: Vec<Vec<LogicalOperator>>,
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
    pub args: Vec<akar_parser::ast::Expression>,
    pub cardinality: u64,
}

#[derive(Debug, Clone)]
pub struct LogicalStandaloneCall {
    pub function_name: String,
    pub args: Vec<akar_parser::ast::Expression>,
    pub cardinality: u64,
}

/// MERGE operator — match or create a node/pattern.
///
/// The processor first attempts to match a node with the given properties.
/// If found, applies `ON MATCH SET` operations. If not found, creates a
/// new node with the pattern properties and applies `ON CREATE SET`.
#[derive(Debug, Clone)]
pub struct LogicalMerge {
    pub table_name: String,
    pub table_id: u64,
    /// Properties from the MERGE pattern (name, expression pairs).
    pub properties: Vec<(String, akar_parser::ast::Expression)>,
    /// SET operations to apply when the node already exists (matched).
    pub on_match: Vec<LogicalSet>,
    /// SET operations to apply when a new node is created.
    pub on_create: Vec<LogicalSet>,
    pub cardinality: u64,
}

/// Logical operator for edge MERGE (P53.20): `MERGE (a)-[r:R]->(b)`.
///
/// Matches an existing edge from `src` to `dst` on the rel table whose props
/// equal the pattern properties; if absent, inserts a new edge. Emits the
/// matched/inserted edge's `_id` as `<edge_var>._id` so a following SET clause
/// can target it.
#[derive(Debug, Clone)]
pub struct LogicalMergeRel {
    pub rel_table_name: String,
    pub rel_table_id: u64,
    /// Variable bound to the edge (e.g. `r`).
    pub edge_var: String,
    /// Node variables bound by a prior MATCH that anchor the endpoints.
    pub src_node_var: String,
    pub dst_node_var: String,
    /// Inline properties from the edge pattern (`{type: $type}`).
    pub properties: Vec<(String, akar_parser::ast::Expression)>,
    /// SET operations applied when the edge already exists (empty for the
    /// standalone-SET form used by Kairos `add_connection`).
    pub on_match: Vec<LogicalSet>,
    /// SET operations applied when a new edge is created.
    pub on_create: Vec<LogicalSet>,
    pub cardinality: u64,
}

// ==================== DDL Operators ====================

/// Logical operator for CREATE NODE TABLE.
#[derive(Debug, Clone)]
pub struct LogicalCreateNodeTable {
    pub name: String,
    pub columns: Vec<CatalogColumn>,
    pub primary_key: String,
    pub cardinality: u64,
}

/// Logical operator for CREATE REL TABLE.
#[derive(Debug, Clone)]
pub struct LogicalCreateRelTable {
    pub name: String,
    pub from: String,
    pub to: String,
    pub columns: Vec<CatalogColumn>,
    pub cardinality: u64,
}

/// Logical operator for DROP TABLE.
#[derive(Debug, Clone)]
pub struct LogicalDropTable {
    pub name: String,
    pub cardinality: u64,
}

/// Logical operator for ALTER TABLE.
#[derive(Debug, Clone)]
pub struct LogicalAlterTable {
    pub table_name: String,
    pub action: akar_parser::ast::AlterAction,
    pub cardinality: u64,
}

/// Logical operator for CREATE [ART|HASH] INDEX.
#[derive(Debug, Clone)]
pub struct LogicalCreateIndex {
    pub index_type: akar_catalog::IndexType,
    pub index_name: String,
    pub table_name: String,
    pub column_name: String,
    pub cardinality: u64,
}

/// Logical operator for DROP INDEX.
#[derive(Debug, Clone)]
pub struct LogicalDropIndex {
    pub index_name: String,
    pub table_name: String,
    pub cardinality: u64,
}

/// Logical operator for CREATE VECTOR INDEX.
#[derive(Debug, Clone)]
pub struct LogicalCreateVectorIndex {
    pub index_name: String,
    pub table_name: String,
    pub column_name: String,
    pub metric: String,
    pub dimensions: u64,
    pub cardinality: u64,
}

/// Logical operator for CREATE SEQUENCE.
#[derive(Debug, Clone)]
pub struct LogicalCreateSequence {
    pub name: String,
    pub if_not_exists: bool,
    pub or_replace: bool,
    pub start_with: i64,
    pub increment: i64,
    pub min_value: i64,
    pub max_value: i64,
    pub cycle: bool,
    pub cardinality: u64,
}

/// Logical operator for DROP SEQUENCE.
#[derive(Debug, Clone)]
pub struct LogicalDropSequence {
    pub name: String,
    pub if_exists: bool,
    pub cardinality: u64,
}

/// Logical operator for CREATE DML (node creation with properties).
#[derive(Debug, Clone)]
pub struct LogicalCreateDml {
    pub table_name: String,
    pub table_id: u64,
    pub properties: Vec<(String, akar_parser::ast::Expression)>,
    pub cardinality: u64,
}

#[derive(Debug, Clone)]
pub struct LogicalCreateNode {
    pub table_name: String,
    pub table_id: u64,
    pub out_var_name: String,
    pub properties: Vec<(String, akar_parser::ast::Expression)>,
    pub cardinality: u64,
}

/// Logical operator for extending from a source node through a relationship.
///
/// Replaces the combination of ScanRel + ScanNode(dest) in the pipeline.
/// For each source node, looks up adjacency list entries in the rel table,
/// producing output rows that include source fields, rel properties, and
/// destination node properties.
///
/// Ported from C++ `LogicalExtend`.
#[derive(Debug, Clone)]
pub struct LogicalExtend {
    /// Name of the relationship table to extend through.
    pub rel_table_name: String,
    /// ID of the relationship table.
    pub rel_table_id: u64,
    /// Variable name of the relationship (e.g., "r" in `-[r:RELATES_TO]->`).
    /// Used as the field-name prefix for relationship properties.
    pub rel_var: String,
    /// Variable name of the bound (source) node.
    pub bound_node_var: String,
    /// Direction of the extend (forward, backward, or both).
    pub direction: akar_parser::ast::EdgeDirection,
    /// Variable name of the destination node (e.g., "p").
    pub dst_node_var: String,
    /// Table name of the destination node (e.g., "Post").
    pub dst_table_name: String,
    /// Table ID of the destination node.
    pub dst_table_id: u64,
    /// Estimated cardinality.
    pub cardinality: u64,
}

#[derive(Debug, Clone)]
pub struct LogicalCreateRel {
    pub table_name: String,
    pub table_id: u64,
    pub src_node_name: String,
    pub dst_node_name: String,
    pub properties: Vec<(String, akar_parser::ast::Expression)>,
    pub cardinality: u64,
}

/// Logical operator for EXPORT DATABASE.
#[derive(Debug, Clone)]
pub struct LogicalExportDatabase {
    pub file_path: String,
    pub file_type: String,
    pub schema_only: bool,
    pub options: std::collections::HashMap<String, String>,
    pub cardinality: u64,
}

/// Logical operator for IMPORT DATABASE.
#[derive(Debug, Clone)]
pub struct LogicalImportDatabase {
    pub file_path: String,
    pub query: String,
    pub index_query: String,
    pub cardinality: u64,
}

/// Logical operator for creating an FTS index (P8 architecture)
#[derive(Debug, Clone)]
pub struct LogicalCreateFtsIndex {
    pub index_name: String,
    pub table_name: String,
    pub column_name: String,
    pub if_not_exists: bool,
    /// Derived macro table names.
    pub docs_table: String,
    pub terms_table: String,
    pub posting_table: String,
    pub cardinality: u64,
}

/// Logical operator for querying an FTS index via `USING FTS INDEX` clause.
#[derive(Debug, Clone)]
pub struct LogicalFtsScan {
    pub index_name: String,
    pub query_string: String,
    pub docs_table: String,
    pub terms_table: String,
    pub posting_table: String,
    /// Source node table/column the index was created on (P52.39).
    pub table_name: String,
    pub column_name: String,
    pub cardinality: u64,
}

/// EmptyResult operator — returns an empty result set (0 rows).
/// Inserted when planner knows the query will yield no rows (e.g. WHERE false).
#[derive(Debug, Clone)]
pub struct LogicalEmptyResult {
    pub cardinality: u64,
}

/// MultiplicityReducer operator — deduplicates rows from pattern matching fan-out.
#[derive(Debug, Clone)]
pub struct LogicalMultiplicityReducer {
    pub key_columns: Vec<usize>,
    pub children: Vec<LogicalOperator>,
    pub cardinality: u64,
}

/// Skip operator — skips the first N rows (like LIMIT offset without limit).
#[derive(Debug, Clone)]
pub struct LogicalSkip {
    pub offset: u64,
    pub children: Vec<LogicalOperator>,
    pub cardinality: u64,
}

/// Insert operator — row-level insertion (unlike BatchInsert).
#[derive(Debug, Clone)]
pub struct LogicalInsert {
    pub table_name: String,
    pub table_id: u64,
    pub columns: Vec<String>,
    pub values: Vec<Vec<akar_common::types::Value>>,
    pub cardinality: u64,
}

/// ExtensionClause operator — handles EXTENSION commands (INSTALL, LOAD).
#[derive(Debug, Clone)]
pub struct LogicalExtensionClause {
    pub action: akar_parser::ast::ExtensionAction,
    pub extension_name: String,
    pub cardinality: u64,
}
