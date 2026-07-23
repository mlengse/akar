# Parser/Binder/Processor Audit 2026-07-19

> **Date:** 2026-07-19
> **Scope:** Detailed audit of akar-parser, akar-binder, and akar-processor crates
> **Methodology:** File-by-file analysis of implementation completeness vs C++ reference

---

## 1. Parser Crate (`akar-parser/`)

### 1.1 Crate Overview

| Aspect | Details |
|---|---|
| **Location** | `akar-parser/src/` |
| **Parser engine** | `pest` v2 (PEG parser generator) |
| **Grammar file** | `cypher.pest` (477 lines) |
| **AST definitions** | `ast.rs` (604 lines) |
| **Parser code** | `parser/mod.rs`, `parser/expression.rs`, `parser/dml.rs`, `parser/ddl.rs` |
| **Tests** | `parser_test.rs` (753 lines, ~60 tests) |
| **Dependencies** | `pest`, `pest_derive` only |
| **`todo!()` / `unimplemented!()`** | **NONE** |

### 1.2 Supported Statement Types (33 variants)

| # | Statement | AST Struct | Status |
|---|-----------|-----------|--------|
| 1 | `Query` | `Query { clauses }` | Full |
| 2 | `CreateNodeTable` | `CreateNodeTable { name, columns, primary_key }` | Full |
| 3 | `CreateRelTable` | `CreateRelTable { name, from, to, columns }` | Full |
| 4 | `DropTable` | `DropTable { name }` | Full |
| 5 | `CopyFrom` | `CopyFrom { table_name, file_path, options }` | Full |
| 6 | `CopyTo` | `CopyTo { query, file_path, format, header }` | Full |
| 7 | `AlterTable` | `AlterTable { table_name, action }` | Partial |
| 8 | `CreateVectorIndex` | `CreateVectorIndex { index_name, table_name, column_name, metric, dimensions }` | Full |
| 9 | `CreateIndex` | `CreateIndex { index_type, index_name, table_name, variable, property, conflict_action }` | Full |
| 10 | `DropIndex` | `DropIndex { index_name, table_name }` | Full |
| 11 | `Union` | `UnionStatement { left, right, all }` | Full |
| 12 | `Merge` | `MergeStatement { patterns, on_create, on_match }` | Full |
| 13 | `StandaloneCall` | `StandaloneCall { function_name, args }` | Full |
| 14 | `CreateDml` | `CreateClause { patterns }` | Full |
| 15 | `Explain` | `ExplainStatement { statement, explain_type }` | Full |
| 16 | `CreateSequence` | `CreateSequence { name, ... }` | Full |
| 17 | `DropSequence` | `DropSequence { name, if_exists }` | Full |
| 18 | `CreateMacro` | `CreateMacro { name, positional_args, default_args, expression }` | Full |
| 19 | `ExportDatabase` | `ExportDatabase { file_path, options }` | Full |
| 20 | `ImportDatabase` | `ImportDatabase { file_path }` | Full |
| 21 | `Analyze` | `AnalyzeStatement { table_name }` | Full |
| 22 | `CreateFtsIndex` | `CreateFtsIndex { index_name, table_name, column_name, if_not_exists }` | Full |
| 23 | `Transaction` | `TransactionStatement { action }` | Full |
| 24 | `Extension` | `ExtensionStatement { action, name }` | Partial |
| 25 | `AttachDatabase` | `AttachDatabase { path, alias, options }` | Full |
| 26 | `DetachDatabase` | `DetachDatabase { alias }` | Full |
| 27 | `UseDatabase` | `UseDatabase { alias }` | Full |
| 28 | `LoadFrom` | `LoadFrom { path, options }` | Full |
| 29 | `CreateType` | `CreateType { name, type_name }` | Full |
| 30 | `CommentOnTable` | `CommentOnTable { table_name, comment }` | Full |
| 31 | `CreateGraph` | `CreateGraph { name, is_any }` | Full |
| 32 | `UseGraph` | `UseGraph { name }` | Full |
| 33 | `DropGraph` | `DropGraph { name }` | Full |

### 1.3 Supported Clause Types (10 variants)

| Clause | AST Struct | Fields |
|---|---|---|
| `Match` | `MatchClause` | `patterns`, `fts_query` |
| `OptionalMatch` | `OptionalMatchClause` | `patterns` |
| `Return` | `ReturnClause` | `expressions`, `distinct` |
| `With` | `ReturnClause` | (same as Return) |
| `Where` | `WhereClause` | `expression` |
| `Create` | `CreateClause` | `patterns` |
| `Delete` | `DeleteClause` | `detach`, `expressions` |
| `Set` | `SetClause` | `items` |
| `Unwind` | `UnwindClause` | `expression`, `variable` |
| `Foreach` | `ForeachClause` | `variable`, `expression`, `clauses` |

**🔴 CRITICAL: `ReturnClause` has NO fields for `order_by`, `limit`, or `skip`/`offset`** — ORDER BY/LIMIT/SKIP are silently discarded.

### 1.4 Supported Expression Types (13 variants)

| Expression | Description |
|---|---|
| `Constant(Constant)` | Null, Bool, Integer, Float, String |
| `Variable(String)` | Variable reference |
| `Parameter(String)` | `$param` references |
| `PropertyAccess(Box, String)` | `expr.prop` |
| `FunctionCall(String, Vec)` | Function invocations |
| `BinaryOp(BinaryOp, Box, Box)` | +, -, *, /, %, =, <>, <, >, <=, >=, AND, OR, XOR, \|\|, IN, NOT IN, STARTS WITH, ENDS WITH, CONTAINS, LIKE |
| `UnaryOp(UnaryOp, Box)` | NOT, unary -, IS NULL, IS NOT NULL |
| `List(Vec)` | `[1, 2, 3]` |
| `Map(Vec<(String, Expression)>)` | `{key: value}` |
| `ExistsSubquery(Box)` | `EXISTS { MATCH ... }` |
| `Case(CaseExpr)` | `CASE WHEN ... THEN ... END` |
| `Star` | `*` in `RETURN *` |
| `ListPredicate` | `ANY/ALL/NONE/SINGLE(x IN list WHERE pred)` |
| `Lambda` | `x -> x + 1` |

### 1.5 Grammar Features NOT Connected to AST

| Grammar Rule | AST Field Missing | Impact |
|---|---|---|
| `order_by` under `return_clause` | No `order_by` field on `ReturnClause` | 🔴 **ORDER BY silently ignored** |
| `limit` under `return_clause` | No `limit` field on `ReturnClause` | 🔴 **LIMIT silently ignored** |
| `order_by` under `with_clause` | Not parsed by DML parser | 🔴 **ORDER BY in WITH silently ignored** |
| `limit` under `with_clause` | Not parsed by DML parser | 🔴 **LIMIT in WITH silently ignored** |
| `offset`/`SKIP` | No `skip`/`offset` field | 🔴 **SKIP silently ignored** |
| `optional_match_clause` WHERE | `OptionalMatchClause` has no WHERE field | 🔴 **WHERE on OPTIONAL MATCH not captured** |

### 1.6 C++ Features MISSING in Rust Parser

#### DDL Missing

| Feature | C++ Status | Rust Status |
|---|---|---|
| `DROP MACRO` | Supported | **MISSING** |
| `CREATE NODE TABLE IF NOT EXISTS` | Supported | **MISSING** |
| `CREATE NODE TABLE AS query` (CTAS) | Supported | **MISSING** |
| `CREATE REL TABLE GROUP` | Supported | **MISSING** |
| `CREATE REL TABLE IF NOT EXISTS` | Supported | **MISSING** |
| `ALTER TABLE ... ADD [IF NOT EXISTS] col type [DEFAULT expr]` | Supported | **MISSING** |
| `ALTER TABLE ... ADD/DROP FROM a TO b` | Supported | **MISSING** |
| `DECIMAL(precision, scale)` | Supported | **MISSING** |

#### DML/Query Missing

| Feature | C++ Status | Rust Status |
|---|---|---|
| `ORDER BY` in RETURN/WITH | Supported | 🔴 **PARSED BUT IGNORED** |
| `LIMIT` in RETURN/WITH | Supported | 🔴 **PARSED BUT IGNORED** |
| `SKIP`/`OFFSET` in RETURN/WITH | Supported | 🔴 **PARSED BUT IGNORED** |
| `MATCH ... HINT` (join hints) | Supported | **MISSING** |
| `CALL func() YIELD col1, col2` | Supported | **MISSING** |
| Named paths `p = (a)-[*]->(b)` | Supported | **MISSING** |
| `COUNT { MATCH ... }` subquery | Supported | **MISSING** |
| Multi-statement (`stmt; stmt; ...`) | Supported | **MISSING** |

#### Expression Missing

| Feature | C++ Status | Rust Status |
|---|---|---|
| `^` (power) operator | Supported | **MISSING** |
| Bitwise operators (`\|`, `&`, `>>`, `<<`) | Supported | **MISSING** |
| `=~` (regex match) operator | Supported | **MISSING** |
| List slicing `expr[start..end]` | Supported | **MISSING** |
| Recursive path comprehension | Supported | **MISSING** |

### 1.7 Critical Parser Gaps Summary

**Highest impact (silently produce wrong results):**
1. ORDER BY, LIMIT, SKIP — parsed but discarded
2. WHERE on OPTIONAL MATCH — grammar supports it, AST ignores it

**Medium impact:**
3. Named path patterns
4. `^` (exponentiation)
5. Bitwise operators
6. Regex matching
7. List slicing
8. `COUNT { MATCH ... }`
9. `CALL ... YIELD`
10. `DROP MACRO`

---

## 2. Binder Crate (`akar-binder/`)

### 2.1 Crate Overview

| Aspect | Details |
|---|---|
| **Location** | `akar-binder/src/` |
| **Files** | 8 files, ~3,227 lines |
| **Key files** | `binder/mod.rs` (1,738 lines), `bound_statement.rs` (438 lines), `binder/dml.rs` (603 lines) |
| **`todo!()` / `unimplemented!()`** | **NONE** |
| **`unreachable!()`** | 4 occurrences (2 in `mod.rs`, 2 in `dml.rs`) |

### 2.2 Bound Statement Coverage (33 variants)

| Category | Statements | Status |
|---|---|---|
| **Queries** | MATCH, RETURN, WHERE, WITH, CREATE, DELETE, SET, UNWIND, FOREACH, OptionalMatch | ✅ |
| **DDL** | CREATE NODE/REL TABLE, DROP TABLE, ALTER TABLE | ✅ |
| **Indexes** | CREATE/DROP INDEX, CREATE VECTOR INDEX | ✅ |
| **Sequences** | CREATE/DROP SEQUENCE | ✅ |
| **FTS** | CREATE FTS INDEX | ✅ |
| **Graph** | CREATE/USE/DROP GRAPH | ✅ |
| **Database** | EXPORT/IMPORT DATABASE, ATTACH/DETACH/USE DATABASE | ✅ |
| **DML** | CREATE DML, MERGE, COPY FROM/TO | ✅ |
| **Misc** | EXPLAIN, ANALYZE, TRANSACTION, EXTENSION, STANDALONE_CALL, LOAD FROM, CREATE TYPE, CREATE MACRO, COMMENT ON | ✅ |
| **NOT Implemented** | ORDER BY, SKIP, LIMIT | 🔴 Parser only |

### 2.3 Expression Resolution

`resolve_expression()` handles:
- Constant, Variable, Parameter, PropertyAccess
- FunctionCall (20+ functions)
- BinaryOp, UnaryOp
- List, Map, ExistsSubquery, Case, Star
- ListPredicate, Lambda

### 2.4 Critical Binder Issues

1. **Property type resolution is heuristic-based:**
```rust
// Hardcoded name → type mappings
"name" => LogicalType::String,
"age" => LogicalType::Int64,
// ... instead of catalog lookup
```

2. **Code duplication between `mod.rs` and `dml.rs`:**
Both files define identical implementations of 14+ methods:
- `bind_query`, `bind_match`, `bind_pattern`, `bind_return`
- `bind_where`, `bind_match_create`, `bind_unwind`, `bind_foreach`
- `bind_optional_match`, `bind_set`, `bind_union`, `bind_merge`
- `bind_create_dml`, `bind_delete`

The `mod.rs` version is canonical; `dml.rs` appears to be a refactoring artifact.

3. **`ddl.rs` is structurally broken:**
File starts with `impl Binder {` but immediately has test assertions — contains parser-level tests, not binder implementation.

---

## 3. Planner Crate (`akar-planner/`)

### 3.1 Crate Overview

| Aspect | Details |
|---|---|
| **Location** | `akar-planner/src/` |
| **Files** | 4 files, ~2,295 lines |
| **Key files** | `planner.rs` (916 lines), `logical_operator.rs` (997 lines), `join_order.rs` (375 lines) |

### 3.2 Logical Operator Coverage (48 variants)

| Operator | Produced by Planner? | Notes |
|---|---|---|
| `ScanNode` | Yes (MATCH) | — |
| `Filter` | Yes (WHERE) | — |
| `Projection` | Yes (RETURN/WITH) | — |
| `HashJoin` | Via join_order | — |
| `CrossProduct` | Via join_order | — |
| `Aggregate` | Yes (DISTINCT) | Also optimizer for COUNT/SUM/etc |
| `Union` | Yes (BoundUnion) | — |
| `StandaloneCall` | Yes | — |
| `CopyFrom` | Yes | — |
| `Delete` | Yes (DELETE) | — |
| `Set` | Yes (SET) | — |
| `OptionalMatch` | Yes | — |
| `Unwind` | Yes | — |
| `Foreach` | Yes | — |
| `Merge` | Yes | — |
| `Explain` | Yes | — |
| `RecursiveExtend` | Yes (var-length) | — |
| All DDL variants | Yes | — |
| `OrderBy` | **No** | Not produced by planner |
| `Limit` | **No** | Not produced by planner |
| `TopK` | No | Optimizer fuses OrderBy+Limit |
| `Flatten` | No | Tree pass FactorizationRewriting |
| `SemiMasker` | No | Tree pass SIPOptimization |
| `Accumulate` | No | Tree pass AccHashJoinOptimization |
| `SemiJoin` | No | Not produced |
| `AntiJoin` | No | Not produced |
| `Intersect` | No | Not produced |

### 3.3 Missing Planner Features

1. **ORDER BY/LIMIT/SKIP never produced** — planner never creates `LogicalOrderBy`, `LogicalLimit`, or `LogicalSkip`

2. **AggregateDetection has no GROUP BY extraction** — only creates aggregates with empty `group_by`

3. **15+ statements produce empty plans:**
- `BoundTransaction`, `BoundExtension`, `BoundAttachDatabase`
- `BoundDetachDatabase`, `BoundUseDatabase`, `BoundLoadFrom`
- `BoundCreateType`, `BoundCommentOnTable`, `BoundCreateGraph`
- `BoundUseGraph`, `BoundDropGraph`, `BoundAnalyze`
- `BoundDropIndex`, `BoundCreateMacro`, `BoundCopyTo`

---

## 4. Optimizer Crate (`akar-optimizer/`)

### 4.1 Crate Overview

| Aspect | Details |
|---|---|
| **Location** | `akar-optimizer/src/` |
| **Files** | 19 files, ~3,295 lines |
| **Passes** | 22 (15 flat + 7 tree) |

### 4.2 Pass Inventory

#### Flat Passes (15)

| # | Pass | Purpose | Status |
|---|------|---------|--------|
| 1 | `RemoveUnnecessaryOperators` | Remove empty projections, tautology filters | ✅ |
| 2 | `FilterPushDown` | Move filters closer to scan | ✅ |
| 3 | `PredicatePushDown` | Merge Filter → ScanNode predicate | ✅ |
| 4 | `ProjectionPushDown` | Remove unused columns | ✅ |
| 5 | `ConstantFolding` | Pre-evaluate constants | ✅ |
| 6 | `AggregateDetection` | Detect aggregates in Projections | ⚠️ No GROUP BY extraction |
| 7 | `JoinOptimization` | Convert equi-join filters | ✅ |
| 8 | `TopKOptimization` | Fuse ORDER BY + LIMIT → TopK | 🔴 Dead code |
| 9 | `VectorSimilarityDetection` | Detect distance function + ORDER BY + LIMIT | ✅ |
| 10 | `ArtRangeScanDetection` | Detect PK range filter | ✅ |
| 11 | `LimitPushDown` | Push Limit below Filter/Projection | 🔴 Dead code |
| 12 | `CommonSubexpressionElimination` | Deduplicate expressions | ✅ |
| 13 | `OrderByPushDown` | Push ORDER BY below UNION ALL | ✅ (Ladybug) |
| 14 | `UnwindDedup` | Remove duplicate UNWIND | ✅ (Ladybug) |
| 15 | `CountRelTable` | Replace ScanRel+COUNT with CSR metadata | ✅ (Ladybug) |

#### Tree Passes (7)

| # | Pass | Purpose | Status |
|---|------|---------|--------|
| T1 | `FactorizationRewriting` | Insert Flatten for WCOJ | ✅ |
| T2 | `ForeignJoinPushDown` | Mark foreign table joins | ✅ |
| T3 | `AccHashJoinOptimization` | Wrap selective probe in Accumulate | ✅ |
| T4 | `SIPOptimization` | Inject SemiMasker for SIP | ✅ (has `println!()`) |
| T5 | `CorrelatedSubqueryUnnesting` | Wire ExpressionsScan to Accumulate | ✅ |
| T6 | `AggKeyDependency` | Remove redundant GROUP BY keys | ✅ |
| T7 | `CardinalityEstimation` | Annotate row counts | ✅ |

### 4.3 Dead Code Analysis

**TopKOptimization** — looks for `OrderBy + Limit` patterns, but planner never produces them. This pass is effectively dead code.

**LimitPushDown** — looks for `Limit` operators, but planner never produces them. This pass is effectively dead code.

**Root cause:** The planner never creates `LogicalOrderBy` or `LogicalLimit` operators from AST (because the AST doesn't store ORDER BY/LIMIT/SKIP fields).

---

## 5. Processor Crate (`akar-processor/`)

### 5.1 Crate Overview

| Aspect | Details |
|---|---|
| **Location** | `akar-processor/src/` |
| **Files** | ~30 files |
| **Physical operators** | 45 structs implementing `PhysicalOperatorExec` |
| **`todo!()` / `unimplemented!()`** | **NONE** in physical operators |
| **`unreachable!()`** | 1 occurrence in `expression_evaluator.rs` |

### 5.2 Physical Operator Inventory (45 structs)

| Category | Operators | Count |
|---|---|---|
| **Scan** | PhysicalScan, PhysicalScanRel, PhysicalVectorSimilarityScan, PhysicalArtIndexRangeScan, PhysicalIndexLookup, PhysicalPathPropertyProbe | 6 |
| **Filter/Projection** | PhysicalFilter, PhysicalProjection, PhysicalFlatten, PhysicalUnwind, PhysicalSemiMasker | 5 |
| **Sort/Limit/Aggregate** | PhysicalOrderBy, PhysicalTopK, PhysicalLimit, PhysicalSkip, PhysicalAggregate, PhysicalCountRelTable | 6 |
| **Join** | PhysicalHashJoin, PhysicalSemiJoin, PhysicalAntiJoin, PhysicalIntersect, PhysicalCrossProduct | 5 |
| **DML** | PhysicalSet, PhysicalDelete, PhysicalInsertNode, PhysicalInsertRel, PhysicalExtend, PhysicalMerge, PhysicalInsert, PhysicalCopyFrom, PhysicalBatchInsert | 9 |
| **Recursive** | PhysicalRecursiveExtend, PhysicalPackedExtend | 2 |
| **FTS** | PhysicalCreateFtsIndex, PhysicalFtsScan | 2 |
| **Misc** | PhysicalEmptyResult, PhysicalMultiplicityReducer, PhysicalUnion, PhysicalAccumulate, Partitioner, PhysicalExplain, PhysicalForeach, PhysicalStandaloneCall, PhysicalExtensionClause, PhysicalPrimaryKeyScan | 10 |

### 5.3 DDL No-Op Stubs (12 operators)

All return empty chunks without side effects in `map_ddl.rs`:

```
CreateNodeTable, CreateRelTable, DropTable, AlterTable,
CreateIndex, DropIndex, CreateVectorIndex, CreateSequence,
DropSequence, CreateDml, ExportDatabase, ImportDatabase
```

### 5.4 Partial Implementations

1. **ExpressionsScan** — returns empty `DataChunk`, correlated variable scanning not wired
2. **TableFunctionCall** — only `vector_similarity_scan` has handler; others error
3. **Union** — `PhysicalUnion` struct exists but `execute()` is no-op; logic handled inline in mapper

---

## 6. Workspace Inventory

### 6.1 Crates (32 total)

| # | Crate | Purpose |
|---|-------|---------|
| 1 | `akar-common` | Core types, type system, file system |
| 2 | `akar-storage` | Storage engine, tables, WAL, buffer manager |
| 3 | `akar-transaction` | Transaction manager, concurrency control |
| 4 | `akar-catalog` | Catalog, schema metadata, sequences, macros |
| 5 | `akar-parser` | SQL/Cypher parser, AST definitions |
| 6 | `akar-binder` | Semantic analysis, binding, type resolution |
| 7 | `akar-planner` | Query planner, logical operators, join ordering |
| 8 | `akar-optimizer` | Query optimizer passes |
| 9 | `akar-processor` | Query execution engine, physical operators |
| 10 | `akar-function` | Function registry, scalar/aggregate/table functions |
| 11 | `akar-graph` | Graph data structures |
| 12-26 | Extension crates | json, fts, vector, httpfs, duckdb, algo, neo4j, llm, sqlite, delta, iceberg, azure, postgres, unity-catalog |
| 27 | `akar-main` | Main entry point: Database, Connection, QueryResult |
| 28 | `akar-cli` | CLI |
| 29 | `akar-wasm` | WASM build target |
| 30 | `akar-migrate` | Database migration tool |
| 31 | `akar-c` | C bindings |

### 6.2 Type System

**LogicalTypeID:** 38 variants (Any, Node, Rel, RecursiveRel, Serial, Bool, Int64-8, UInt64-8, Int128, Double, Float, Date, Timestamp variants, Interval, Decimal, InternalID, UInt128, Json, Time, String, Blob, List, Array, Struct, Map, Union, Uuid)

**PhysicalTypeID:** 19 variants

**Statement:** 32 variants

**Expression:** 13 variants (+ ListPredicate, Lambda)

**LogicalOperator:** 48 variants

**BoundStatement:** 33 variants

### 6.3 Function Registry

| Category | Count |
|---|---|
| **Scalar functions** | ~209 names (including aliases) |
| **Aggregate functions** | 14 names |
| **Table functions** | 1 built-in + dynamic registration |

### 6.4 Connection API

| Method | Description |
|--------|-------------|
| `Connection::new(database)` | Create connection |
| `query(&self, query_str)` | Execute Cypher query |
| `prepare(&self, query_str)` | Prepare parameterized query |
| `execute(&self, prepared, params)` | Execute prepared statement |

### 6.5 Standalone Call Handlers (24 registered)

`show_tables`, `table_info`, `show_functions`, `show_indexes`, `show_sequences`, `show_macros`, `show_connection`, `db_version`, `catalog_version`, `current_setting`, `stats_info`, `storage_info`, `show_attached_databases`, `bm_info`, `file_info`, `free_space_info`, `disk_size_info`, `storage_version`, `show_loaded_extensions`, `show_official_extensions`, `clear_warnings`, `show_warnings`, `export_csv`, `export_parquet`

---

## 7. Structural Issues Summary

| Issue | Location | Severity |
|---|---|---|
| **Binder code duplication** | `mod.rs` + `dml.rs` (14+ identical methods) | 🟡 High |
| **`ddl.rs` is test code** | `binder/ddl.rs` (structurally broken) | 🟡 Medium |
| **ORDER BY/LIMIT/SKIP discarded** | `ast.rs` ReturnClause | 🔴 Critical |
| **Hardcoded type resolution** | `binder/mod.rs` | 🔴 Critical |
| **12 DDL no-op stubs** | `map_ddl.rs` | 🔴 Critical |
| **CSR adjacency stub** | `csr.rs` | 🔴 Critical |
| **TopK/LimitPushDown dead code** | Optimizer passes | 🟡 High |
| **No GROUP BY extraction** | AggregateDetection | 🟡 Medium |
| **15+ empty plan statements** | `planner.rs` | 🟡 Medium |
| **`println!()` in SIP pass** | Optimizer | 🟢 Low |
