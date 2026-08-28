//! Bound statement types — AST nodes after semantic analysis.

use akar_catalog::IndexType;
use akar_common::types::LogicalTypeID;
use akar_parser::ast::{Expression, ExtensionAction, TransactionAction};
use std::collections::HashMap;

/// A bound statement after semantic analysis.
#[derive(Debug, Clone)]
pub enum BoundStatement {
    BoundQuery(BoundQuery),
    BoundStandaloneCall(BoundStandaloneCall),
    BoundCreateNodeTable(BoundCreateNodeTable),
    BoundCreateRelTable(BoundCreateRelTable),
    BoundDropTable(BoundDropTable),
    BoundCopyFrom(BoundCopyFrom),
    BoundCopyTo(BoundCopyTo),
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
    BoundCreateMacro(BoundCreateMacro),
    BoundExportDatabase(BoundExportDatabase),
    BoundImportDatabase(BoundImportDatabase),
    BoundCreateFtsIndex(BoundCreateFtsIndex),
    BoundAnalyze(BoundAnalyze),
    BoundTransaction(BoundTransaction),
    BoundExtension(BoundExtension),
    BoundAttachDatabase(BoundAttachDatabase),
    BoundDetachDatabase(BoundDetachDatabase),
    BoundUseDatabase(BoundUseDatabase),
    BoundLoadFrom(BoundLoadFrom),
    BoundCreateType(BoundCreateType),
    BoundCommentOnTable(BoundCommentOnTable),
    BoundCreateGraph(BoundCreateGraph),
    BoundUseGraph(BoundUseGraph),
    BoundDropGraph(BoundDropGraph),
}

/// Bound TRANSACTION statement.
#[derive(Debug, Clone)]
pub struct BoundTransaction {
    pub action: TransactionAction,
}

/// Bound EXTENSION management statement.
#[derive(Debug, Clone)]
pub struct BoundExtension {
    pub action: ExtensionAction,
    pub name: String,
}

/// Bound ATTACH DATABASE statement.
#[derive(Debug, Clone)]
pub struct BoundAttachDatabase {
    pub path: String,
    pub alias: String,
    pub options: HashMap<String, String>,
}

/// Bound DETACH DATABASE statement.
#[derive(Debug, Clone)]
pub struct BoundDetachDatabase {
    pub alias: String,
}

/// Bound USE DATABASE statement.
#[derive(Debug, Clone)]
pub struct BoundUseDatabase {
    pub alias: String,
}

/// Bound LOAD FROM statement.
#[derive(Debug, Clone)]
pub struct BoundLoadFrom {
    pub path: String,
    pub options: HashMap<String, String>,
}

/// Bound ANALYZE statement.
#[derive(Debug, Clone)]
pub struct BoundAnalyze {
    /// Table name to analyze, or None for all tables.
    pub table_name: Option<String>,
    /// Resolved table IDs for the tables to analyze.
    pub table_ids: Vec<u64>,
}

/// COPY FROM statement — load data from a file into a table.
#[derive(Debug, Clone)]
pub struct BoundCopyFrom {
    pub table_name: String,
    pub table_id: u64,
    pub file_path: String,
    pub options: HashMap<String, String>,
    /// Resolved column schema from the catalog (column names + types).
    pub columns: Vec<akar_catalog::CatalogColumn>,
}

/// COPY TO statement — export query results to a file.
#[derive(Debug, Clone)]
pub struct BoundCopyTo {
    pub file_path: String,
    pub format: akar_parser::ast::CopyToFormat,
    pub header: bool,
    /// The inner query to execute.
    pub query: BoundQuery,
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
    BoundCreate(BoundMatchClause),
    BoundMerge(BoundMerge),
}

#[derive(Debug, Clone)]
pub struct BoundSetItem {
    pub property: akar_parser::ast::Expression,
    pub value: akar_parser::ast::Expression,
    pub column_name: String,
    pub column_idx: usize,
    pub table_name: String,
    pub table_id: u64,
    pub is_node: bool,
}

#[derive(Debug, Clone)]
pub struct BoundSetClause {
    pub items: Vec<BoundSetItem>,
}

#[derive(Debug, Clone)]
pub struct BoundDeleteItem {
    pub expression: Expression,
    pub table_name: String,
    pub table_id: u64,
    pub primary_key_column: String,
    pub is_node: bool,
}

#[derive(Debug, Clone)]
pub struct BoundDeleteClause {
    pub detach: bool,
    pub items: Vec<BoundDeleteItem>,
}

#[derive(Debug, Clone)]
pub struct BoundUnwindClause {
    pub expression: akar_parser::ast::Expression,
    pub variable: String,
}

/// Bound EXPLAIN statement — wraps an inner bound statement.
#[derive(Debug, Clone)]
pub struct BoundExplain {
    /// The inner bound statement to explain.
    pub inner: Box<BoundStatement>,
    /// The type of explain output.
    pub explain_type: akar_parser::ast::ExplainType,
}

/// Bound FOREACH clause — iterate over list and execute sub-statements.
#[derive(Debug, Clone)]
pub struct BoundForeachClause {
    pub variable: String,
    pub expression: akar_parser::ast::Expression,
    /// Bound sub-statements (CREATE, SET, DELETE) produced by bind_foreach.
    pub sub_statements: Vec<BoundStatement>,
}

#[derive(Debug, Clone)]
pub struct BoundMatchClause {
    pub patterns: Vec<BoundPattern>,
    /// New variables introduced by this MATCH clause.
    pub new_variables: Vec<BoundVariable>,
    /// Optional FTS query (from USING FTS INDEX clause).
    pub fts_query: Option<BoundFtsQuery>,
}

/// A bound USING FTS INDEX query.
#[derive(Debug, Clone)]
pub struct BoundFtsQuery {
    pub index_name: String,
    pub query_string: String,
    /// The macro table names derived from the FTS index.
    pub docs_table: String,
    pub terms_table: String,
    pub posting_table: String,
    /// The source node table/column the index was created on (P52.39), used
    /// to keep the derived macro tables in sync with live DML.
    pub table_name: String,
    pub column_name: String,
}

#[derive(Debug, Clone)]
pub struct BoundPattern {
    pub node_variable: Option<String>,
    pub node_label: Option<String>,
    pub node_table_id: Option<u64>,
    pub properties: Vec<(String, akar_parser::ast::Expression)>,
    pub edge: Option<BoundEdgePattern>,
}

#[derive(Debug, Clone)]
pub struct BoundEdgePattern {
    pub variable: Option<String>,
    pub label: Option<String>,
    pub rel_table_id: Option<u64>,
    pub direction: akar_parser::ast::EdgeDirection,
    pub properties: Vec<(String, akar_parser::ast::Expression)>,
    pub lower_bound: Option<u64>,
    pub upper_bound: Option<u64>,
}

/// A bound expression with resolved type information.
#[derive(Debug, Clone)]
pub struct BoundExpression {
    pub expression: Expression,
    pub resolved_type: LogicalTypeID,
    pub is_constant: bool,
    /// Output column name override from `AS alias` in RETURN/WITH (P53.16).
    pub alias: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BoundReturnClause {
    pub expressions: Vec<BoundExpression>,
    pub distinct: bool,
    /// Bound ORDER BY items.
    pub order_by: Option<Vec<BoundOrderByItem>>,
    /// Bound LIMIT.
    pub limit: Option<u64>,
    /// Bound SKIP.
    pub skip: Option<u64>,
}

/// A bound sort item for ORDER BY.
#[derive(Debug, Clone)]
pub struct BoundOrderByItem {
    pub expression: BoundExpression,
    pub ascending: bool,
}

#[derive(Debug, Clone)]
pub struct BoundWhereClause {
    pub expression: BoundExpression,
}

// DDL
#[derive(Debug, Clone)]
pub struct BoundCreateNodeTable {
    pub name: String,
    pub columns: Vec<akar_catalog::CatalogColumn>,
    pub primary_key: String,
    pub if_not_exists: bool,
}

#[derive(Debug, Clone)]
pub struct BoundCreateRelTable {
    pub name: String,
    pub from: String,
    pub to: String,
    pub src_table_id: u64,
    pub dst_table_id: u64,
    pub columns: Vec<akar_catalog::CatalogColumn>,
    pub if_not_exists: bool,
}

#[derive(Debug, Clone)]
pub struct BoundDropTable {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct BoundAlterTable {
    pub table_name: String,
    pub action: akar_parser::ast::AlterAction,
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
pub struct BoundStandaloneCall {
    pub function_name: String,
    pub args: Vec<akar_parser::ast::Expression>,
}

/// Bound EXPORT DATABASE statement.
#[derive(Debug, Clone)]
pub struct BoundExportDatabase {
    pub file_path: String,
    pub file_type: String,
    pub schema_only: bool,
    pub options: std::collections::HashMap<String, String>,
}

/// Bound IMPORT DATABASE statement.
#[derive(Debug, Clone)]
pub struct BoundImportDatabase {
    pub file_path: String,
    pub query: String,
    pub index_query: String,
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

/// Bound CREATE MACRO statement.
#[derive(Debug, Clone)]
pub struct BoundCreateMacro {
    pub name: String,
    pub positional_args: Vec<String>,
    pub default_args: Vec<(String, String)>,
    pub expression: String,
}

/// A node element of a CREATE/MERGE pattern.
#[derive(Debug, Clone)]
pub struct BoundNodeCreate {
    pub variable: Option<String>,
    pub table_name: String,
    pub table_id: u64,
    pub properties: Vec<(String, akar_parser::ast::Expression)>,
}

/// An edge element of a CREATE/MERGE pattern with resolved endpoints.
#[derive(Debug, Clone)]
pub struct BoundEdgeCreate {
    pub variable: Option<String>,
    pub table_name: String,
    pub table_id: u64,
    pub src_var: String,
    pub dst_var: String,
    pub properties: Vec<(String, akar_parser::ast::Expression)>,
}

/// A single element of a CREATE/MERGE pattern path (node and/or edge).
#[derive(Debug, Clone)]
pub struct BoundCreatePattern {
    pub node: Option<BoundNodeCreate>,
    pub edge: Option<BoundEdgeCreate>,
}

/// Bound CREATE DML statement — create nodes and/or relationships.
#[derive(Debug, Clone)]
pub struct BoundCreateDml {
    pub patterns: Vec<BoundCreatePattern>,
}

/// Bound MERGE statement — match or create a node/relationship pattern.
#[derive(Debug, Clone)]
pub struct BoundMerge {
    pub table_name: String,
    pub table_id: u64,
    /// Properties from the primary (first) MERGE pattern node (used for matching and creation).
    pub properties: Vec<(String, akar_parser::ast::Expression)>,
    /// All bound patterns (nodes + edges) of the MERGE statement.
    pub patterns: Vec<BoundCreatePattern>,
    /// ON CREATE SET items: resolved column info.
    pub on_create: Vec<BoundSetItem>,
    /// ON MATCH SET items: resolved column info.
    pub on_match: Vec<BoundSetItem>,
}

/// Bound CREATE FTS INDEX statement.
#[derive(Debug, Clone)]
pub struct BoundCreateFtsIndex {
    pub index_name: String,
    pub table_name: String,
    pub column_name: String,
    pub if_not_exists: bool,
    /// The macro table names that will be created.
    pub docs_table: String,
    pub terms_table: String,
    pub posting_table: String,
}

/// Bound CREATE TYPE — user-defined type alias.
#[derive(Debug, Clone)]
pub struct BoundCreateType {
    pub name: String,
    pub type_name: String,
}

/// Bound COMMENT ON TABLE — table comment.
#[derive(Debug, Clone)]
pub struct BoundCommentOnTable {
    pub table_name: String,
    pub comment: String,
}

/// Bound CREATE GRAPH — projected graph.
#[derive(Debug, Clone)]
pub struct BoundCreateGraph {
    pub name: String,
    pub is_any: bool,
}

/// Bound USE GRAPH — set graph context.
#[derive(Debug, Clone)]
pub struct BoundUseGraph {
    pub name: String,
}

/// Bound DROP GRAPH — remove projected graph.
#[derive(Debug, Clone)]
pub struct BoundDropGraph {
    pub name: String,
}
