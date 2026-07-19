# COMPLETE AUDIT: Rust Binder, Planner, and Optimizer Crates 19/07/2026

---

## CRATE 1: `kuzu-binder` (8 files, ~3227 lines)

**Location:** `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-binder\src\`

---

### 1. `lib.rs` (10 lines)
- **Purpose:** Crate root -- re-exports `Binder`, sub-modules `binder`, `bound_statement`, `confidential_statement_analyzer`, test module.
- **Statements handled:** (delegated to sub-modules)
- **Markers/Notes:** None
- **Key types:** `Binder` (re-exported)

---

### 2. `bound_statement.rs` (438 lines)
- **Purpose:** Defines all bound statement types (semantically resolved AST).
- **Statements handled / enum variants:**
  | Variant | Handled |
  |---|---|
  | `BoundQuery` | Yes |
  | `BoundStandaloneCall` | Yes |
  | `BoundCreateNodeTable` | Yes |
  | `BoundCreateRelTable` | Yes |
  | `BoundDropTable` | Yes |
  | `BoundCopyFrom` | Yes |
  | `BoundCopyTo` | Yes |
  | `BoundAlterTable` | Yes |
  | `BoundCreateVectorIndex` | Yes |
  | `BoundCreateIndex` | Yes |
  | `BoundDropIndex` | Yes |
  | `BoundUnion` | Yes |
  | `BoundMerge` | Yes |
  | `BoundCreateDml` | Yes |
  | `BoundExplain` | Yes |
  | `BoundCreateSequence` | Yes |
  | `BoundDropSequence` | Yes |
  | `BoundCreateMacro` | Yes |
  | `BoundExportDatabase` | Yes |
  | `BoundImportDatabase` | Yes |
  | `BoundCreateFtsIndex` | Yes |
  | `BoundAnalyze` | Yes |
  | `BoundTransaction` | Yes |
  | `BoundExtension` | Yes |
  | `BoundAttachDatabase` | Yes |
  | `BoundDetachDatabase` | Yes |
  | `BoundUseDatabase` | Yes |
  | `BoundLoadFrom` | Yes |
  | `BoundCreateType` | Yes |
  | `BoundCommentOnTable` | Yes |
  | `BoundCreateGraph` | Yes |
  | `BoundUseGraph` | Yes |
  | `BoundDropGraph` | Yes |

- **Markers/Notes:** None
- **Key types/structs:** `BoundStatement` (enum, 33 variants), `BoundQuery`, `BoundClause` (enum, 10 clause types), `BoundVariable`, `BoundMatchClause`, `BoundPattern`, `BoundEdgePattern`, `BoundReturnClause`, `BoundWhereClause`, `BoundSetClause`, `BoundDeleteClause`, `BoundUnwindClause`, `BoundForeachClause`, `BoundExpression`, `BoundSetItem`, `BoundDeleteItem`, `BoundFtsQuery`, `BoundCreateNodeTable`, `BoundCreateRelTable`, `BoundDropTable`, `BoundAlterTable`, `BoundCreateVectorIndex`, `BoundCreateIndex`, `BoundDropIndex`, `BoundStandaloneCall`, `BoundExportDatabase`, `BoundImportDatabase`, `BoundCreateSequence`, `BoundDropSequence`, `BoundCreateMacro`, `BoundCreateDml`, `BoundMerge`, `BoundCreateFtsIndex`, `BoundCreateType`, `BoundCommentOnTable`, `BoundCreateGraph`, `BoundUseGraph`, `BoundDropGraph`, `BoundTransaction`, `BoundExtension`, `BoundAttachDatabase`, `BoundDetachDatabase`, `BoundUseDatabase`, `BoundLoadFrom`, `BoundAnalyze`, `BoundCopyFrom`, `BoundCopyTo`, `BoundUnion`, `BoundExplain`

---

### 3. `confidential_statement_analyzer.rs` (81 lines)
- **Purpose:** String-level detection of confidential CALL options (S3, GCS, Azure secrets).
- **Statements handled:** Only `CALL <confidential_option> = '...'` -- pure string scan, no parser involvement.
- **Markers/Notes:** None
- **Key types:** `is_confidential_call(&str) -> bool`, static `CONFIDENTIAL_OPTIONS` set

---

### 4. `binder_test.rs` (229 lines)
- **Purpose:** Integration tests for the binder.
- **Tests:** 16 tests covering:
  - `test_bind_create_node_table`
  - `test_bind_drop_table`
  - `test_bind_match_existing_table`
  - `test_bind_match_nonexistent_table`
  - `test_bind_rel_pattern`
  - `test_bind_where_boolean`
  - `test_bind_duplicate_variable`
  - `test_bind_invalid_type`
  - `test_bind_empty_table_name`
  - `test_bind_create_rel_table`
  - `test_bind_function_return_type`
  - `test_bind_sequence_function_return_type`
  - `test_bind_property_type_resolution`
  - `test_bind_complex_where`
- **Markers/Notes:** Line 139: "Note: multiple MATCH clauses not yet supported in grammar"

---

### 5. `binder/mod.rs` (1738 lines)
- **Purpose:** Core binder implementation -- resolves all statement types against the catalog.
- **Statements handled (via `bind()`):**
  1. `Query` → `bind_query()` -- MATCH, RETURN, WHERE, CREATE, DELETE, SET, UNWIND, FOREACH, OptionalMatch, WITH
  2. `CreateNodeTable` → `bind_create_node_table()`
  3. `CreateRelTable` → `bind_create_rel_table()`
  4. `DropTable` → `bind_drop_table()`
  5. `CopyFrom` → `bind_copy_from()` (with CSV header validation)
  6. `CopyTo` → `bind_copy_to()`
  7. `AlterTable` → `bind_alter_table()` (AddColumn, DropColumn, RenameColumn, RenameTable)
  8. `CreateVectorIndex` → `bind_create_vector_index()`
  9. `CreateIndex` → `bind_create_index()`
  10. `DropIndex` → `bind_drop_index()`
  11. `Union` → `bind_union()`
  12. `Merge` → `bind_merge()`
  13. `StandaloneCall` → `bind_standalone_call()`
  14. `CreateDml` → `bind_create_dml()`
  15. `Explain` → `bind_explain()`
  16. `CreateSequence` → `bind_create_sequence()`
  17. `DropSequence` → `bind_drop_sequence()`
  18. `CreateMacro` → `bind_create_macro()`
  19. `ExportDatabase` → `bind_export_database()`
  20. `ImportDatabase` → `bind_import_database()`
  21. `Analyze` → `bind_analyze()`
  22. `CreateFtsIndex` → `bind_create_fts_index()` (creates 3 macro tables)
  23. `Transaction` → `bind_transaction()`
  24. `Extension` → `bind_extension()`
  25. `AttachDatabase` → `bind_attach_database()`
  26. `DetachDatabase` → `bind_detach_database()`
  27. `UseDatabase` → `bind_use_database()`
  28. `LoadFrom` → `bind_load_from()`
  29. `CreateType` → `bind_create_type()`
  30. `CommentOnTable` → `bind_comment_on_table()`
  31. `CreateGraph` → `bind_create_graph()`
  32. `UseGraph` → `bind_use_graph()`
  33. `DropGraph` → `bind_drop_graph()`
- **Expression resolution:** `resolve_expression()` handles: Constant, Variable, Parameter, PropertyAccess, FunctionCall (20+ functions), BinaryOp, UnaryOp, List, Map, ExistsSubquery, Case, Star, ListPredicate, Lambda
- **Inline property conversion:** MATCH/CREATE inline properties (`{name: 'Alice'}`) get converted to implicit WHERE clauses
- **Markers/Notes:**
  - Line 3: `#![allow(clippy::collapsible_if, clippy::never_loop)]`
  - Line 1217: "Note: CALL create_fts_index is superseded by the DDL `CREATE FTS INDEX` statement."
  - `unreachable!()` at lines 1148, 1152 (in `bind_union`)
- **Missing/Pending:**
  - Property type resolution uses hardcoded heuristics (`name` → String, `age` → Int64, etc.) -- not true catalog lookup for types
  - Lambda body variable binding is "deferred to the evaluator"
  - `bind_create_dml` does not use the `_variables` parameter
  - No ORDER BY, SKIP, LIMIT clause binding (these are in parser but binder treats them as pass-through through the query structure)

---

### 6. `binder/helpers.rs` (56 lines)
- **Purpose:** Shared helper functions reused across binder modules.
- **Functions:** `resolve_set_items()`, `expr_to_debug_string()`
- **Markers/Notes:** None

---

### 7. `binder/dml.rs` (603 lines)
- **Purpose:** DML binding methods impl for Binder (duplicate/broken out from mod.rs).
- **Contains:** `bind_query`, `bind_match`, `bind_pattern`, `bind_return`, `bind_where`, `bind_match_create`, `bind_unwind`, `bind_foreach`, `bind_optional_match`, `bind_set`, `bind_union`, `bind_merge`, `bind_create_dml`, `bind_delete`
- **Markers/Notes:** `unreachable!()` at lines 503, 507 (in `bind_union` panics if children aren't BoundQuery)
- **Note:** This file has substantial duplication with `mod.rs` -- both files define the same methods (e.g., `bind_query`, `bind_match`, etc.). The `mod.rs` version seems to be the canonical one used via `binder::Binder`, while `dml.rs` defines `impl Binder` with identical logic. This is likely a refactoring artifact where code was being split out.

---

### 8. `binder/ddl.rs` (757 lines)
- **Purpose:** DDL binding methods and extensive parsing tests.
- **Structure:** Contains only test functions (starting with `#[test]`) and two helper functions (`parse_create_fts_index`, `parse_using_fts_clause`). The file begins with `impl Binder {` but immediately has `assert_eq!(...)` code that is test code, not valid impl methods. This appears to be a file that was incorrectly structured -- it contains parser-level tests for DDL, sequences, FTS, list predicates, parameters, COPY, EXPLAIN, etc.
- **Markers/Notes:** None beyond the structural confusion

---

### Summary of Binder Coverage

| Category | Statements | Status |
|---|---|---|
| **Queries** | MATCH, RETURN, WHERE, WITH, CREATE (pattern), DELETE, SET, UNWIND, FOREACH, OptionalMatch | **Implemented** |
| **DDL** | CREATE NODE TABLE, CREATE REL TABLE, DROP TABLE, ALTER TABLE | **Implemented** |
| **Indexes** | CREATE [ART/HASH] INDEX, DROP INDEX, CREATE VECTOR INDEX | **Implemented** |
| **Sequences** | CREATE SEQUENCE, DROP SEQUENCE | **Implemented** |
| **FTS** | CREATE FTS INDEX, USING FTS INDEX | **Implemented** |
| **Graph** | CREATE GRAPH, USE GRAPH, DROP GRAPH | **Implemented** |
| **Database** | EXPORT DATABASE, IMPORT DATABASE, ATTACH/DETACH/USE DATABASE | **Implemented** |
| **DML** | CREATE DML (node creation), MERGE, COPY FROM, COPY TO | **Implemented** |
| **Misc** | EXPLAIN, ANALYZE, TRANSACTION, EXTENSION, STANDALONE_CALL, LOAD FROM, CREATE TYPE, CREATE MACRO, COMMENT ON TABLE | **Implemented** |
| **Aggregate** | COUNT, SUM, MIN, MAX, AVG, nextval/currval | **Recognized in expressions** |
| **NOT Implemented** | ORDER BY, SKIP, LIMIT (parser has them, binder passes through) | - |
| **Simplification** | Property type resolution uses hardcoded name-based heuristics | - |

---

## CRATE 2: `kuzu-planner` (4 files, ~2295 lines)

**Location:** `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-planner\src\`

---

### 1. `lib.rs` (7 lines)
- **Purpose:** Crate root -- re-exports `QueryPlanner`, sub-modules `join_order`, `logical_operator`, `planner`.

---

### 2. `planner.rs` (916 lines)
- **Purpose:** Converts bound statements into logical operator plans.
- **Statements handled in `plan()`:**
  1. `BoundQuery` → `plan_query()` (core query planning)
  2. `BoundCopyFrom` → `plan_copy_from()`
  3. `BoundUnion` → `plan_union()` (wraps left/right in projection roots)
  4. `BoundMerge` → `plan_merge()` (ON MATCH SET + ON CREATE SET as LogicalSet sub-ops)
  5. `BoundExplain` → `plan_explain()` (plans inner, wraps in LogicalExplain)
  6. `BoundCreateNodeTable`, `BoundCreateRelTable`, `BoundDropTable`, `BoundAlterTable`
  7. `BoundCreateIndex`, `BoundDropIndex`, `BoundCreateVectorIndex`
  8. `BoundCreateSequence`, `BoundDropSequence`
  9. `BoundCreateDml`, `BoundExportDatabase`, `BoundImportDatabase`
  10. `BoundCreateFtsIndex` → LogicalCreateFtsIndex
  11. `BoundStandaloneCall` → LogicalStandaloneCall
  12. `_` (default catch-all) → `Ok(Vec::new())` -- **Many bound statements produce empty plans:**
      - `BoundTransaction`
      - `BoundExtension`
      - `BoundAttachDatabase`
      - `BoundDetachDatabase`
      - `BoundUseDatabase`
      - `BoundLoadFrom`
      - `BoundCreateType`
      - `BoundCommentOnTable`
      - `BoundCreateGraph`
      - `BoundUseGraph`
      - `BoundDropGraph`
      - `BoundAnalyze`
      - `BoundDropIndex`
      - `BoundCreateMacro`
      - `BoundCopyTo`

- **Query planning (`plan_query`):** Builds operator pipeline from bound clauses:
  - `BoundMatch` → ScanNode(s) + RecursiveExtend (for var-length paths) or Extend (for regular rel patterns)
  - Multiple scans → join tree via `build_join_tree` (greedy) or cross product
  - `BoundWhere` → filter expression collected, applied after joins
  - `BoundReturn` → Projection (topmost), DISTINCT → Aggregate with group-by
  - `BoundWith` → Projection pushed to `delete_exprs`
  - `BoundOptionalMatch` → full pipeline split into left/right with LogicalOptionalMatch
  - `BoundDelete` → LogicalDelete per item
  - `BoundUnwind` → LogicalUnwind
  - `BoundSet` → LogicalSet per item
  - `BoundCreate` → LogicalCreateNode + LogicalCreateRel
  - `BoundForeach` → LogicalForeach with sub-plans
  - **Implicit WHERE** from inline properties is already handled by the binder

- **Markers/Notes:** None
- **Tests:** 10 tests covering plan structure for match-return, return-only, unwind, match-where-return, DDL, scan node fields, rel patterns, plan ordering

- **Missing operators that are defined in `logical_operator.rs` but never produced by the planner:**
  - `ScanRel` (superseded by `Extend` for rel patterns, but the operator exists)
  - `VectorSimilarityScan` (detected later by optimizer pass)
  - `HashJoin`, `CrossProduct` (produced by join_order module, not directly)
  - `OrderBy`, `Limit`, `TopK` (optimizer pass converts from... nothing produced by planner currently)
  - `Aggregate` (only produced for DISTINCT)
  - `Flatten`, `SemiMasker`, `Accumulate`, `ExpressionsScan` (optimizer passes)
  - `SemiJoin`, `AntiJoin`, `Intersect` (not produced)
  - `ArtIndexRangeScan` (optimizer pass)
  - `BatchInsert`, `IndexLookup`
  - `EmptyResult`, `MultiplicityReducer`, `Skip`, `Insert`, `ExtensionClause`, `PathPropertyProbe`, `Partitioner`, `CountRelTable`

---

### 3. `logical_operator.rs` (997 lines)
- **Purpose:** Defines all logical operator types and their tree traversal methods.
- **`LogicalOperator` enum -- 48 variants:**

| Variant | Produced by Planner? | Notes |
|---|---|---|
| `ScanNode` | Yes (MATCH) | - |
| `ScanRel` | No | Only in OptionalMatch, optimizer replaces with CountRelTable |
| `VectorSimilarityScan` | No | Optimizer pass VectorSimilarityDetection |
| `ArtIndexRangeScan` | No | Optimizer pass ArtRangeScanDetection |
| `Filter` | Yes (WHERE clause) | - |
| `Projection` | Yes (RETURN/WITH) | - |
| `HashJoin` | Via join_order | - |
| `CrossProduct` | Via join_order | - |
| `OrderBy` | No | Not produced by planner |
| `Limit` | No | Not produced by planner |
| `TopK` | No | Optimizer fuses OrderBy+Limit |
| `Aggregate` | Yes (DISTINCT) | Also optimizer AggregateDetection for COUNT/SUM/etc |
| `Union` | Yes (BoundUnion) | - |
| `Flatten` | No | Tree pass FactorizationRewriting |
| `TableFunctionCall` | No | Defined but not produced |
| `StandaloneCall` | Yes | - |
| `CopyFrom` | Yes | - |
| `BatchInsert` | No | Defined but not produced |
| `IndexLookup` | No | Defined but not produced |
| `Delete` | Yes (DELETE clause) | - |
| `Set` | Yes (SET clause) | - |
| `OptionalMatch` | Yes | - |
| `Unwind` | Yes | - |
| `Foreach` | Yes | - |
| `Merge` | Yes | - |
| `SemiJoin` | No | Not produced |
| `AntiJoin` | No | Not produced |
| `Intersect` | No | Not produced |
| `Explain` | Yes | - |
| `RecursiveExtend` | Yes (var-length patterns) | - |
| `SemiMasker` | No | Tree pass SIPOptimization |
| `Accumulate` | No | Tree pass AccHashJoinOptimization |
| `ExpressionsScan` | No | Defined for correlated subqueries |
| `CountRelTable` | No | Ladybug optimizer pass |
| `Partitioner` | No | Defined but not produced |
| `PathPropertyProbe` | No | Defined but not produced |
| `CreateNodeTable` | Yes | - |
| `CreateRelTable` | Yes | - |
| `DropTable` | Yes | - |
| `AlterTable` | Yes | - |
| `CreateIndex` | Yes | - |
| `DropIndex` | Yes | - |
| `CreateVectorIndex` | Yes | - |
| `CreateSequence` | Yes | - |
| `DropSequence` | Yes | - |
| `CreateDml` | Yes | - |
| `CreateNode` | Yes (CREATE clause) | - |
| `CreateRel` | Yes (CREATE clause) | - |
| `Extend` | Yes (rel patterns) | Combines ScanRel+ScanNode(dest) |
| `ExportDatabase` | Yes | - |
| `ImportDatabase` | Yes | - |
| `CreateFtsIndex` | Yes | - |
| `FtsScan` | No | Defined but not produced |
| `EmptyResult` | No | Defined |
| `MultiplicityReducer` | No | Defined |
| `Skip` | No | Defined |
| `Insert` | No | Defined |
| `ExtensionClause` | No | Defined |

- **Key structs:** `LogicalScanNode`, `LogicalScanRel`, `LogicalFilter`, `LogicalProjection`, `LogicalHashJoin` (with `push_down_eligible`), `LogicalSemiJoin`, `LogicalAntiJoin`, `LogicalIntersect`, `LogicalRecursiveExtend` (with `weight_property`, `cost_output_name`, `semantic`), `LogicalOptionalMatch`, `LogicalSet`, `LogicalDelete`, `LogicalCopyFrom`, `LogicalBatchInsert`, `LogicalIndexLookup`, `LogicalForeach`, `LogicalMerge`, `LogicalTopK`, `LogicalLimit`, `LogicalOrderBy`, `LogicalAggregate`, `LogicalUnion`, `LogicalFlatten`, `LogicalUnwind`, `LogicalExplain`, `LogicalExpressionsScan`, `LogicalAccumulate`, `LogicalSemiMasker`, `LogicalCountRelTable`, `LogicalPartitioner`, `LogicalPathPropertyProbe`, `LogicalStandaloneCall`, `LogicalTableFunctionCall`, all DDL structs, `LogicalCreateNode`, `LogicalCreateRel`, `LogicalExtend`, `LogicalFtsScan`, `LogicalArtIndexRangeScan`, `LogicalVectorSimilarityScan`, `LogicalEmptyResult`, `LogicalMultiplicityReducer`, `LogicalSkip`, `LogicalInsert`, `LogicalExtensionClause`
- **Tree traversal:** `visit_bottom_up()`, `children_mut()`, `children()`, `cardinality()`, `set_cardinality()` -- all exhaustively match all 48 variants.
- **Markers/Notes:** None

---

### 4. `join_order.rs` (375 lines)
- **Purpose:** Greedy join order enumeration for query patterns.
- **Algorithm:**
  - `build_join_tree()`: Greedy heuristic -- starts with first scan, joins each subsequent scan
  - For each scan, looks for equality join conditions in the filter expression (`extract_join_conditions`)
  - If join condition found → `HashJoin`, otherwise → `CrossProduct`
  - `extract_join_conditions()` walks BinaryOp trees looking for `Equal` comparisons between different variables
- **Key types:** `JoinPlan` enum (Leaf, HashJoin, CrossProduct)
- **Key functions:** `build_join_tree()`, `flatten_join_plan()`, `flatten_plan()`
- **Markers/Notes:** None
- **Tests:** 6 tests covering single scan, cross product, join condition extraction, AND conditions, variable alias extraction, plan flattening

---

## CRATE 3: `kuzu-optimizer` (19 files, ~3295 lines)

**Location:** `C:\Users\anjan\dev\memory\kuzu\kuzu-core\kuzu-optimizer\src\`

---

### 1. `lib.rs` (10 lines)
- **Purpose:** Crate root -- re-exports `Optimizer`, sub-modules `join_order`, `optimizer`, `passes`, test module.

---

### 2. `optimizer.rs` (199 lines)
- **Purpose:** Chains optimization passes (14 flat + 6/7 tree).
- **Pass registration order:**

| # | Pass Name | Type | Purpose |
|---|---|---|---|
| 1 | `RemoveUnnecessaryOperators` | Flat | Removes empty projections, tautology filters |
| 2 | `FilterPushDown` | Flat | Moves filters closer to scan nodes |
| 3 | `PredicatePushDown` | Flat | Merges Filter predicates into ScanNode.predicate |
| 4 | `ProjectionPushDown` | Flat | Removes unused columns from ScanNode |
| 5 | `ConstantFolding` | Flat | Pre-evaluates constant sub-expressions |
| 6 | `AggregateDetection` | Flat | Detects aggregate functions in Projections → LogicalAggregate |
| 7 | `JoinOptimization` | Flat | Converts equi-join filters to join conditions, reorders |
| 8 | `TopKOptimization` | Flat | Fuses ORDER BY + LIMIT → LogicalTopK |
| 9 | `VectorSimilarityDetection` | Flat | Detects distance function + ORDER BY + LIMIT → VectorSimilarityScan |
| 10 | `ArtRangeScanDetection` | Flat | Detects PK range filter → ArtIndexRangeScan |
| 11 | `LimitPushDown` | Flat | Pushes Limit below Filter/Projection |
| 12 | `CommonSubexpressionElimination` | Flat | Deduplicates expressions in Projection |
| 13 | `OrderByPushDown` | Flat | Pushes ORDER BY below UNION ALL (Ladybug) |
| 14 | `UnwindDedup` | Flat | Removes duplicate consecutive UNWIND (Ladybug) |
| 15 | `CountRelTable` | Flat | Replaces ScanRel+COUNT with CSR metadata (Ladybug) |
| T1 | `FactorizationRewriting` | Tree | Inserts LogicalFlatten for WCOJ factorization |
| T2 | `ForeignJoinPushDown` | Tree | Marks HashJoins with foreign tables as push-down eligible |
| T3 | `AccHashJoinOptimization` | Tree | Wraps selective probe sides in Accumulate |
| T4 | `SIPOptimization` | Tree | Injects SemiMasker for Sideways Information Passing |
| T5 | `CorrelatedSubqueryUnnesting` | Tree | Wires ExpressionsScan to outer Accumulate |
| T6 | `AggKeyDependency` | Tree | Removes redundant GROUP BY keys |
| T7 | `CardinalityEstimation` | Tree | Annotates operators with estimated row counts |

- **`with_stats()` variant:** Same passes but with storage-backed cardinality estimation.
- **Markers/Notes:**
  - `#[allow(dead_code)]` annotation in code
  - `println!()` (debug print) in SIP pass
  - Optimization pass names are hardcoded strings
- **Tests:** 3 tests (pass registration count = 22, empty plan, preserves valid plan)

---

### 3. `join_order.rs` (711 lines)
- **Purpose:** Cardinality-aware join reordering (two algorithms).
- **Greedy reorder (`reorder_joins_greedy`):** Collects leaf scans, sorts by cardinality ascending, rebuilds join tree, appends join conditions as filters.
- **DP-based bushy join (`reorder_joins_dp_bushy`):** Dynamic programming over bitmask subsets, finds optimal bushy join tree minimizing cost, limited to <= 15 relations. Uses `EQUALITY_PREDICATE_SELECTIVITY = 0.1`. Cross products penalized with +1e9 cost.
- **Key types:** `DpState` (cost, cardinality, left_mask, right_mask)
- **Key functions:** `collect_scans_sorted()`, `extract_join_conditions_from_tree()`, `reorder_joins_greedy()`, `reorder_joins_dp_bushy()`, `build_optimal_tree()`
- **Markers/Notes:** None
- **Tests:** 6 tests (collect scans, sort by cardinality, no join needed, extract join conditions, same var not join, scan alias, DP prefers join over cross product)

---

### 4. `passes/mod.rs` (36 lines)
- **Purpose:** Trait definitions and module organization.
- **Traits:**
  - `OptimizationPass` -- `fn name(&self) -> &str`, `fn apply(&self, operators: &[LogicalOperator]) -> Vec<LogicalOperator>`
  - `TreeOptimizationPass` -- `fn name(&self) -> &str`, `fn apply_tree(&self, root: &mut LogicalOperator)`

---

### 5. `passes/flat/mod.rs` (25 lines)
- **Purpose:** Re-exports all flat optimization passes.
- **Modules:**
  - `aggregate_detection` → `AggregateDetection`
  - `art_range_scan` → `ArtRangeScanDetection`
  - `constant_folding` → `ConstantFolding`
  - `filter_pushdown` → `FilterPushDown`
  - `predicate_pushdown` → `PredicatePushDown`
  - `join_optimization` → `JoinOptimization`
  - `ladybug` → `CountRelTable`, `OrderByPushDown`, `UnwindDedup`
  - `projection_pushdown` → `ProjectionPushDown`
  - `scan_ops` → `CommonSubexpressionElimination`, `LimitPushDown`, `RemoveUnnecessaryOperators`
  - `top_k` → `TopKOptimization`
  - `vector_similarity` → `VectorSimilarityDetection`

---

### 6. `passes/flat/aggregate_detection.rs` (74 lines)
- **Purpose:** Detects aggregate functions (COUNT, SUM, AVG, MIN, MAX, STDDEV, VARIANCE, COLLECT) in Projection expressions and replaces with LogicalAggregate.
- **Limitation:** GROUP BY keys are set to empty Vec -- only handles simple `RETURN COUNT(*)` patterns. No GROUP BY extraction.
- **Markers/Notes:** None

---

### 7. `passes/flat/art_range_scan.rs` (168 lines)
- **Purpose:** Detects `ScanNode + Filter(pk >= lower AND pk < upper)` and rewrites to `ArtIndexRangeScan`.
- **Handles:** Single/comparison bounds (`>=`, `>`, `<=`, `<`, `=`), AND-combined bounds, property on either side of comparison.
- **Converts** parser `Constant` to runtime `Value`.
- **Markers/Notes:** None

---

### 8. `passes/flat/constant_folding.rs` (203 lines)
- **Purpose:** Pre-evaluates constant sub-expressions at optimization time.
- **Folding supported:**
  - Integer: `+`, `-`, `*`, `/` (guarded against /0), `%`, `==`, `!=`, `<`, `<=`, `>`, `>=`
  - Float: `+`, `-`, `*`, `/` (guarded against /0.0), `==` (with epsilon), `!=`, `<`, `<=`, `>`, `>=`
  - Boolean: `AND`, `OR`, `XOR`, `==`, `!=`
  - String: `CONCAT` / `+`
  - Unary: `Negate` (int/float), `Not` (bool)
  - Recursive: into PropertyAccess, FunctionCall, List, Map, Case, ListPredicate, Lambda, ExistsSubquery
  - EXISTS subqueries: folds expressions inside the WHERE clause of the subquery
- **Markers/Notes:** None

---

### 9. `passes/flat/filter_pushdown.rs` (112 lines)
- **Purpose:** Moves Filter operators closer to their ScanNode sources.
- **Algorithm:** Tracks current scan, if a filter references only that scan's alias, folds it into `scan.predicate`; otherwise flushes pending filters before the scan.
- **Markers/Notes:** None

---

### 10. `passes/flat/projection_pushdown.rs` (126 lines)
- **Purpose:** Removes unused columns from ScanNode operators based on what Projection and Filter expressions reference.
- **Also collects references from:** CreateRel, CreateNode, Set, Unwind operators.
- **Markers/Notes:** None

---

### 11. `passes/flat/predicate_pushdown.rs` (44 lines)
- **Purpose:** Merges adjacent `Filter` + `ScanNode` pairs by moving the filter expression into `ScanNode.predicate`. Only merges if `scan.predicate.is_none()`.
- **Markers/Notes:** None

---

### 12. `passes/flat/join_optimization.rs` (65 lines)
- **Purpose:** First tries DP-based bushy join reordering, then removes equi-join filter conditions.
- **`is_join_condition()`:** Returns true if expression is `var_a.prop = var_b.prop` with different variables.
- **Markers/Notes:** None

---

### 13. `passes/flat/top_k.rs` (63 lines)
- **Purpose:** Fuses `ORDER BY + LIMIT` into `LogicalTopK`. Also handles `ORDER BY + Projection + LIMIT` → `Projection + TopK`.
- **Markers/Notes:** None

---

### 14. `passes/flat/vector_similarity.rs` (164 lines)
- **Purpose:** Detects `ScanNode + Filter(distance_fn) + [Projection] + OrderBy + Limit` and rewrites to `VectorSimilarityScan`.
- **Recognized functions:** `cosine_similarity`, `euclidean_distance`, `l2_distance`, `dot_product`
- **Extracts query vector** from literal list in function args.
- **Limitation:** `index_name` is set to empty string ("resolved at execution").
- **Markers/Notes:** None

---

### 15. `passes/flat/scan_ops.rs` (139 lines)
- **Purpose:** Three passes:
  - `RemoveUnnecessaryOperators`: Removes empty ScanNodes, empty Projections, tautology Filters (`true`, `1=1`).
  - `LimitPushDown`: Swaps `Limit + Filter` → `Filter + Limit` and `Limit + Projection` → `Projection + Limit`.
  - `CommonSubexpressionElimination`: Deduplicates repeated expressions in Projection.
- **Markers/Notes:** None

---

### 16. `passes/flat/ladybug.rs` (139 lines)
- **Purpose:** Three Ladybug-specific passes:
  - `OrderByPushDown`: Pushes `ORDER BY` below `UNION ALL` by wrapping each union child with the ORDER BY.
  - `UnwindDedup`: Merges consecutive UNWIND operators on the same list (uses debug string as dedup key).
  - `CountRelTable`: Detects `ScanRel + Aggregate(COUNT)` and replaces ScanRel with `CountRelTable` (CSR metadata).
- **Markers/Notes:** None

---

### 17. `passes/tree/mod.rs` (17 lines)
- **Purpose:** Re-exports all tree optimization passes.
- **Modules:**
  - `acc_hash_join` → `AccHashJoinOptimization`
  - `agg_key_dep` → `AggKeyDependency`
  - `cardinality` → `CardinalityEstimation`
  - `factorization` → `FactorizationRewriting`
  - `foreign_join` → `ForeignJoinPushDown`
  - `sip` → `SIPOptimization`
  - `subquery_unnesting` → `CorrelatedSubqueryUnnesting`

---

### 18. `passes/tree/acc_hash_join.rs` (74 lines)
- **Purpose:** When a HashJoin's probe side has filters, wraps it in an `Accumulate` operator to enable build-side semi-mask filtering.
- **Markers/Notes:** None

---

### 19. `passes/tree/agg_key_dep.rs` (111 lines)
- **Purpose:** Removes redundant GROUP BY keys using a heuristic: properties named "id" or "_id" are treated as primary keys; other properties of the same variable are removed.
- **Algorithm:** Two-pass: first find ID properties as keys, then for variables without ID props use the first PropertyAccess as the key. Non-property expressions are kept.
- **Markers/Notes:** None

---

### 20. `passes/tree/cardinality.rs` (205 lines)
- **Purpose:** Bottom-up annotation of estimated row counts.
- **Estimates:**
  - ScanNode: 1000 (fallback) or real stats from StatsStore
  - ScanRel: 5000 (fallback) or real stats
  - Filter: child_card * 0.01
  - HashJoin: probe * build / (probe + build)
  - CrossProduct: left * right
  - SemiJoin: min(left, right)
  - AntiJoin: left * 0.1
  - TopK: min(limit, child)
  - Limit: min(limit, child)
  - Aggregate: 1 (no GROUP BY) or child_card
  - Union: left + right
  - VectorSimilarityScan: top_k
  - RecursiveExtend: upper_bound * 100
  - DDL operators: 1
  - EmptyResult: 0
  - Insert: values.len()
- **Markers/Notes:** `impl OptimizationPass for CardinalityEstimation` is a no-op placeholder (returns operators unchanged).
- **Also implements `OptimizationPass`** as a no-op placeholder for backwards compatibility.

---

### 21. `passes/tree/factorization.rs` (177 lines)
- **Purpose:** Inserts `LogicalFlatten` operators for correct WCOJ factorization.
- **Adds Flatten(group_pos=0)** to children of: HashJoin (both sides), Projection, Aggregate, OrderBy, TopK, Limit, Filter, Union (both), CrossProduct (both), SemiJoin (both), AntiJoin (both), Accumulate, SemiMasker, Skip, MultiplicityReducer.
- **Markers/Notes:** Also implements `OptimizationPass` as a no-op placeholder for backwards compatibility.

---

### 22. `passes/tree/foreign_join.rs` (55 lines)
- **Purpose:** Detects HashJoins where one/both sides are backed by foreign tables via `TableFunctionCall` with known prefixes (`duckdb_`, `postgres_`, `sqlite_`, `neo4j_`). Sets `push_down_eligible = true`.
- **Markers/Notes:** None

---

### 23. `passes/tree/sip.rs` (77 lines)
- **Purpose:** Injects `LogicalSemiMasker` into the build side of a HashJoin when the build side is selective (has filters). Uses `has_filter_in_subtree()` from `acc_hash_join`.
- **Contains:** `println!()` debug output when SIP is triggered.
- **Markers/Notes:** None

---

### 24. `passes/tree/subquery_unnesting.rs` (82 lines)
- **Purpose:** Wires `ExpressionsScan` operators in the build side of Accumulate-HashJoins to their outer Accumulate index.
- **Algorithm:** Two-pass: first pass collects (accumulate_idx, build_side_idx) pairs; second pass wires ExpressionsScans.
- **Markers/Notes:** `#[allow(dead_code)]` on `AccHashJoinInfo` struct.

---

### 25. `passes_test.rs` (847 lines)
- **Purpose:** Comprehensive tests for optimizer passes.
- **Tests:** 30+ tests covering:
  - FilterPushDown (basic, combined)
  - PredicatePushDown (basic, no filter, skip existing predicate)
  - ProjectionPushDown
  - JoinOptimization, is_join_condition (equi, non-join, same-var)
  - TopK detection (basic, with projection)
  - RemoveUnnecessaryOperators (empty projection, tautology filter)
  - ConstantFolding (integer add/mul, boolean and/or, string concat, comparison, negate, not, nested, mixed types no-fold)
  - ExtractRootVariable (simple, property access, constant)
  - FactorizationRewriting (inserts Flatten on both sides of HashJoin)
  - CardinalityEstimation (ScanNode default, Aggregate no keys, Limit, CrossProduct)
  - AggKeyDependency (PK only, no PK in keys, non-property keys unchanged, single key)
  - SIPOptimization (triggers on filtered build side, no filter no semi-masker)

---

## CROSS-CUTTING FINDINGS

### `todo!()`, `unimplemented!()` -- **NONE FOUND** in any of the three crates.

### `unreachable!()` -- 5 occurrences:
1. `kuzu-binder/src/binder/dml.rs:503` -- in `bind_union` if LEFT is not BoundQuery
2. `kuzu-binder/src/binder/dml.rs:507` -- in `bind_union` if RIGHT is not BoundQuery
3. `kuzu-binder/src/binder/mod.rs:1148` -- same as #1 (duplicate code)
4. `kuzu-binder/src/binder/mod.rs:1152` -- same as #2 (duplicate code)
5. `kuzu-optimizer/src/passes/flat/top_k.rs:41` -- in TopK optimization, `unreachable!()` after checking pattern

### `placeholder`, `stub`, `FIXME`, `HACK`, `XXX` -- **NONE FOUND.**

### TODO comments -- **NONE FOUND.**

### Notable Notes/Bugs:
1. **Binder code duplication:** `binder/mod.rs` and `binder/dml.rs` contain nearly identical implementations of `bind_query`, `bind_match`, `bind_pattern`, `bind_return`, `bind_where`, `bind_match_create`, `bind_unwind`, `bind_foreach`, `bind_optional_match`, `bind_set`, `bind_union`, `bind_merge`, `bind_create_dml`, `bind_delete`. This is a significant refactoring issue -- the `mod.rs` version appears to be the one actually used (the `Binder` struct and its `bind()` method are defined there), while `dml.rs` contains the same methods on `impl Binder` but is never invoked because `mod.rs` also defines them.

2. **Binder `ddl.rs` is actually test code:** The file starts with `impl Binder {` but immediately has test assertions, then contains parser-level tests (not binder tests) for DDL, sequences, FTS, etc. This is structurally broken.

3. **Property type resolution is heuristic-based:** The binder's `resolve_expression` for PropertyAccess uses hardcoded name -> type mappings (`name` → String, `age` → Int64, etc.) instead of looking up actual column types from the catalog.

4. **Planner produces empty plans for 15+ bound statement types** (see list above).

5. **Optimizer `println!()` in SIP pass** -- production code should use `tracing::debug!()` consistently.

6. **No ORDER BY / SKIP / LIMIT in planner** -- the planner never produces `LogicalOrderBy`, `LogicalLimit`, or `LogicalSkip` operators. The `TopKOptimization` pass looks for OrderBy + Limit patterns that don't exist in the plan, so TopK fusion is currently dead code.

7. **AggregateDetection has no GROUP BY extraction** -- only creates aggregates with empty group_by.

---

### Summary of Implemented vs. Missing Features

| Feature | Binder | Planner | Optimizer |
|---|---|---|---|
| MATCH (node patterns) | Yes | Yes (ScanNode) | Yes |
| MATCH (rel patterns) | Yes | Yes (Extend) | Yes |
| Var-length paths | Yes | Yes (RecursiveExtend) | Yes |
| Optional MATCH | Yes | Yes (OptionalMatch) | Yes |
| RETURN / WITH | Yes | Yes (Projection) | Yes |
| WHERE | Yes | Yes (Filter) | Yes |
| CREATE (pattern) | Yes | Yes (CreateNode+CreateRel) | Yes |
| DELETE | Yes | Yes (Delete) | Yes |
| SET | Yes | Yes (Set) | Yes |
| UNWIND | Yes | Yes (Unwind) | Yes (UnwindDedup) |
| FOREACH | Yes | Yes (Foreach) | Yes |
| MERGE | Yes | Yes (Merge+Set) | Yes |
| UNION ALL | Yes | Yes (Union) | Yes (OrderByPushDown) |
| DISTINCT | Yes | Yes (Aggregate) | Yes |
| Aggregates (COUNT,SUM,etc) | Yes (type only) | Yes (Aggregate) | Yes (AggregateDetection) |
| ORDER BY | No (parser only) | **Missing** | TopK (dead) |
| LIMIT | No (parser only) | **Missing** | LimitPushDown (dead) |
| SKIP | No | **Missing** | No |
| COPY FROM | Yes | Yes (CopyFrom) | Yes |
| COPY TO | Yes | **Yes (empty plan)** | No |
| CREATE NODE TABLE | Yes | Yes | Yes (cardinality) |
| CREATE REL TABLE | Yes | Yes | Yes |
| DROP TABLE | Yes | Yes | Yes |
| ALTER TABLE | Yes | Yes | Yes |
| CREATE INDEX (ART/HASH) | Yes | Yes | Yes |
| DROP INDEX | Yes | Yes | Yes |
| CREATE VECTOR INDEX | Yes | Yes | Yes |
| CREATE SEQUENCE | Yes | Yes | Yes |
| DROP SEQUENCE | Yes | Yes | Yes |
| CREATE MACRO | Yes | **Empty plan** | No |
| EXPLAIN | Yes | Yes | Yes |
| ANALYZE | Yes | **Empty plan** | No |
| TRANSACTION | Yes | **Empty plan** | No |
| EXTENSION (INSTALL/LOAD) | Yes | **Empty plan** | No |
| ATTACH/DETACH/USE DATABASE | Yes | **Empty plan** | No |
| LOAD FROM | Yes | **Empty plan** | No |
| EXPORT DATABASE | Yes | Yes | No |
| IMPORT DATABASE | Yes | Yes | No |
| CREATE FTS INDEX | Yes | Yes | No |
| CREATE TYPE | Yes | **Empty plan** | No |
| COMMENT ON TABLE | Yes | **Empty plan** | No |
| CREATE/USE/DROP GRAPH | Yes | **Empty plan** | No |
| EXISTS subquery | Yes (type only) | **Missing** | CorrelatedSubqueryUnnesting |
| List predicates (ANY/ALL/NONE) | Yes | **Missing** | No |
| CASE expressions | Yes | No | No |
| Lambda expressions | Yes (partial) | **Missing** | No |
| Parameters ($param) | Yes (Any type) | No | No |
| Constant folding | No | No | Yes |
| Filter push-down | No | No | Yes |
| Predicate push-down | No | No | Yes |
| Projection push-down | No | No | Yes |
| Join reordering | No | Yes (greedy) | Yes (greedy+DP) |
| Top-K detection | No | No | Yes (dead code) |
| Vector similarity | No | No | Yes |
| ART range scan | No | No | Yes |
| Factorization | No | No | Yes |
| SIP (sideways info passing) | No | No | Yes |
| Acc hash join | No | No | Yes |
| Foreign join push-down | No | No | Yes |
| Agg key dependency | No | No | Yes |
| Cardinality estimation | No | No | Yes |
| CSE | No | No | Yes |
| Count rel table (CSR) | No | No | Yes |