//! Logical operator types for query planning.

use kuzu_binder::bound_statement::BoundExpression;
use kuzu_catalog::CatalogColumn;
use kuzu_parser::ast::Expression;

/// A logical operator in the query plan.
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
    Aggregate(LogicalAggregate),
    Union(LogicalUnion),
    Flatten(LogicalFlatten),
    TableFunctionCall(LogicalTableFunctionCall),
    CopyFrom(LogicalCopyFrom),
    Delete(LogicalDelete),
    Set(LogicalSet),
    OptionalMatch(LogicalOptionalMatch),
    Unwind(LogicalUnwind),
    Foreach(LogicalForeach),
    Merge(LogicalMerge),
    SemiJoin(LogicalSemiJoin),
    AntiJoin(LogicalAntiJoin),
    Intersect(LogicalIntersect),
    Explain(LogicalExplain),
    RecursiveExtend(LogicalRecursiveExtend),
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
    ExportDatabase(LogicalExportDatabase),
    ImportDatabase(LogicalImportDatabase),
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
            LogicalOperator::Foreach(s) => s.cardinality,
            LogicalOperator::Merge(s) => s.cardinality,
            LogicalOperator::SemiJoin(s) => s.cardinality,
            LogicalOperator::AntiJoin(s) => s.cardinality,
            LogicalOperator::Intersect(s) => s.cardinality,
            LogicalOperator::Explain(s) => s.cardinality,
            LogicalOperator::RecursiveExtend(s) => s.cardinality,
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
            LogicalOperator::ExportDatabase(s) => s.cardinality,
            LogicalOperator::ImportDatabase(s) => s.cardinality,
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
            LogicalOperator::Foreach(s) => s.cardinality = card,
            LogicalOperator::Merge(s) => s.cardinality = card,
            LogicalOperator::SemiJoin(s) => s.cardinality = card,
            LogicalOperator::AntiJoin(s) => s.cardinality = card,
            LogicalOperator::Intersect(s) => s.cardinality = card,
            LogicalOperator::Explain(s) => s.cardinality = card,
            LogicalOperator::RecursiveExtend(s) => s.cardinality = card,
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
            LogicalOperator::ExportDatabase(s) => s.cardinality = card,
            LogicalOperator::ImportDatabase(s) => s.cardinality = card,
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
            LogicalOperator::OptionalMatch(s) => vec![&mut *s.left, &mut *s.right],
            LogicalOperator::SemiJoin(s) => vec![&mut *s.left, &mut *s.right],
            LogicalOperator::AntiJoin(s) => vec![&mut *s.left, &mut *s.right],
            LogicalOperator::Intersect(s) => vec![&mut *s.left, &mut *s.right],
            LogicalOperator::Explain(s) => vec![&mut *s.inner],
            LogicalOperator::RecursiveExtend(_) => vec![],
            LogicalOperator::TableFunctionCall(_) => vec![],
            LogicalOperator::CopyFrom(_)
            | LogicalOperator::Delete(_)
            | LogicalOperator::Set(_)
            | LogicalOperator::Unwind(_)
            | LogicalOperator::Foreach(_)
            | LogicalOperator::Merge(_) => vec![],
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
            | LogicalOperator::ExportDatabase(_)
            | LogicalOperator::ImportDatabase(_) => vec![],
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
            LogicalOperator::OptionalMatch(s) => vec![&*s.left, &*s.right],
            LogicalOperator::SemiJoin(s) => vec![&*s.left, &*s.right],
            LogicalOperator::AntiJoin(s) => vec![&*s.left, &*s.right],
            LogicalOperator::Intersect(s) => vec![&*s.left, &*s.right],
            LogicalOperator::Explain(s) => vec![&*s.inner],
            LogicalOperator::RecursiveExtend(_) => vec![],
            LogicalOperator::TableFunctionCall(_) => vec![],
            LogicalOperator::CopyFrom(_)
            | LogicalOperator::Delete(_)
            | LogicalOperator::Set(_)
            | LogicalOperator::Unwind(_)
            | LogicalOperator::Foreach(_)
            | LogicalOperator::Merge(_) => vec![],
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
            | LogicalOperator::ExportDatabase(_)
            | LogicalOperator::ImportDatabase(_) => vec![],
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogicalArtIndexRangeScan {
    pub table_name: String,
    pub table_id: u64,
    pub lower_bound: Option<kuzu_common::types::Value>,
    pub upper_bound: Option<kuzu_common::types::Value>,
    pub lower_inclusive: bool,
    pub upper_inclusive: bool,
    pub cardinality: u64,
}

#[derive(Debug, Clone)]
pub struct LogicalVectorSimilarityScan {
    pub index_name: String,
    pub index_id: u64,
    pub query_vector: Vec<f64>,
    pub top_k: u64,
    pub table_name: String,
    pub cardinality: u64,
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
    pub explain_type: kuzu_parser::ast::ExplainType,
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
    pub expression: kuzu_parser::ast::Expression,
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
/// This is a leaf operator that executes BFS traversal during query execution.
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
    pub direction: kuzu_common::enums::ExtendDirection,
    /// Path semantic (WALK / TRAIL / ACYCLIC).
    pub semantic: kuzu_common::enums::PathSemantic,
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

/// FOREACH operator — iterates over list elements and executes sub-plans.
#[derive(Debug, Clone)]
pub struct LogicalForeach {
    pub variable: String,
    pub expression: kuzu_parser::ast::Expression,
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
    pub args: Vec<kuzu_parser::ast::Expression>,
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
    pub properties: Vec<(String, kuzu_parser::ast::Expression)>,
    /// SET operations to apply when the node already exists (matched).
    pub on_match: Vec<LogicalSet>,
    /// SET operations to apply when a new node is created.
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
    pub action: kuzu_parser::ast::AlterAction,
    pub cardinality: u64,
}

/// Logical operator for CREATE [ART|HASH] INDEX.
#[derive(Debug, Clone)]
pub struct LogicalCreateIndex {
    pub index_type: kuzu_catalog::IndexType,
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
    pub properties: Vec<(String, kuzu_parser::ast::Expression)>,
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
