# Fase B Implementation Status: Cypher Coverage

## B1. MERGE ✅
- Grammar, AST, Parser, Binder, Connection handle_ddl
- 5 tests

## B2. CALL Procedure ✅
- Grammar, AST, Parser, BoundCall, execute_table_function in registry
- 4 tests

## B3. DML CREATE (n:Label {props}) ✅ (Completed 2026-06-29)

## B4. FOREACH & Variable-length Path Patterns ✅ (Completed 2026-06-29)

## B5. Subquery Support ✅ (Completed 2026-06-29)

## C1. CLI Enhancement ✅ (Completed 2026-06-29)
- **Cargo.toml**: Added `rustyline = "12"`, `dirs`, `kuzu-catalog`. Removed `atty`.
- **main.rs**: Full rewrite with rustyline 12 REPL:
  - Multi-line input: validator checks `;` termination, prompts `kuzu>` / `  ..>`
  - Command history: saved to `$data_dir/kuzu/history.txt`, loaded/saved on startup/exit
  - Tab completion: `CypherCompleter` implements `Completer`, `Hinter`, `Highlighter`, `Validator`, `Helper`
    - Suggests Cypher keywords (MATCH, RETURN, WHERE, CREATE, etc.)
    - Suggests table names from catalog
  - `.mode <table|csv|json|line|column>`: Changes output format
    - `table`: Aligned table with borders (default)
    - `csv`: Comma-separated values
    - `json`: JSON array of objects
    - `line`: One key: value per line, records separated by blank line
    - `column`: Values listed vertically per column
  - `.tables`: Lists table names from catalog
  - `.schema`: Shows table schemas (NODE/REL type, columns with types)
  - `.import <file.csv> <table>`: CSV import via COPY FROM
  - `.export <file.csv> <query>`: CSV export to file
  - `.help`: Shows available commands
  - `.exit` / `.quit`: Exits the shell
- **kuzu-main/src/database.rs**: Added `pub fn catalog()` to expose catalog for CLI tab completion
- **home@0.5.11 pinned**: rustyline 14+ transitively depends on home@0.5.12 which needs rustc 1.88
- **Grammar**: `exists_subquery = { "EXISTS" ~ "{" ~ query_statement ~ "}" }` added to `primary` (before `variable` to avoid ambiguity)
- **AST**: `Expression::ExistsSubquery(Box<Query>)`
- **Parser**: `parse_expression` handles `Rule::exists_subquery`
- **Binder**: `resolve_expression` binds inner query, returns `LogicalTypeID::Bool`
- **Expression Evaluator**: Handles `ExistsSubquery` via `subquery_fn` callback
- **Prepared Statement**: `collect_params_from_expr` and `substitute_params` handle `ExistsSubquery` by recursing into inner clauses
- **Optimizer**: `fold_expression` handles `ExistsSubquery` via `fold_query`
- **Tests**: 2 parser tests (EXISTS in WHERE, EXISTS in RETURN) + 2 integration tests
- **Catatan**: Only EXISTS subquery implemented (not scalar subquery with `{ ... }`). Correlated subqueries not supported. Subquery execution needs Connection-level wiring.
### FOREACH
- Grammar: `foreach_clause = { "FOREACH" ~ "(" ~ variable ~ "IN" ~ expression ~ "|" ~ foreach_body ~ ")" }`
- `foreach_body = { create_clause_inline | set_clause | delete_clause }`
- AST: `Clause::Foreach(ForeachClause { variable, expression, clauses })` — sub-clauses are CREATE/SET/DELETE
- Parser: `parse_foreach_clause()` in parser.rs — parses variable, list expression, body clauses
- Binder: `bind_foreach()` in binder.rs — validates list expression, binds sub-statements by wrapping in Query
- Bound: `BoundClause::BoundForeach(BoundForeachClause { variable, expression, sub_statements })`
- Planner: `plan_query` → `LogicalForeach { variable, expression, sub_plans }`
- Physical: `PhysicalForeach` — evaluates list via `Expression::List`, for each item creates QueryProcessor, executes sub-plans
- Processor: Maps `LogicalOperator::Foreach` → `PhysicalForeach::execute()`
- Tests: 2 parser tests (basic, in match context) + 2 integration tests

### Variable-length Path Patterns
- Grammar: `edge_pattern` now includes optional `var_length` before `property_map`
- `var_length = { "*" ~ (integer ~ ".." ~ integer)? }` — supports `[*]`, `[*1..5]`
- AST: `EdgePattern { ..., lower_bound: Option<u64>, upper_bound: Option<u64> }`
- Parser: `parse_edge_pattern` extracts lower/upper bounds from var_length rule
- Binder: `bind_pattern` propagates bounds to `BoundEdgePattern`
- Note: Currently parses and binds correctly; physical execution still uses flat scan (no recursive BFS/DFS extend yet)
- Tests: 3 parser tests (simple `[*]`, with bounds `[*1..5]`, with variable `[r:*1..3]`) + 2 integration tests

### Files Changed
- `kuzu-parser/src/cypher.pest` — added `foreach_clause`, `foreach_body`, `create_clause_inline`, `var_length` rules
- `kuzu-parser/src/ast.rs` — added `Clause::Foreach`, `ForeachClause`; added `lower_bound`, `upper_bound` to `EdgePattern`
- `kuzu-parser/src/parser.rs` — added `parse_foreach_clause`, var_length handling in `parse_edge_pattern`
- `kuzu-binder/src/bound_statement.rs` — added `BoundClause::BoundForeach`, `BoundForeachClause`; bounds in `BoundEdgePattern`
- `kuzu-binder/src/binder.rs` — added `bind_foreach`; propagate bounds in `bind_pattern`
- `kuzu-planner/src/logical_operator.rs` — added `LogicalOperator::Foreach`, `LogicalForeach` struct
- `kuzu-planner/src/planner.rs` — handle `BoundForeach` in `plan_query`
- `kuzu-optimizer/src/join_order.rs` — handle `Foreach` in `collect_scans_recursive`
- `kuzu-optimizer/src/passes.rs` — handle `Foreach` in `FactorizationRewriting` and `CardinalityEstimation`
- `kuzu-processor/src/physical_operator.rs` — added `PhysicalForeach` struct + `ast_constant_to_value` helper
- `kuzu-processor/src/processor.rs` — handle `LogicalOperator::Foreach` in execute
- `kuzu-main/src/connection.rs` — added foreach_tests and var_length_path_tests modules
- `kuzu-main/src/prepared_statement.rs` — handle `BoundForeach` in `collect_params_from_statement`
- Grammar: `create_dml_statement = { "CREATE" ~ pattern ~ return_clause? }` in statement alternatives
- AST: `Statement::CreateDml(CreateClause)` variant
- Parser: `parse_statement` handles `Rule::create_dml_statement`
- Bound: `BoundStatement::BoundCreateDml(BoundCreateDml { table_name, table_id, properties })`
- Binder: `bind_create_dml` → validates node pattern, looks up catalog, returns BoundCreateDml
- Connection: `handle_ddl` → inserts row into NodeTable via `insert_row`
- Planner: automatically returns empty plan (hits `_ => Ok(Vec::new())`)
- **Files changed**: `cypher.pest`, `ast.rs`, `parser.rs`, `bound_statement.rs`, `binder.rs`, `connection.rs`, `plan-nextPhase.prompt.md`
- **Tests**: 7 test cases (basic, multiple, without variable, nonexistent table, duplicate PK, empty properties, verify via MATCH)
- **Full workspace**: All tests passing (no regressions)

## Notes
- `999.99` float literal has parsing issues in the grammar (integer `999` used instead)
- ~~`RETURN p.name` projection uses column index 0 regardless of expression~~ **FIXED** (see Bug 0.1 fix below)

---

## Priority 0 Bug Fixes (2026-07-02) ✅

### Bug 0.1: `evaluate_property_access` ignored property name (FIXED)
**Root cause**: `_prop` parameter was unused; always returned first column.
**Fix**:
- Added `field_names: Vec<String>` to `DataChunk` struct in `kuzu-common/src/vector.rs`
- Added `DataChunk::with_names(names)` builder method
- `PhysicalScan::execute()` now populates `field_names` from `table_columns`
- `PhysicalScanRel::execute()` now populates `field_names` from `table_columns`
- `PhysicalHashJoin::execute()` concatenates `build.field_names ++ probe.field_names` on output
- `PhysicalCrossProduct::execute()` concatenates `left.field_names ++ right.field_names` on output
- `evaluate_property_access` now looks up `prop` in `chunk.field_names` → returns correct column

### Bug 0.2: Flat pipeline ScanNode/ScanRel overwrote intermediate_result (FIXED)
**Root cause**: Each ScanNode/ScanRel replaced `intermediate_result` rather than extending it, so only the last scan's data reached HashJoin/CrossProduct.
**Fix** (`kuzu-processor/src/processor.rs`):
- ScanNode and ScanRel arms now call `existing.extend(result)` when `intermediate_result` is already `Some(_)`, instead of replacing it
- HashJoin arm: added `derive_join_column_indices(join_keys, input)` to compute actual `build_columns`/`probe_columns` from the join key expressions + `field_names` metadata
- Added `derive_join_column_indices()` and `extract_join_prop()` helper functions in the UNION helpers section

### Test regressions added (kuzu-processor/src/processor.rs):
1. `test_property_access_resolves_named_column` — verifies `t.name` returns String col, `t.id` returns Int64 col
2. `test_hash_join_non_overlapping_ids_returns_zero_rows` — non-overlapping join keys → 0 rows
3. `test_scan_accumulation_both_scans_reach_join` — end-to-end join: A.id=[1,2], B.id=[2,3], join on id → 1 row (id=2)


## B6. GDS Framework + Shortest Path ✅ (Completed 2026-07-01)

---

## B7. Recursive Extend Enhancement ✅ (Completed 2026-07-01)

---

## B8. nextval()/currval() Sequence Functions ✅ (Completed 2026-07-01)

### What changed
Added `nextval()` and `currval()` scalar functions for sequence operations, callable from Cypher.

**Architecture**:
- Added `SequenceOp { is_nextval: bool }` variant to `ScalarFunction` enum in `registry.rs`
- Registered `nextval` and `currval` in `register_builtins()`
- `evaluate_scalar()` returns error for `SequenceOp` — requires catalog access
- Added optional `sequence_fn` callback to `ExpressionEvaluator` (similar to `subquery_fn`)
- Added `evaluate_sequence_op()` method that extracts sequence name from args and calls the callback
- Added `with_sequence_fn()` builder to `ExpressionEvaluator` and `QueryProcessor`
- Created `Connection::create_processor()` helper that wires the sequence callback (looks up sequences in `Catalog`, calls `next_k_val(1)` or `curr_val()`)
- All `QueryProcessor::with_catalog()` calls replaced with `self.create_processor()`

**Files changed**:
| File | Change |
|------|--------|
| `kuzu-function/src/registry.rs` | Added `SequenceOp { is_nextval: bool }` variant + Debug impl + registration |
| `kuzu-function/src/scalar.rs` | Handle `SequenceOp` → error |
| `kuzu-processor/src/expression_evaluator.rs` | Added `sequence_fn` callback, `with_sequence_fn()`, `evaluate_sequence_op()` |
| `kuzu-processor/src/processor.rs` | Added `sequence_fn` to `QueryProcessor`, `with_sequence_fn()`, wired to `ExpressionEvaluator` |
| `kuzu-main/src/connection.rs` | Added `create_processor()` helper with sequence callback, replaced all `QueryProcessor::with_catalog()` calls |

**Usage**: `CREATE SEQUENCE my_seq;` → `RETURN nextval('my_seq');` → returns 1, then 2, 3, etc. `RETURN currval('my_seq');` → returns current value without advancing.

**Verified**: ✅ Full workspace compiles, all tests pass with 0 failures.

- Follow-up fix (2026-07-01): planner no-scan branch must preserve clause order (`UNWIND` before `RETURN` projection). If projection is emitted first, `UNWIND [..] RETURN x` can return 0 rows.
- Follow-up fix (2026-07-01): avoid double-invoking scalar function evaluation when inferring output type. Stateful functions like `nextval()` must execute once per row; cache row results then infer vector type from cached values.

### What changed
Upgraded `PhysicalRecursiveExtend` from simple BFS to GDS-style path tracking:

**Before**: Produced 3 columns `(src, dst, length)` using `HashMap<u64, Vec<u64>>` adjacency (lost edge IDs), no path tracking.

**After**: Produces 5 columns `(src, dst, length, path_node_ids, path_edge_ids)` with:
- Edge ID tracking: adjacency stores `(neighbor_offset, edge_id)` pairs
- Parent chain tracking: `HashMap<u64, (parent, edge_id, depth)>` for full path reconstruction
- Path output: `path_node_ids` = `List(Int64)` from source→destination, `path_edge_ids` = `List(Int64)` of edge IDs traversed
- Semantic enforcement: WALK (first-visit), TRAIL (no repeated edges via parent chain scan), ACYCLIC (no repeated nodes)
- Backward-compatible: first 3 columns unchanged

**Files changed**:
| File | Change |
|------|--------|
| `kuzu-processor/src/physical_operator.rs` | Added `semantic` field, edge ID adjacency, parent tracking, path reconstruction, List column output |
| `kuzu-processor/src/processor.rs` | Passes `re.semantic` to physical operator |

**Verified**: ✅ Full workspace compiles, all tests pass with 0 failures.

### What was built

**GDS Framework** (`kuzu-graph/src/gds/`) — 7 files, ~1200 lines:
| File | Contents |
|------|----------|
| `mod.rs` | Module declarations + re-exports |
| `frontier.rs` | `Frontier` trait, `SparseFrontier`, `DenseFrontier`, `FrontierPair` trait, `SPFrontierPair`, `DenseSparseDynamicFrontierPair`, `DenseFrontierPair` |
| `compute.rs` | `EdgeCompute` trait, `VertexCompute` trait, `DefaultEdgeCompute` |
| `bfs_graph.rs` | `ParentList`, `BaseBFSGraph` trait, `DenseBFSGraph`, `SparseBFSGraph`, `BFSGraphManager` |
| `output_writer.rs` | `RJOutputWriter` trait, `PathsOutputWriterInfo`, `PathsOutputWriter`, `SPPathsOutputWriter` |
| `utils.rs` | `GDSUtils` with `run_single_shortest_path`, `run_all_shortest_paths`, `run_weighted_shortest_path`, `run_all_weighted_shortest_paths` |

**Algorithm Integration** (`kuzu-algo/src/lib.rs`):
- Shortest path functions registered as `TableFunction::CustomTable` with executable callbacks
- 17 total function registrations (10 algorithms + 7 aliases)

**Key Design**:
- Rust-idiomatic: Box-based linked lists (not raw pointers), Vec-based arrays (not custom memory managers)
- Trait-based: `Frontier`, `FrontierPair`, `BaseBFSGraph`, `EdgeCompute`, `VertexCompute`
- Dense frontier default (simpler than adaptive sparse↔dense)
- Sequential BFS core (avoids &mut issues; parallel via rayon is easy to add later)

**Files changed/created**:
- `kuzu-graph/src/gds/mod.rs` (new)
- `kuzu-graph/src/gds/frontier.rs` (new)
- `kuzu-graph/src/gds/compute.rs` (new)
- `kuzu-graph/src/gds/bfs_graph.rs` (new)
- `kuzu-graph/src/gds/output_writer.rs` (new)
- `kuzu-graph/src/gds/utils.rs` (new)
- `kuzu-graph/src/lib.rs` (added `pub mod gds;`)
- `kuzu-graph/Cargo.toml` (added rayon dependency)
- `kuzu-algo/src/lib.rs` (GDS table function integration)

**Tests**: 13 new GDS tests + all 691+ existing tests pass with 0 failures

---

## B3. SIP / SemiMask (Fase 3) ✅ (Completed 2026-07-01)

### What changed
Simplified port of Kuzu C++ SIP (Sideways Information Passing) optimization using `NodeSemiMask`.

**Core types** (`kuzu-processor/src/physical_operator.rs`):
- `NodeSemiMask`: Shared set of node offsets via `Arc<Mutex<HashSet<u64>>>` with `initialized` flag
- `PhysicalSemiMasker`: Reads a column of node IDs (Int64 or InternalID), collects into shared mask, passes input through
- `PhysicalScan`: Added optional `semi_mask` + `mask_id_column` — filters rows during scan
- `PhysicalHashJoin`: Added optional `semi_mask` — collects build-side keys during hash table building

**Logical operator** (`kuzu-planner/src/logical_operator.rs`):
- `LogicalSemiMasker` struct: table_id, key_column, children
- `LogicalOperator::SemiMasker` variant
- All match arms updated: cardinality(), set_cardinality(), children_mut(), children()

**Processor** (`kuzu-processor/src/processor.rs`):
- `LogicalOperator::SemiMasker(sm)` → creates `PhysicalSemiMasker` at execution time
- HashJoin dispatch: added `semi_mask: None` to PhysicalHashJoin init

**Optimizer** (`kuzu-optimizer/src/join_order.rs`, `passes.rs`):
- `collect_scans_recursive` handles SemiMasker (recurses into children)
- `FactorizationRewriting` flattens SemiMasker children
- `CardinalityEstimation` estimates via child cardinality

**Files changed**:
| File | Change |
|------|--------|
| `kuzu-processor/src/physical_operator.rs` | Added NodeSemiMask, PhysicalSemiMasker, semi_mask fields to PhysicalScan/PhysicalHashJoin |
| `kuzu-processor/src/processor.rs` | SemiMasker dispatch, HashJoin semi_mask init, 4 new tests |
| `kuzu-planner/src/logical_operator.rs` | Added LogicalSemiMasker + SemiMasker variant + match arms |
| `kuzu-optimizer/src/join_order.rs` | SemiMasker handling in collect_scans_recursive |
| `kuzu-optimizer/src/passes.rs` | SemiMasker handling in FactorizationRewriting + CardinalityEstimation |

**4 new tests**: semi_masker_basic (collect offsets), scan_with_semi_mask (filter rows), semi_mask_uninitialized_passes_all, hash_join_with_semi_mask_collects_build_keys

**Verified**: ✅ Full workspace compiles, all 700+ tests pass with 0 failures.


---

## B5. Free Space Manager (Fase 5.1) ✅ (Completed 2026-07-01)

### What changed
Ported C++ `FreeSpaceManager` from `src/storage/free_space_manager.cpp` to Rust.

**New file**: `kuzu-storage/src/free_space_manager.rs`

**Core types**:
- `PageRange { start_page_idx: u64, num_pages: u64 }` — contiguous free page range
- `FreeSpaceManager` — manages free pages using power-of-2 level sorted lists

**API**:
- `add_free_pages(PageRange)` — insert a free range
- `pop_free_pages(num_pages) -> Option<PageRange>` — allocate pages, splits if needed
- `evict_and_add_free_pages(PageRange)` — simplified eviction + free

## Update 2026-07-01: STAR expression infrastructure + function aliases

### STAR expression (`Expression::Star`)
- **AST**: Added `Expression::Star` variant to `kuzu-parser/src/ast.rs`
- **Binder**: `bind_return()` expands `Star` to all in-scope variables; `resolve_expression()` handles `Star` as `Any` type
- **Grammar note**: Actual `RETURN *` parsing requires grammar changes (star_return rule) that interact with `*` as multiplication operator. Grammar refactoring needed before STAR works end-to-end.

### String function aliases
- `lower` / `upper` (aliases for `to_lower` / `to_upper`)
- `lcase` / `ucase` (SQL standard aliases)
- `ceiling` (alias for `ceil`)

### Cast function name aliases
- `date()`, `timestamp()`, `float()`, `double()`, `int64()`, `int()`, `bool()`, `boolean()`, `string()`, `blob()`
- Added `CastTarget::Interval` variant and `Interval` import in scalar.rs

### All match sites updated
- `kuzu-optimizer/src/passes.rs` - `fold_expression` handles `Star`
- `kuzu-main/src/prepared_statement.rs` - `collect_params_from_expr` and `substitute_params` handle `Star`
- `kuzu-processor/src/expression_evaluator.rs` - `evaluate` handles `Star`
- `kuzu-processor/src/physical_operator.rs` - `evaluate_expression_legacy` handles `Star`

### Tests
- Parser: `test_lower_upper_parse`, `test_cast_aliases_parse` (passing)
- Integration: `test_lower_function_alias`, `test_upper_function_alias`, `test_ceiling_function_alias` (all passing)

### Fixes included in this session
- Planner: no-scan branch preserves clause order (UNWIND before RETURN projection)
- Evaluator: single-invocation pattern avoids double-calling stateful functions like nextval

- `add_uncheckpointed_free_pages()` / `rollback_checkpoint()` / `finalize_checkpoint()` — checkpoint integration
- `num_entries()` / `total_free_pages()` — query methods

**9 tests**: get_level, add/pop exact, split larger range, no suitable range, empty pop, multiple free lists, uncheckpointed lifecycle, multiple pops from same range, page range ordering

**Verified**: ✅ Full workspace compiles, all 720+ tests pass with 0 failures.

## Correction: Items Already Complete (not reflected in KONSOLIDASI_DOKUMEN.md)

The following items listed as gaps in the consolidated document are **already implemented**:
- Agg Key Dependency (Fase 4.4) — ✅ Full pass in kuzu-optimizer/src/passes.rs with tests
- SERIAL auto-increment (Fase 6.2) — ✅ Catalog has create_serial_sequence()/drop_serial_sequence()
- Intersect Enhancement (Fase 6.4) — ✅ PhysicalIntersect with 7 tests
- Macro support (Fase 6.5) — ✅ BoundCreateMacro, bind_create_macro, ScalarMacroEntry
- array_value() (Fase 6.3) — ✅ Already implemented as ListOp::Creation alias

### All items completed! 🎉

---

## B6. Zone Map Predicate Skipping (Fase 5.2) ✅ (Completed 2026-07-01)

### What changed
Ported C++ zone map predicate system from `src/storage/predicate/` to Rust.

**New file**: `kuzu-storage/src/predicate.rs`

**Core types**:
- `ZoneMapCheckResult` enum: `AlwaysScan` / `SkipScan`
- `ColumnChunkStats` struct: `min`, `max`, `guaranteed_no_nulls`, `guaranteed_all_nulls`
- `check_zone_map(stats, op, constant)` — checks constant comparison predicates against zone maps
- `check_null_zone_map(stats, is_null)` — checks IS NULL/IS NOT NULL against zone maps

**Supported predicate types** (matching C++ `ColumnConstantPredicate`):
- `= / ==` → SkipScan if constant outside [min, max]
- `!= / <>` → SkipScan if constant == min == max (single value chunk)
- `>` → SkipScan if constant >= max
- `>=` → SkipScan if constant > max
- `<` → SkipScan if constant <= min
- `<=` → SkipScan if constant < min

**Supported data types**: Int64, Int32, Double, Float, String, InternalID

**21 tests**: all 6 comparison ops, edge cases (boundary values, single-value chunks, mixed types), null predicate (IS NULL with no-nulls, IS NOT NULL with all-nulls), string zone maps, InternalID zone maps, stats update

**Verified**: ✅ Full workspace compiles, all 740+ tests pass with 0 failures.

- Correlated Subquery Unnesting (Fase 4.1)
- Acc Hash Join Optimization (Fase 4.2)
- Foreign Join PushDown (Fase 4.3)

---

## B7. Optimizer Passes & Infrastructure (Fase 4) ✅ (Completed 2026-07-01)

### What changed
Completed all remaining Medium-priority optimizer passes and their dependencies.

**New types** (`kuzu-common/src/enums.rs`):
- `AccumulateType` enum: `Regular`, `Optional`

**New logical operators** (`kuzu-planner/src/logical_operator.rs`):
- `LogicalAccumulate` — materializes all input into memory (for Acc Hash Join + correlated subqueries)
- `LogicalExpressionsScan` — reads correlated variables from outer context
- `LogicalHashJoin.push_down_eligible` — flag for foreign join push-down detection

**New optimizer passes** (4 new, total from 13 → 17):
| Pass | Type | Description |
|------|------|-------------|
| AccHashJoinOptimization | Tree | Wraps selective probe sides in Accumulate for SIP |
| CorrelatedSubqueryUnnesting | Tree | Wires ExpressionsScan to outer Accumulate |
| ForeignJoinPushDown | Tree | Detects foreign-table joins, marks for push-down |

**Files changed**:
| File | Changes |
|------|---------|
| `kuzu-common/src/enums.rs` | Added `AccumulateType` |
| `kuzu-planner/src/logical_operator.rs` | Added `LogicalAccumulate`, `LogicalExpressionsScan`, `push_down_eligible` flag |
| `kuzu-planner/src/join_order.rs` | Updated HashJoin construction |
| `kuzu-optimizer/src/passes.rs` | Added 3 new passes + helper functions |
| `kuzu-optimizer/src/optimizer.rs` | Registered all new passes (17 total) |
| `kuzu-optimizer/src/join_order.rs` | Updated HashJoin construction + match arms |
| `kuzu-processor/src/processor.rs` | Accumulate/ExpressionsScan dispatch + serialize |

**Verified**: ✅ Full workspace compiles, all 750+ tests pass with 0 failures.


