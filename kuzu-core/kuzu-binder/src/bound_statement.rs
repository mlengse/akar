//! Bound statement types — AST nodes after semantic analysis.

use kuzu_catalog::CatalogEntry;
use kuzu_common::types::LogicalTypeID;
use kuzu_parser::ast::Expression;

/// A bound statement after semantic analysis.
#[derive(Debug, Clone)]
pub enum BoundStatement {
    BoundQuery(BoundQuery),
    BoundCreateNodeTable(BoundCreateNodeTable),
    BoundCreateRelTable(BoundCreateRelTable),
    BoundDropTable(BoundDropTable),
}

#[derive(Debug, Clone)]
pub struct BoundQuery {
    pub clauses: Vec<BoundClause>,
}

#[derive(Debug, Clone)]
pub enum BoundClause {
    BoundMatch(BoundMatchClause),
    BoundReturn(BoundReturnClause),
    BoundWhere(BoundWhereClause),
    BoundCreate(BoundCreateClause),
}

#[derive(Debug, Clone)]
pub struct BoundMatchClause {
    pub patterns: Vec<BoundPattern>,
}

#[derive(Debug, Clone)]
pub struct BoundPattern {
    pub node_variable: Option<String>,
    pub node_label: Option<String>,
    pub edge: Option<BoundEdgePattern>,
}

#[derive(Debug, Clone)]
pub struct BoundEdgePattern {
    pub variable: Option<String>,
    pub label: Option<String>,
    pub direction: kuzu_parser::ast::EdgeDirection,
}

#[derive(Debug, Clone)]
pub struct BoundReturnClause {
    pub expressions: Vec<(Expression, Option<String>, LogicalTypeID)>,
}

#[derive(Debug, Clone)]
pub struct BoundWhereClause {
    pub expression: Expression,
}

#[derive(Debug, Clone)]
pub struct BoundCreateClause {
    pub patterns: Vec<BoundPattern>,
}

// DDL
#[derive(Debug, Clone)]
pub struct BoundCreateNodeTable {
    pub name: String,
    pub catalog_entry: CatalogEntry,
}

#[derive(Debug, Clone)]
pub struct BoundCreateRelTable {
    pub name: String,
    pub catalog_entry: CatalogEntry,
}

#[derive(Debug, Clone)]
pub struct BoundDropTable {
    pub name: String,
}
