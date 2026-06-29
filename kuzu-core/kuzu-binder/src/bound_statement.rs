//! Bound statement types — AST nodes after semantic analysis.

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
    BoundUnion(BoundUnion),
    BoundMerge(BoundMerge),
    BoundCreateDml(BoundCreateDml),
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

/// Bound CALL statement — invoke a table function.
#[derive(Debug, Clone)]
pub struct BoundCall {
    pub function_name: String,
    pub args: Vec<kuzu_parser::ast::Expression>,
}

/// Bound CREATE DML statement — create a node with properties.
#[derive(Debug, Clone)]
pub struct BoundCreateDml {
    pub table_name: String,
    pub table_id: u64,
    pub properties: Vec<(String, kuzu_parser::ast::Expression)>,
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
