# Audit parser details 19/07/2026

## Comprehensive Audit: Rust Parser Crate vs C++ ANTLR Parser

### 1. Crate Overview

| Aspect | Details |
|---|---|
| **Location** | `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-parser\` |
| **Parser engine** | `pest` v2 (PEG parser generator) |
| **Grammar file** | `src/cypher.pest` (477 lines) |
| **AST definitions** | `src/ast.rs` (604 lines) |
| **Parser code** | `src/parser/mod.rs` (entry), `src/parser/expression.rs`, `src/parser/dml.rs`, `src/parser/ddl.rs` |
| **Tests** | `src/parser_test.rs` (753 lines, ~60 tests) |
| **Dependencies** | `pest`, `pest_derive` only (no ANTLR dependency) |
| **`todo!()` / `unimplemented!()`** | **NONE found anywhere in the crate** |

---

### 2. All Supported Statement Types (Rust Parser)

Derived from the `Statement` enum in `ast.rs` lines 105-139:

| # | Statement Variant | AST Struct | Status |
|---|---|---|---|
| 1 | `Query` | `Query { clauses: Vec<Clause> }` | Full |
| 2 | `CreateNodeTable` | `CreateNodeTable { name, columns, primary_key }` | Full |
| 3 | `CreateRelTable` | `CreateRelTable { name, from, to, columns }` | Full |
| 4 | `DropTable` | `DropTable { name }` | Full |
| 5 | `CopyFrom` | `CopyFrom { table_name, file_path, options }` | Full |
| 6 | `CopyTo` | `CopyTo { query, file_path, format, header }` | Full |
| 7 | `AlterTable` | `AlterTable { table_name, action }` | Partial (see below) |
| 8 | `CreateVectorIndex` | `CreateVectorIndex { index_name, table_name, column_name, metric, dimensions }` | Full |
| 9 | `CreateIndex` | `CreateIndex { index_type, index_name, table_name, variable, property, conflict_action }` | Full |
| 10 | `DropIndex` | `DropIndex { index_name, table_name }` | Full |
| 11 | `Union` | `UnionStatement { left, right, all }` | Full |
| 12 | `Merge` | `MergeStatement { patterns, on_create, on_match }` | Full |
| 13 | `StandaloneCall` | `StandaloneCall { function_name, args }` | Full |
| 14 | `CreateDml` | `CreateClause { patterns }` | Full |
| 15 | `Explain` | `ExplainStatement { statement, explain_type }` | Full |
| 16 | `CreateSequence` | `CreateSequence { name, if_not_exists, or_replace, start_with, increment, min_value, max_value, cycle }` | Full |
| 17 | `DropSequence` | `DropSequence { name, if_exists }` | Full |
| 18 | `CreateMacro` | `CreateMacro { name, positional_args, default_args, expression }` | Full |
| 19 | `ExportDatabase` | `ExportDatabase { file_path, options }` | Full |
| 20 | `ImportDatabase` | `ImportDatabase { file_path }` | Full |
| 21 | `Analyze` | `AnalyzeStatement { table_name }` | Full |
| 22 | `CreateFtsIndex` | `CreateFtsIndex { index_name, table_name, column_name, if_not_exists }` | Full |
| 23 | `Transaction` | `TransactionStatement { action }` | Full |
| 24 | `Extension` | `ExtensionStatement { action, name }` | Partial (see below) |
| 25 | `AttachDatabase` | `AttachDatabase { path, alias, options }` | Full |
| 26 | `DetachDatabase` | `DetachDatabase { alias }` | Full |
| 27 | `UseDatabase` | `UseDatabase { alias }` | Full |
| 28 | `LoadFrom` | `LoadFrom { path, options }` | Full |
| 29 | `CreateType` | `CreateType { name, type_name }` | Full |
| 30 | `CommentOnTable` | `CommentOnTable { table_name, comment }` | Full |
| 31 | `CreateGraph` | `CreateGraph { name, is_any }` | Full |
| 32 | `UseGraph` | `UseGraph { name }` | Full |
| 33 | `DropGraph` | `DropGraph { name }` | Full |

---

### 3. All Supported Clause Types

From `Clause` enum in `ast.rs`:

| Clause | AST Struct | Fields |
|---|---|---|
| `Match` | `MatchClause` | `patterns: Vec<Pattern>`, `fts_query: Option<FtsQuery>` |
| `OptionalMatch` | `OptionalMatchClause` | `patterns: Vec<Pattern>` |
| `Return` | `ReturnClause` | `expressions: Vec<ReturnItem>`, `distinct: bool` |
| `With` | `ReturnClause` | (same as Return) |
| `Where` | `WhereClause` | `expression: Expression` |
| `Create` | `CreateClause` | `patterns: Vec<Pattern>` |
| `Delete` | `DeleteClause` | `detach: bool`, `expressions: Vec<Expression>` |
| `Set` | `SetClause` | `items: Vec<SetItem>` |
| `Unwind` | `UnwindClause` | `expression: Expression`, `variable: String` |
| `Foreach` | `ForeachClause` | `variable, expression, clauses: Vec<Clause>` |

**IMPORTANT: `ReturnClause` has NO fields for `order_by`, `limit`, or `skip`/`offset`, even though the PEG grammar defines them.**

---

### 4. All Supported Expression Types

From `Expression` enum in `ast.rs`:

| Expression Variant | Description |
|---|---|
| `Constant(Constant)` | Null, Bool, Integer(i64), Float(f64), String |
| `Variable(String)` | Variable reference |
| `Parameter(String)` | `$param` parameter references |
| `PropertyAccess(Box<Expression>, String)` | `expr.prop` |
| `FunctionCall(String, Vec<Expression>)` | Function/macro invocations including `COUNT(*)` |
| `BinaryOp(BinaryOp, ...)` | `+`, `-`, `*`, `/`, `%`, `=`, `<>`, `<`, `>`, `<=`, `>=`, `AND`, `OR`, `XOR`, `\|\|` (concat), `IN`, `NOT IN`, `STARTS WITH`, `ENDS WITH`, `CONTAINS`, `LIKE` |
| `UnaryOp(UnaryOp, ...)` | `NOT`, unary `-`, `IS NULL`, `IS NOT NULL` |
| `List(Vec<Expression>)` | `[1, 2, 3]` |
| `Map(Vec<(String, Expression)>)` | `{key: value}` |
| `ExistsSubquery(Box<Query>)` | `EXISTS { MATCH ... }` |
| `Case(CaseExpr)` | `CASE [x] WHEN ... THEN ... [ELSE ...] END` |
| `Star` | `*` in `RETURN *` |
| `ListPredicate { quantifier, list, var_name, predicate }` | `ANY(x IN list WHERE pred)`, `ALL`, `NONE`, `SINGLE` |
| `Lambda { var_name, body }` | `x -> x + 1` |

**Binary operators**: Add, Subtract, Multiply, Divide, Modulo, Equal, NotEqual, LessThan, LessThanOrEqual, GreaterThan, GreaterThanOrEqual, And, Or, Xor, Concat, In, NotIn, StartsWith, EndsWith, Contains, Like

**Unary operators**: Not, Negate, IsNull, IsNotNull

**NOTE**: `BETWEEN` is parsed (grammar line 385) and desugared to `>= lower AND <= upper` at parse time (expression.rs lines 311-323).

---

### 5. Features in the PEG Grammar but NOT Connected to the AST

These rules exist in `cypher.pest` but the Rust parser code does not store their results in any AST struct:

| Grammar Rule | AST Field Missing | Impact |
|---|---|---|
| `order_by` under `return_clause` (line 334) | No `order_by` field on `ReturnClause` | **ORDER BY silently ignored** |
| `limit` under `return_clause` (line 334) | No `limit` field on `ReturnClause` | **LIMIT silently ignored** |
| `order_by` under `with_clause` (line 331) | Not parsed by DML parser | **ORDER BY in WITH silently ignored** |
| `limit` under `with_clause` (line 331) | Not parsed by DML parser | **LIMIT in WITH silently ignored** |
| `offset`/`SKIP` (line 348) | No `skip`/`offset` field | **SKIP silently ignored** |
| `optional_match_clause` with `where_clause?` (line 297) | `OptionalMatchClause` has no WHERE field | **WHERE on OPTIONAL MATCH not captured** |
| `using_fts_clause` | Only parsed on `MatchClause` | OK - handled |

---

### 6. Comparison: C++ ANTLR Features MISSING in Rust Parser

Based on the C++ ANTLR grammar (`src/antlr4/Cypher.g4`, 917 lines) and C++ parser headers:

#### 6a. DDL Statements Missing

| Feature | C++ ANTLR Rule | Rust Status |
|---|---|---|
| `DROP MACRO` | `DROP ... MACRO` (unified in `kU_Drop`) | **MISSING** (only `DropTable`, `DropSequence`) |
| `CREATE NODE TABLE IF NOT EXISTS` | `kU_IfNotExists` in `kU_CreateNodeTable` | **MISSING** |
| `CREATE NODE TABLE AS query` (CTAS) | `CREATE NODE TABLE name AS oC_Query` | **MISSING** |
| `CREATE REL TABLE GROUP` | `CREATE REL TABLE GROUP name` | **MISSING** |
| `CREATE REL TABLE IF NOT EXISTS` | `kU_IfNotExists` in `kU_CreateRelTable` | **MISSING** |
| `CREATE REL TABLE AS query` | `CREATE REL TABLE ... AS oC_Query` | **MISSING** |
| `ALTER TABLE ... ADD [IF NOT EXISTS] col type [DEFAULT expr]` | `kU_AddProperty` with `kU_Default` | **MISSING** (Rust: bare `ADD col type` only) |
| `ALTER TABLE ... ADD FROM a TO b` | `kU_AddFromToConnection` | **MISSING** |
| `ALTER TABLE ... DROP FROM a TO b` | `kU_DropFromToConnection` | **MISSING** |
| `ALTER TABLE ... DROP [IF EXISTS] col` | `kU_DropProperty` with `kU_IfExists` | **MISSING** |
| `DECIMAL(precision, scale)` data type | `kU_DecimalType` | **MISSING** |
| Constraints in `CREATE REL TABLE` | Additional identifier after properties | **MISSING** |

#### 6b. DML / Query Features Missing

| Feature | C++ ANTLR Rule | Rust Status |
|---|---|---|
| `ORDER BY` in `RETURN`/`WITH` | `oC_Order` | **PARSED BUT IGNORED** |
| `LIMIT` in `RETURN`/`WITH` | `oC_Limit` | **PARSED BUT IGNORED** |
| `SKIP`/`OFFSET` in `RETURN`/`WITH` | `oC_Skip` | **PARSED BUT IGNORED** |
| `MATCH ... HINT ...` (join hints) | `kU_Hint` / `kU_JoinNode` | **MISSING** |
| `CALL func() YIELD col1, col2` | `kU_InQueryCall` with `oC_YieldItems` | **MISSING** (Rust: plain `CALL` only) |
| `CALL var = expr` (assignment form) | `CALL SP SymbolicName SP? '=' SP? Expression` | **MISSING** |
| `COPY FROM (query)` | `kU_ScanSource :: '(' oC_Query ')'` | **MISSING** (Rust: file path only) |
| `COPY FROM table_function()` | `kU_ScanSource :: oC_FunctionInvocation` | **MISSING** |
| `COPY FROM table.column` | `kU_ScanSource :: Variable '.' SchemaName` | **MISSING** |
| `COPY table (col1, col2) FROM ...` | `kU_ColumnNames?` on `kU_CopyFrom` | **MISSING** |
| `COPY FROM ... BY COLUMN` | `kU_CopyFromByColumn` | **MISSING** |
| Multiple file paths in COPY FROM | `[ 'f1', 'f2', ... ]` or `GLOB('pattern')` | **MISSING** (single path only) |
| `COUNT { MATCH ... }` subquery | `oC_ExistCountSubquery` with `COUNT` | **MISSING** (only `EXISTS` supported) |
| Named paths: `p = (a)-[*]->(b)` | `oC_PatternPart :: Variable '=' ...` | **MISSING** |
| `LOAD WITH HEADERS (col type, ...) FROM ...` | `LOAD WITH HEADERS '(' columnDefinitions ')' FROM ...` | **MISSING** |
| `WHERE` on `OPTIONAL MATCH` | `optional_match_clause :: ... where_clause?` | **MISSING** (grammar supports it, Rust ignores it) |

#### 6c. Expression Features Missing

| Feature | C++ ANTLR Rule | Rust Status |
|---|---|---|
| `^` (power/exponentiation) operator | `oC_PowerOfExpression` | **MISSING** |
| `\|` (bitwise OR) operator | `kU_BitwiseOrOperatorExpression` | **MISSING** |
| `&` (bitwise AND) operator | `kU_BitwiseAndOperatorExpression` | **MISSING** |
| `>>` / `<<` (bit shift) operators | `kU_BitShiftOperatorExpression` | **MISSING** |
| `=~` (regex match) operator | `oC_RegularExpression` | **MISSING** |
| `expr[start..end]` list slicing | `oC_ListOperatorExpression` with `COLON`/`DOTDOT` | **MISSING** (only `expr[idx]` supported) |
| `expr[idx]` list indexing | `oC_ListOperatorExpression :: '[' expression ']'` | Captured but only in `postfix_expr` with `"[" ~ expression ~ "]"` |
| Struct literals `{name: value}` | `kU_StructLiteral` | **PARTIAL** (Rust maps use this syntax but typed as `Map`) |
| `ALL SHORTEST` / `WSHORTEST` / `TRAIL` / `ACYCLIC` path types | `kU_RecursiveType` | **MISSING** |
| Recursive path comprehension `[var, var \| WHERE ... \| proj1, proj2]` | `kU_RecursiveComprehension` | **MISSING** |

#### 6d. Extension / Admin Features Missing

| Feature | C++ ANTLR Rule | Rust Status |
|---|---|---|
| `UPDATE EXTENSION name` | `kU_UpdateExtension` | **MISSING** (only INSTALL/LOAD/UNINSTALL) |
| `FORCE INSTALL EXTENSION name FROM path` | `kU_InstallExtension` with `FORCE` and `FROM` | **MISSING** |
| `CREATE USER name WITH PASSWORD '...'` | `kU_CreateUser` | **MISSING** |
| `CREATE ROLE name` | `kU_CreateRole` | **MISSING** |
| `BEGIN TRANSACTION READ ONLY` | `kU_Transaction` with `READ ONLY` | **MISSING** |
| `ATTACH 'path' (DBTYPE xxx)` (required DBTYPE) | `kU_AttachDatabase` with mandatory `DBTYPE` | **PARTIAL** (Rust: generic options, no DBTYPE enforcement) |

#### 6e. Other Missing Features

| Feature | C++ ANTLR / Rust | Status |
|---|---|---|
| Multi-statement queries (`stmt; stmt; ...`) | `ku_Statements :: oC_Cypher (';' oC_Cypher)*` | **MISSING** (Rust: single statement only) |
| `ALTER TABLE ... RENAME TO` | `kU_RenameTable` | Supported |
| `ALTER TABLE ... RENAME col TO newcol` | `kU_RenameProperty` | Supported |
| `CALL func(arg1, arg2)` (function call form) | `kU_StandaloneCall` with `oC_FunctionInvocation` | Supported |

---

### 7. Comparison: Data Types Supported

**Rust parser** (primitive_type in grammar):
`STRING`, `INT64`, `INT32`, `INT16`, `INT8`, `UINT64`, `UINT32`, `UINT16`, `UINT8`, `BOOL`, `BOOLEAN`, `DOUBLE`, `FLOAT`, `BLOB`, `DATE`, `TIMESTAMP`, `INTERVAL`, `SERIAL`

Plus composite types: `MAP(K,V)`, `STRUCT(fields)`, `UNION(fields)`, array syntax `type[]`

**C++ parser additionally supports**: `DECIMAL(precision, scale)`.

---

### 8. Summary of Critical Gaps

**Highest impact** (features that work syntactically but silently produce wrong results):

1. **ORDER BY, LIMIT, SKIP in RETURN/WITH** -- The PEG grammar parses them, but the parser code **never stores them in the AST**. Queries with `ORDER BY`, `LIMIT`, or `SKIP` will parse successfully but these clauses will be silently discarded.

2. **WHERE on OPTIONAL MATCH** -- The grammar supports `OPTIONAL MATCH ... WHERE expr`, but `OptionalMatchClause` has no WHERE field.

**Medium impact** (common Cypher features):
3. Named path patterns (`p = (a)-[*]->(b)`)
4. `^` (exponentiation) operator
5. Bitwise operators (`|`, `&`, `>>`, `<<`)
6. Regex matching (`=~`)
7. List slicing (`list[1..3]`)
8. `COUNT { MATCH ... }` subquery
9. `CALL ... YIELD ...`
10. `DROP MACRO`

**Low impact** (less commonly used):
11. `CREATE NODE TABLE IF NOT EXISTS` / CTAS
12. `CREATE REL TABLE GROUP`
13. `UPDATE EXTENSION` / `FORCE INSTALL`
14. User/role management DDL
15. Read-only transaction syntax
16. `DECIMAL` type
17. Multi-statement support