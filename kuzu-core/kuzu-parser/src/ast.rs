//! Abstract Syntax Tree (AST) types for Cypher queries.

/// A Cypher statement (top-level AST node).
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Query(Query),
    CreateNodeTable(CreateNodeTable),
    CreateRelTable(CreateRelTable),
    DropTable(DropTable),
}

/// A Cypher query (e.g., MATCH ... RETURN ...).
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub clauses: Vec<Clause>,
}

/// A clause in a query.
#[derive(Debug, Clone, PartialEq)]
pub enum Clause {
    Match(MatchClause),
    Return(ReturnClause),
    Where(WhereClause),
    Create(CreateClause),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchClause {
    pub patterns: Vec<Pattern>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReturnClause {
    pub expressions: Vec<ReturnItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReturnItem {
    pub expression: Expression,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhereClause {
    pub expression: Expression,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateClause {
    pub patterns: Vec<Pattern>,
}

/// A graph pattern (node or relationship).
#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub node: Option<NodePattern>,
    pub edge: Option<EdgePattern>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodePattern {
    pub variable: Option<String>,
    pub labels: Vec<String>,
    pub properties: Vec<(String, Expression)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EdgePattern {
    pub variable: Option<String>,
    pub labels: Vec<String>,
    pub direction: EdgeDirection,
    pub properties: Vec<(String, Expression)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EdgeDirection {
    LeftToRight,
    RightToLeft,
    Both,
}

/// An expression in a Cypher query.
#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    Constant(Constant),
    Variable(String),
    /// A query parameter reference like `$name` or `$age`.
    Parameter(String),
    PropertyAccess(Box<Expression>, String),
    FunctionCall(String, Vec<Expression>),
    BinaryOp(BinaryOp, Box<Expression>, Box<Expression>),
    UnaryOp(UnaryOp, Box<Expression>),
    List(Vec<Expression>),
    Map(Vec<(String, Expression)>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Constant {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    And,
    Or,
    Xor,
    Concat,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOp {
    Not,
    Negate,
}

// DDL statements
#[derive(Debug, Clone, PartialEq)]
pub struct CreateNodeTable {
    pub name: String,
    pub columns: Vec<ColumnDef>,
    pub primary_key: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateRelTable {
    pub name: String,
    pub from: String,
    pub to: String,
    pub columns: Vec<ColumnDef>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DropTable {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnDef {
    pub name: String,
    pub type_name: String,
}
