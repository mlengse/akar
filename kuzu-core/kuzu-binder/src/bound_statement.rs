//! Bound statement types — AST nodes after semantic analysis.

use kuzu_catalog::IndexType;
use kuzu_common::types::LogicalTypeID;
use kuzu_parser::ast::Expression;
use std::collections::HashMap;

/// A bound statement after semantic analysis.
#[derive(Debug, Clone)]
pub enum BoundStatement {
    BoundQuery(BoundQuery),
    BoundCall(BoundCall),
    BoundCreateNodeTable(BoundCreateNodeTable),
    BoundCreateRelTable(BoundCreateRelTable),
    BoundDropTable(BoundDropTable),
    BoundCopyFrom(BoundCopyFrom),
    BoundAlterTable(BoundAlterTable),
    BoundCreateVectorIndex(BoundCreateVectorIndex),
    BoundCreateIndex(BoundCreateIndex),
    BoundDropIndex(BoundDropIndex),
    BoundUnion(BoundUnion),
    BoundMerge(BoundMerge),
    BoundCreateDml(BoundCreateDml),
    BoundExplain(BoundExplain),
    BoundCreateSequence(BoundCreateSequence),
    BoundDropSequence(BoundDropSequence),
}

/// COPY FROM statement — load data from a file into a table.
#[derive(Debug, Clone)]
pub struct BoundCopyFrom {
    pub table_name: String,
    pub table_id: u64,
    pub file_path: String,
    pub options: HashMap<String, String>,
    /// Resolved column schema from the catalog (column names + types).
    pub columns: Vec<kuzu_catalog::CatalogColumn>,
}

/// A resolved variable in scope (from MATCH patterns).
#[derive(Debug, Clone)]
pub struct BoundVariable {
    pub name: String,
    pub table_id: u64,
    pub label: Option<String>,
    pub is_node: bool,
}

#[derive(Debug, Clone)]
pub struct BoundQuery {
    pub clauses: Vec<BoundClause>,
    /// Variables in scope (accumulated across clauses).
    pub variables: Vec<BoundVariable>,
}

#[derive(Debug, Clone)]
pub enum BoundClause {
    BoundMatch(BoundMatchClause),
    BoundReturn(BoundReturnClause),
    BoundWhere(BoundWhereClause),
    BoundDelete(BoundDeleteClause),
    BoundSet(BoundSetClause),
    BoundOptionalMatch(BoundMatchClause),
    BoundWith(BoundReturnClause),
    BoundUnwind(BoundUnwindClause),
    BoundForeach(BoundForeachClause),
}

#[derive(Debug, Clone)]
pub struct BoundSetItem {
    pub property: kuzu_parser::ast::Expression,
    pub value: kuzu_parser::ast::Expression,
    pub column_name: String,
    pub column_idx: usize,
    pub table_name: String,
    pub table_id: u64,
}

#[derive(Debug, Clone)]
pub struct BoundSetClause {
    pub items: Vec<BoundSetItem>,
}

#[derive(Debug, Clone)]
pub struct BoundDeleteClause {
    pub expressions: Vec<Expression>,
    pub table_name: String,
    pub table_id: u64,
    pub primary_key_column: String,
}

#[derive(Debug, Clone)]
pub struct BoundUnwindClause {
    pub expression: kuzu_parser::ast::Expression,
    pub variable: String,
}

/// Bound EXPLAIN statement — wraps an inner bound statement.
#[derive(Debug, Clone)]
pub struct BoundExplain {
    /// The inner bound statement to explain.
    pub inner: Box<BoundStatement>,
    /// The type of explain output.
    pub explain_type: kuzu_parser::ast::ExplainType,
}

/// Bound FOREACH clause — iterate over list and execute sub-statements.
#[derive(Debug, Clone)]
pub struct BoundForeachClause {
    pub variable: String,
    pub expression: kuzu_parser::ast::Expression,
    /// Bound sub-statements (CREATE, SET, DELETE) produced by bind_foreach.
    pub sub_statements: Vec<BoundStatement>,
}

#[derive(Debug, Clone)]
pub struct BoundMatchClause {
    pub patterns: Vec<BoundPattern>,
    /// New variables introduced by this MATCH clause.
    pub new_variables: Vec<BoundVariable>,
}

#[derive(Debug, Clone)]
pub struct BoundPattern {
    pub node_variable: Option<String>,
    pub node_label: Option<String>,
    pub node_table_id: Option<u64>,
    pub edge: Option<BoundEdgePattern>,
}

#[derive(Debug, Clone)]
pub struct BoundEdgePattern {
    pub variable: Option<String>,
    pub label: Option<String>,
    pub rel_table_id: Option<u64>,
    pub direction: kuzu_parser::ast::EdgeDirection,
    pub lower_bound: Option<u64>,
    pub upper_bound: Option<u64>,
}

/// A bound expression with resolved type information.
#[derive(Debug, Clone)]
pub struct BoundExpression {
    pub expression: Expression,
    pub resolved_type: LogicalTypeID,
    pub is_constant: bool,
}

#[derive(Debug, Clone)]
pub struct BoundReturnClause {
    pub expressions: Vec<BoundExpression>,
}

#[derive(Debug, Clone)]
pub struct BoundWhereClause {
    pub expression: BoundExpression,
}

// DDL
#[derive(Debug, Clone)]
pub struct BoundCreateNodeTable {
    pub name: String,
    pub columns: Vec<kuzu_catalog::CatalogColumn>,
    pub primary_key: String,
}

#[derive(Debug, Clone)]
pub struct BoundCreateRelTable {
    pub name: String,
    pub from: String,
    pub to: String,
    pub columns: Vec<kuzu_catalog::CatalogColumn>,
}

#[derive(Debug, Clone)]
pub struct BoundDropTable {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct BoundAlterTable {
    pub table_name: String,
    pub action: kuzu_parser::ast::AlterAction,
}

#[derive(Debug, Clone)]
pub struct BoundUnion {
    pub left: Box<BoundQuery>,
    pub right: Box<BoundQuery>,
    pub all: bool,
}

/// Bound CREATE VECTOR INDEX statement.
#[derive(Debug, Clone)]
pub struct BoundCreateVectorIndex {
    pub index_name: String,
    pub table_name: String,
    pub column_name: String,
    pub metric: String,
    pub dimensions: u64,
}

/// Bound `CREATE [ART|HASH] INDEX` statement.
#[derive(Debug, Clone)]
pub struct BoundCreateIndex {
    pub index_type: IndexType,
    pub index_name: String,
    pub table_name: String,
    pub column_name: String,
}

/// Bound `DROP INDEX` statement.
#[derive(Debug, Clone)]
pub struct BoundDropIndex {
    pub index_name: String,
    pub table_name: String,
}

/// Bound CALL statement — invoke a table function.
#[derive(Debug, Clone)]
pub struct BoundCall {
    pub function_name: String,
    pub args: Vec<kuzu_parser::ast::Expression>,
}

/// Bound CREATE SEQUENCE statement.
#[derive(Debug, Clone)]
pub struct BoundCreateSequence {
    pub name: String,
    pub if_not_exists: bool,
    pub or_replace: bool,
    pub start_with: i64,
    pub increment: i64,
    pub min_value: i64,
    pub max_value: i64,
    pub cycle: bool,
}

/// Bound DROP SEQUENCE statement.
#[derive(Debug, Clone)]
pub struct BoundDropSequence {
    pub name: String,
    pub if_exists: bool,
}

/// Bound CREATE DML statement — create a node with properties.
#[derive(Debug, Clone)]
pub struct BoundCreateDml {
    pub table_name: String,
    pub table_id: u64,
    pub properties: Vec<(String, Expression)>,
}

/// Bound MERGE statement — match or create a node pattern.
#[derive(Debug, Clone)]
pub struct BoundMerge {
    pub table_name: String,
    pub table_id: u64,
    /// Properties from the MERGE pattern (used for matching and creation).
    pub properties: Vec<(String, kuzu_parser::ast::Expression)>,
    /// ON CREATE SET items: resolved column info.
    pub on_create: Vec<BoundSetItem>,
    /// ON MATCH SET items: resolved column info.
    pub on_match: Vec<BoundSetItem>,
}
