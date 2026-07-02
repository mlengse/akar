---
description: "Port a C++ feature from Kuzu/LadybugDB C++ sources to Rust in kuzu-core. Full workflow: research C++ → plan → implement → test → verify."
agent: "Kuzu C++ to Rust Refactor"
---
You are implementing a feature from the Kuzu C++ codebase (or LadybugDB C++ fork) into the Rust port at `kuzu-core/`. Follow the **CONSOLIDATED_PLAN.md** methodology.

## Input

The feature to port is described below. Read it carefully, then follow the workflow.

{input}

## Workflow

### Step 1: Research C++ Sources

1. Search the C++ sources in `src/` (main Kuzu) and `ladybug/` (LadybugDB fork) for relevant files:
   - Header files: `src/include/...` and `ladybug/src/include/...`
   - Implementation files: `src/...` and `ladybug/src/...`
2. Read the key C++ files to understand:
   - Data structures and types used
   - Public API surface
   - Algorithm/execution logic
   - How it integrates with other subsystems (catalog, planner, processor, storage, etc.)
3. Document your findings in session memory (`/memories/session/`) for reference during implementation.

### Step 2: Check Existing Rust Implementation

1. Search the Rust crates in `kuzu-core/` for any existing stubs, TODOs, or partial implementations:
   - `kuzu-parser/` — grammar (`cypher.pest`), AST (`ast.rs`), parser (`parser.rs`)
   - `kuzu-binder/` — binder (`binder.rs`), bound types (`bound_statement.rs`)
   - `kuzu-planner/` — planner (`planner.rs`), logical operators (`logical_operator.rs`)
   - `kuzu-optimizer/` — optimizer passes (`passes.rs`)
   - `kuzu-processor/` — physical operators (`physical_operator.rs`), processor (`processor.rs`)
   - `kuzu-catalog/` — catalog entries (`lib.rs`)
   - `kuzu-function/` — function registry (`registry.rs`), scalar functions (`scalar.rs`)
   - `kuzu-storage/` — storage engine files
   - `kuzu-common/` — types (`types.rs`), enums (`enums.rs`)
   - `kuzu-main/` — connection (`connection.rs`), prepared statements (`prepared_statement.rs`)
2. Read existing similar features to understand patterns and conventions.

### Step 3: Plan the Implementation

Based on C++ research and Rust status, produce a detailed plan:

| Step | File(s) | Changes |
|------|---------|---------|
| Parser | `kuzu-parser/src/cypher.pest`, `ast.rs`, `parser.rs` | Grammar rules, AST types, parse function |
| Binder | `kuzu-binder/src/binder.rs`, `bound_statement.rs` | Binding logic, bound types |
| Planner | `kuzu-planner/src/planner.rs`, `logical_operator.rs` | Logical operator variant, planning |
| Optimizer | `kuzu-optimizer/src/passes.rs` | Optimization pass (if applicable) |
| Processor | `kuzu-processor/src/processor.rs`, `physical_operator.rs` | Physical operator, execution |
| Catalog | `kuzu-catalog/src/lib.rs` | Catalog entry types (if new) |
| Function | `kuzu-function/src/registry.rs`, `scalar.rs` | Built-in functions (if new) |
| Storage | `kuzu-storage/src/...` | Storage changes (if needed) |
| Connection | `kuzu-main/src/connection.rs` | DDL/query handling (if needed) |
| Tests | `kuzu-main/tests/` or crate-level tests | New test cases |

### Step 4: Implement

Implement each step in order, following existing patterns in the Rust codebase:

- **Grammar changes**: Add PEG rules to `cypher.pest`. Keep rules modular and composable.
- **AST types**: Add variants to enums in `ast.rs`. Match C++ semantics but use Rust idioms.
- **Parser**: Add parse functions that return `Result<Statement/Clause/Expression>`.
- **Binder**: Bind parsed structures to validated bound types with catalog lookups.
- **Planner**: Create logical operators. Use `cardinality: u64` for cost estimation.
- **Optimizer**: Add passes if the feature benefits from rewrite (e.g., filter push-down, detection passes).
- **Processor**: Implement `PhysicalOperatorExec` trait with `execute(&self, input: Vec<DataChunk>) -> OperatorResult`.
- **Functions**: Register in `register_builtins()` using `register_scalar_function` / `register_aggregate_function` / `register_table_function`.

### Step 5: Test

1. Add unit tests for each layer:
   - Parser tests: test grammar parses correctly
   - Binder tests: test binding validates correctly
   - Processor tests: test execution produces correct results
   - Integration tests: end-to-end Cypher queries
2. Include:
   - Happy path
   - Edge cases (empty input, nulls, boundary values)
   - Error conditions (invalid syntax, type mismatches, nonexistent references)

### Step 6: Verify

After implementing all changes:

```bash
# Check compilation
cargo check --workspace

# Run clippy (no new warnings preferred)
cargo clippy --workspace

# Run tests
cargo test --workspace
```

- ✅ All existing tests MUST still pass (0 regressions)
- ✅ New tests cover: happy path, edge cases, error conditions
- ✅ Public API matches C++ semantics where applicable

## Reference Documents

- **Feature Gap List**: Sections C1–F3 in the plan for prioritized gaps
- **C++ Sources**: `src/` (main Kuzu), `ladybug/` (LadybugDB fork)
- **Rust Workspace**: `kuzu-core/` (28 crates)
- **Verification Checklist**: Section 6 in the plan

## Important Notes

- The Rust workspace uses `pest.rs` PEG parser (not ANTLR4). Grammar is in `kuzu-parser/src/cypher.pest`.
- Physical operators implement `PhysicalOperatorExec` trait.
- Logical operators have a `cardinality: u64` field for cost-based optimization.
- The optimizer has 13 passes: 11 flat (`OptimizationPass` trait) + 2 tree (`TreeOptimizationPass` trait).
- New passes in `passes.rs` must be registered in `optimizer.rs::Optimizer::new()`.
- All extensions implement the `Extension` trait from `kuzu-extension`.
- Use `Arc<Mutex<...>>` for shared mutable state (catalog, storage).
- Use `DashMap` for concurrent map access.
