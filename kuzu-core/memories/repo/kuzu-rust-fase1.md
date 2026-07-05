# Fase 1 & 2 Implementation: Fungsi Built-in & Evaluator + Expression Evaluator (Completed 2026-06-29)

## Summary
### Fase 1 — Completed
All items in Fase 1 of the kuzu-core Rust implementation plan:
- String ops: All 16 variants implemented (Concat, Contains, StartsWith, EndsWith, ToUpper, ToLower, Trim, LTrim, RTrim, Length, Reverse, Repeat, Replace, Substring, RegexMatches, RegexReplace)
- Date ops: All 12 variants implemented with proper time crate integration (Year, Month, Day, Hour, Minute, Second, DatePart, DateTrunc, DateDiff, DateAdd, CurrentDate, CurrentTimestamp)
- List ops: Implemented Creation, Concat, Sort (previously stubs). Fixed Extract to use 1-based indexing (Cypher convention).
- Map ops: Implemented Creation, Extract, Contains (previously stubs).
- Struct ops: Implemented Creation (previously stub).
- Cast ops: Implemented Int32, Float, Bool, Date, Timestamp (previously stubs). Enhanced Int64/Double with more type conversions.
- Boolean ops: Already fully working.
- Utility ops: Already fully working.
- Aggregate: New `AggValueState` enum with full Value-based computation (Count, Sum, Avg, Min, Max, Collect, StdDev, Variance). Wired into PhysicalAggregate in kuzu-processor.

### Fase 2 Step 1 — ExpressionEvaluator (Completed 2026-06-29)
Created a proper `ExpressionEvaluator` in `kuzu-processor/src/expression_evaluator.rs`:
- Recursively evaluates expression trees by calling `evaluate_scalar` from `kuzu-function`
- Supports: `Variable` (read from DataChunk by index or fallback to first field), `Constant` (return literal), `BinaryOp` (dispatch to arithmetic/comparison/boolean via registry), `UnaryOp` (dispatch to NOT/negate), `FunctionCall` (resolve name in registry and evaluate per-row), `PropertyAccess`, `List`/`Map` literals, `Parameter` (null fallback)
- Proper SQL NULL semantics: if any argument to a function is null, the result is null
- Output type detection: determines result type from first non-null evaluation row
- Wired into `PhysicalFilter::execute()` via `self.evaluator` option — when a FunctionRegistry is available (passed from QueryProcessor), the new evaluator is used; otherwise falls back to legacy logic.

## Files Changed
### Fase 1
1. `kuzu-function/Cargo.toml` - Added `time` dependency
2. `kuzu-function/src/scalar.rs` - Major additions: all date functions, list/map/struct ops, cast enhancements, `evaluate_aggregate`, `AggValueState` enum
3. `kuzu-function/src/lib.rs` - Export `evaluate_aggregate`
4. `kuzu-common/src/vector.rs` - Added `get_value()` method to ValueVector
5. `kuzu-processor/src/physical_operator.rs` - Generalized PhysicalAggregate to use `AggValueState` and `Value` types
6. `kuzu-processor/src/processor.rs` - Updated aggregate tests to use Value-based assertions

### Fase 2 Step 1
1. `kuzu-processor/src/expression_evaluator.rs` — **new file**: `ExpressionEvaluator` struct with `evaluate()` method
2. `kuzu-processor/src/lib.rs` — registered `expression_evaluator` module, exported `ExpressionEvaluator`
3. `kuzu-processor/src/physical_operator.rs` — updated `PhysicalFilter` with optional `evaluator` field, `new()`/`with_evaluator()` constructors, updated `evaluate_expression` to use new evaluator when available
4. `kuzu-processor/src/processor.rs` — passes registry-backed `ExpressionEvaluator` to `PhysicalFilter`, updated test constructions to use `PhysicalFilter::new()`

## Test Results
- kuzu-processor: 28 tests, all passing (8 new ExpressionEvaluator tests)
- Full workspace: All 290+ tests passing

## Notes
- `time` crate had to be downgraded from 0.3.51 to 0.3.36 for rustc 1.87.0 compatibility
- Substring and List::Extract now use Cypher 1-based indexing
- Avg returns Double instead of integer division
- StdDev/Variance use population formula (divide by n)
- ExpressionEvaluator variable resolution: if variable name is a numeric string (e.g., "0"), it's used as field index; otherwise falls back to first field (legacy compatibility for unresolved pattern variables like "p")
- Parameter expressions in filter trees are handled by returning null vectors — proper parameter substitution should happen at the prepared statement layer

## Array Functions (2026-07-01) — Added E3 from CONSOLIDATED_PLAN.md
Implemented 5 array math functions + 2 list functions from C++ `function/array/`:
### Array math functions:
- **array_cosine_similarity(a, b)** → `Double` — cosine similarity between two numeric arrays
- **array_distance(a, b)** → `Double` — Euclidean distance between two arrays
- **array_inner_product(a, b)** → `Double` — dot/inner product of two arrays
- **array_cross_product(a, b)** → `List[Double]` — 3D cross product (requires 3-element arrays)
- **array_squared_distance(a, b)** → `Double` — squared Euclidean distance

### List functions fixed/added:
- **list_prepend(list, element)** — registered (variant + implementation existed but was never registered)
- **list_slice(list, start, ?end)** → `List` — 1-based indexing, inclusive end when provided, to end when omitted

Files changed:
- `kuzu-function/src/registry.rs` — Added `ArrayOp` enum (CosineSimilarity, Distance, InnerProduct, CrossProduct, SquaredDistance), `ScalarFunction::Array` variant, `ListOp::Slice` variant, registered all 7 functions
- `kuzu-function/src/scalar.rs` — Added `ListOp::Slice` implementation, `evaluate_array()` function with dispatch, 17 tests
- `kuzu-function/src/lib.rs` — Added `ArrayOp` to re-exports

Note: These are built-in versions of functions that already exist as CustomScalar callbacks in kuzu-vector (cosine_similarity, euclidean_distance, dot_product). The array_* variants work on plain Value::List without requiring the vector extension.

## Schema Functions (2026-07-01) — Added D2 from CONSOLIDATED_PLAN.md

## EXPLAIN Statement (2026-07-01) — Added D3 from CONSOLIDATED_PLAN.md
Implemented the EXPLAIN statement (port of C++ `explain_statement.h`, `logical_explain.h`, `map_explain.cpp`):

### Implementation across 7 layers:
1. **Parser** (`kuzu-parser/src/`):
   - `ast.rs`: Added `ExplainType` enum (PhysicalPlan, LogicalPlan, Profile), `ExplainStatement` struct, `Statement::Explain` variant
   - `parser.rs`: Text-level EXPLAIN prefix handling (avoids PEG recursion issues)
   - 4 parser tests: `EXPLAIN MATCH`, `EXPLAIN LOGICAL`, `EXPLAIN PROFILE`, `EXPLAIN CREATE`

2. **Binder** (`kuzu-binder/src/`):
   - `bound_statement.rs`: Added `BoundExplain` struct
   - `binder.rs`: `bind_explain()` — recursively binds inner statement

3. **Planner** (`kuzu-planner/src/`):
   - `logical_operator.rs`: Added `LogicalExplain` struct with `inner: Box<LogicalOperator>`, `explain_type`
   - `planner.rs`: `plan_explain()` — plans inner statement, wraps in LogicalExplain

4. **Processor** (`kuzu-processor/src/`):
   - `physical_operator.rs`: Added `PhysicalExplain` struct
   - `processor.rs`: `LogicalOperator::Explain` dispatch + `serialize_plan_tree()` helper that DFS-traverses operator tree and builds indented text representation

5. **Optimizer** (`kuzu-optimizer/src/`):
   - `passes.rs`: Added `LogicalOperator::Explain` to match arms in FactorizationRewriting and CardinalityEstimation
   - `join_order.rs`: Added `LogicalOperator::Explain` to leaf/scan match arm

6. **Connection** (`kuzu-main/src/`):
   - `connection.rs`: `handle_ddl` returns `Ok(None)` for Explain (routed through query pipeline)

7. **PreparedStatement**: `collect_params_from_statement` recurses into BoundExplain inner
Implemented 5 schema functions ported from C++ `function/schema/`:
- **OFFSET(v)** → `INT64` — extracts `InternalID.offset` from a node/rel InternalID or struct with `_id` field
- **ID(v)** → `InternalID` — returns the InternalID (offset + table_id) unchanged, or extracts from struct's `_id` field
- **START_NODE(r)** → `InternalID` — extracts `_src` field from a rel struct
- **END_NODE(r)** → `InternalID` — extracts `_dst` field from a rel struct
- **LABEL(v)** → `STRING` — returns string directly, or extracts `_label` from struct, or formats `Table({table_id})` from InternalID

Files changed:
- `kuzu-function/src/registry.rs` — Added `SchemaOp` enum (Offset, Id, StartNode, EndNode, Label), `ScalarFunction::Schema` variant, registration of 5 functions
- `kuzu-function/src/scalar.rs` — Added `evaluate_schema()` function, dispatch in `evaluate_scalar()`, 12 tests
- `kuzu-function/src/lib.rs` — Added `SchemaOp` to re-exports

Note: C++ classifies ID/START_NODE/END_NODE/LABEL as **rewrite functions** (compile-time expression replacement), while OFFSET is a runtime **scalar function**. The Rust port implements all 5 as scalar functions for simplicity — they operate on `Value::InternalID` and `Value::Struct` at evaluation time. Full rewrite-function optimization can be added later if needed.
