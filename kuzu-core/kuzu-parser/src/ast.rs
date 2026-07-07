//! Abstract Syntax Tree (AST) types for Cypher queries.

/// EXPLAIN type — what kind of plan to show.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExplainType {
    /// `EXPLAIN` — show the physical plan (default).
    PhysicalPlan,
    /// `EXPLAIN LOGICAL` — show the logical plan.
    LogicalPlan,
    /// `EXPLAIN PROFILE` — execute and show profile.
    Profile,
}

/// EXPORT DATABASE statement.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportDatabase {
    /// Path to the export directory.
    pub file_path: String,
    /// Export options (format, schema_only, etc.).
    pub options: std::collections::HashMap<String, String>,
}

/// IMPORT DATABASE statement.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportDatabase {
    /// Path to the import directory (previously exported).
    pub file_path: String,
}

/// ANALYZE statement — collect table statistics.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalyzeStatement {
    /// Table name to analyze, or None for all tables (ANALYZE *).
    pub table_name: Option<String>,
}

/// Transaction action type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransactionAction {
    Begin,
    Commit,
    Rollback,
    Checkpoint,
}

/// TRANSACTION statement (BEGIN, COMMIT, ROLLBACK, CHECKPOINT).
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionStatement {
    pub action: TransactionAction,
}

/// Extension management action.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExtensionAction {
    Install,
    Load,
    Uninstall,
}

/// EXTENSION management statement (INSTALL/LOAD/UNINSTALL EXTENSION name).
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionStatement {
    pub action: ExtensionAction,
    pub name: String,
}

/// ATTACH DATABASE statement.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachDatabase {
    pub path: String,
    pub alias: String,
    pub options: std::collections::HashMap<String, String>,
}

/// DETACH DATABASE statement.
#[derive(Debug, Clone, PartialEq)]
pub struct DetachDatabase {
    pub alias: String,
}

/// USE DATABASE statement.
#[derive(Debug, Clone, PartialEq)]
pub struct UseDatabase {
    pub alias: String,
}

/// LOAD FROM statement — scan external file without importing.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadFrom {
    pub path: String,
    pub options: std::collections::HashMap<String, String>,
}

/// EXPLAIN statement wrapper.
#[derive(Debug, Clone, PartialEq)]
pub struct ExplainStatement {
    /// The statement being explained.
    pub statement: Box<Statement>,
    /// The type of explain output.
    pub explain_type: ExplainType,
}

/// A Cypher statement (top-level AST node).
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Query(Query),
    CreateNodeTable(CreateNodeTable),
    CreateRelTable(CreateRelTable),
    DropTable(DropTable),
    CopyFrom(CopyFrom),
    CopyTo(CopyTo),
    AlterTable(AlterTable),
    CreateVectorIndex(CreateVectorIndex),
    CreateIndex(CreateIndex),
    DropIndex(DropIndex),
    Union(UnionStatement),
    Merge(MergeStatement),
    Call(CallStatement),
    CreateDml(CreateClause),
    Explain(ExplainStatement),
    CreateSequence(CreateSequence),
    DropSequence(DropSequence),
    CreateMacro(CreateMacro),
    ExportDatabase(ExportDatabase),
    ImportDatabase(ImportDatabase),
    Analyze(AnalyzeStatement),
    CreateFtsIndex(CreateFtsIndex),
    Transaction(TransactionStatement),
    Extension(ExtensionStatement),
    AttachDatabase(AttachDatabase),
    DetachDatabase(DetachDatabase),
    UseDatabase(UseDatabase),
    LoadFrom(LoadFrom),
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
    Delete(DeleteClause),
    Set(SetClause),
    OptionalMatch(OptionalMatchClause),
    With(ReturnClause),
    Unwind(UnwindClause),
    Foreach(ForeachClause),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForeachClause {
    pub variable: String,
    pub expression: Expression,
    /// Sub-statements inside FOREACH (CREATE, SET, DELETE clauses).
    pub clauses: Vec<Clause>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnwindClause {
    pub expression: Expression,
    pub variable: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetClause {
    pub items: Vec<SetItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetItem {
    pub property: Expression,
    pub value: Expression,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeleteClause {
    pub detach: bool,
    pub expressions: Vec<Expression>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchClause {
    pub patterns: Vec<Pattern>,
    /// Optional FTS query attached to this MATCH via `USING FTS INDEX name('query')`.
    pub fts_query: Option<FtsQuery>,
}

/// A `USING FTS INDEX index_name('search string')` clause attached to a MATCH.
#[derive(Debug, Clone, PartialEq)]
pub struct FtsQuery {
    pub index_name: String,
    pub query_string: String,
}

/// CREATE FTS INDEX statement.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateFtsIndex {
    pub index_name: String,
    pub table_name: String,
    pub column_name: String,
    pub if_not_exists: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OptionalMatchClause {
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
    pub lower_bound: Option<u64>,
    pub upper_bound: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EdgeDirection {
    LeftToRight,
    RightToLeft,
    Both,
}

impl ExplainStatement {
    /// Create a new EXPLAIN statement wrapping the given inner statement.
    pub fn new(statement: Statement, explain_type: ExplainType) -> Self {
        Self {
            statement: Box::new(statement),
            explain_type,
        }
    }
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
    /// EXISTS { MATCH ... WHERE ... } — returns true if the pattern matches.
    ExistsSubquery(Box<Query>),
    /// CASE [subject] WHEN ... THEN ... [ELSE ...] END
    Case(CaseExpr),
    /// STAR expression — represents `*` in `RETURN *`.
    /// The binder expands this to all variables in scope.
    Star,
    /// ANY/ALL/NONE/SINGLE list predicates.
    /// Example: ANY(x IN [1,2,3] WHERE x > 5)
    ListPredicate {
        quantifier: Quantifier,
        list: Box<Expression>,
        var_name: String,
        predicate: Box<Expression>,
    },
}

/// Quantifier for list predicates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Quantifier {
    Any,
    All,
    None,
    Single,
}

/// A single WHEN ... THEN ... branch inside a CASE expression.
#[derive(Debug, Clone, PartialEq)]
pub struct CaseAlternative {
    /// The WHEN expression (a value for simple CASE, or a condition for searched CASE).
    pub when: Expression,
    /// The THEN expression returned when WHEN matches.
    pub then: Expression,
}

/// A CASE expression (simple or searched).
#[derive(Debug, Clone, PartialEq)]
pub struct CaseExpr {
    /// Optional subject expression for simple CASE: `CASE x WHEN v THEN ...`
    pub subject: Option<Box<Expression>>,
    /// The WHEN/THEN branches.
    pub alternatives: Vec<CaseAlternative>,
    /// Optional ELSE expression returned when no branch matches.
    pub else_expr: Option<Box<Expression>>,
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
    /// x IN [list] — true if x equals any element of the list
    In,
    /// x NOT IN [list] — true if x equals no element of the list
    NotIn,
    /// x STARTS WITH prefix — true if string x starts with prefix
    StartsWith,
    /// x ENDS WITH suffix — true if string x ends with suffix
    EndsWith,
    /// x CONTAINS substr — true if string x contains substr
    Contains,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOp {
    Not,
    Negate,
    /// x IS NULL — true if x evaluates to null
    IsNull,
    /// x IS NOT NULL — true if x does not evaluate to null
    IsNotNull,
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

/// A `CREATE [ART|HASH] INDEX` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateIndex {
    pub index_type: String,
    pub index_name: String,
    pub table_name: String,
    pub variable: String,
    pub property: String,
    pub conflict_action: Option<String>,
}

/// A `DROP INDEX` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct DropIndex {
    pub index_name: String,
    pub table_name: String,
}

/// ALTER TABLE statement.
#[derive(Debug, Clone, PartialEq)]
pub struct AlterTable {
    pub table_name: String,
    pub action: AlterAction,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AlterAction {
    AddColumn { name: String, type_name: String },
    DropColumn { name: String },
    RenameColumn { old_name: String, new_name: String },
    RenameTable { new_name: String },
}

/// UNION statement — combines results from two queries.
#[derive(Debug, Clone, PartialEq)]
pub struct UnionStatement {
    pub left: Query,
    pub right: Query,
    pub all: bool,
}

/// COPY FROM statement — load data from a file into a table.
#[derive(Debug, Clone, PartialEq)]
pub struct CopyFrom {
    pub table_name: String,
    pub file_path: String,
    pub options: std::collections::HashMap<String, String>,
}

/// COPY TO statement — export query results to a file.
///
/// Syntax: `COPY (query) TO 'path' (FORMAT 'CSV'|'PARQUET', HEADER true|false)`
#[derive(Debug, Clone, PartialEq)]
pub struct CopyTo {
    pub query: Query,
    pub file_path: String,
    pub format: CopyToFormat,
    pub header: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CopyToFormat {
    Csv,
    Parquet,
}

/// MERGE statement — match or create a pattern with optional ON CREATE / ON MATCH actions.
#[derive(Debug, Clone, PartialEq)]
pub struct MergeStatement {
    pub patterns: Vec<Pattern>,
    pub on_create: Vec<SetItem>,
    pub on_match: Vec<SetItem>,
}

/// CALL statement — invoke a table function or procedure.
#[derive(Debug, Clone, PartialEq)]
pub struct CallStatement {
    pub function_name: String,
    pub args: Vec<Expression>,
}

/// CREATE SEQUENCE statement — creates a sequence for auto-incrementing counters.
///
/// Syntax:
/// ```sql
/// CREATE [OR REPLACE] SEQUENCE [IF NOT EXISTS] name
///   [START WITH value]
///   [INCREMENT [BY] value]
///   [MINVALUE value | NO MINVALUE]
///   [MAXVALUE value | NO MAXVALUE]
///   [CYCLE | NO CYCLE]
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct CreateSequence {
    pub name: String,
    pub if_not_exists: bool,
    pub or_replace: bool,
    /// START WITH value. Default: 1 for increment > 0, max_value for increment < 0.
    pub start_with: Option<i64>,
    /// INCREMENT BY value. Default: 1.
    pub increment: Option<i64>,
    /// MINVALUE. Auto-computed from defaults if None.
    pub min_value: Option<i64>,
    /// MAXVALUE. Auto-computed from defaults if None.
    pub max_value: Option<i64>,
    /// CYCLE behavior. Default: false (NO CYCLE).
    pub cycle: Option<bool>,
}

/// DROP SEQUENCE statement.
#[derive(Debug, Clone, PartialEq)]
pub struct DropSequence {
    pub name: String,
    pub if_exists: bool,
}

/// CREATE MACRO statement — defines a Cypher scalar macro.
///
/// Syntax: `CREATE MACRO macroName(param1, param2, ...) AS expression`
///
/// Macros are expanded at binding time: macro invocations are replaced
/// with the macro body expression (with parameters substituted).
///
/// Ported from C++ `parser/create_macro.h`.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateMacro {
    /// The macro name.
    pub name: String,
    /// Positional parameter names (no default value).
    pub positional_args: Vec<String>,
    /// Parameters with default values (name, default expression).
    pub default_args: Vec<(String, Expression)>,
    /// The macro body expression.
    pub expression: Box<Expression>,
}

/// CREATE VECTOR INDEX statement — creates an HNSW index on a vector column.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateVectorIndex {
    pub index_name: String,
    pub table_name: String,
    pub column_name: String,
    pub metric: String,
    pub dimensions: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnDef {
    pub name: String,
    pub type_name: String,
}
