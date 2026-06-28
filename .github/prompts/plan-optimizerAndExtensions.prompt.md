# Plan: Fase 17 & 19 — Optimizer Deepening + Extensions Porting

## TL;DR

Two independent workstreams:

**Fase 17** — Deepen `FactorizationRewriting` and `CardinalityEstimation` from no-op stubs to real implementations. Both require adding a tree-based visitor pattern alongside the existing flat-list passes, adding `LogicalFlatten` + cardinality fields, and implementing the core logic from the C++ reference.

**Fase 19** — Port 9 remaining C++ extensions. 3 are feasible now (ALGO, NEO4J, LLM) using existing `kuzu-graph` algorithms. 6 are **blocked** on a real DuckDB Rust binding (AZURE, DELTA, ICEBERG, POSTGRES, SQLITE, UNITY_CATALOG) — they get placeholder crates with documentation.

---

## Phase A: Fase 17 — Optimizer Passes Deepening

### Prerequisite: Tree-based visitor infrastructure

The current `OptimizationPass` trait works on flat `&[LogicalOperator]`. Both target passes in C++ use bottom-up tree traversal. We need a second pass trait and a tree visitor utility.

**Step A1** — Add `cardinality: u64` and schema tracking to `LogicalOperator` variants

- Modify each `Logical*` struct in `kuzu-planner/src/logical_operator.rs` to add `cardinality: u64` (default 0)
- Add `compute_factorized_schema()` method or equivalent — start with a minimal no-op that just propagates
- Add `LogicalFlatten` variant with `group_pos: usize, children: Vec<LogicalOperator>` to the enum
- Re-export from `kuzu-planner/src/lib.rs`

*Files: `kuzu-core/kuzu-planner/src/logical_operator.rs`, `kuzu-core/kuzu-planner/src/lib.rs`*

**Step A2** — Add `TreeOptimizationPass` trait alongside existing one

- New trait: `TreeOptimizationPass` with `fn apply_tree(&self, root: &mut LogicalOperator)`
- Tree helper: `fn visit_operators_bottom_up(root: &mut LogicalOperator, f: &mut dyn FnMut(&mut LogicalOperator))`
- Method on `LogicalOperator` to get children mutably (centralized dispatcher)

*File: `kuzu-core/kuzu-optimizer/src/passes.rs`*

**Step A3** — Implement `FactorizationRewriting` (tree-based)

- Port the 153-line C++ `factorization_rewriter.cpp` logic
- Implements `TreeOptimizationPass`
- Bottom-up traversal, for each operator type:
  - **HashJoin**: flatten probe-side + build-side groups via `get_groups_pos_to_flatten_on_probe_side()`
  - **Projection**: if random function → flatten all; else `FlattenAllButOne` per expression
  - **Aggregate/OrderBy/Limit/Distinct/Unwind/Filter**: flatten required groups
  - **Intersect**: flatten probe + each build child
  - **Union**: flatten each child
  - **SetProperty/Delete/Insert/Merge/CopyTo**: flatten groups
- Helper `append_flattens()` inserts `LogicalFlatten` nodes before children for non-flat groups
- After rewrite, calls `compute_factorized_schema()` on each visited node

*File: `kuzu-core/kuzu-optimizer/src/passes.rs`*

**Step A4** — Implement `CardinalityEstimation` (tree-based)

- Port the 120-line C++ `cardinality_updater.cpp` logic
- Implements `TreeOptimizationPass`
- Bottom-up traversal, using static selectivity constants (no storage dependency yet):
  - **ScanNode**: if PK equality → 1; else use table name heuristic (configurable node count)
  - **Filter (equality, PK)**: 1
  - **Filter (equality, non-PK)**: child_card * 0.01 (EQUALITY_PREDICATE_SELECTIVITY)
  - **Filter (non-equality)**: child_card * 0.1 (NON_EQUALITY_PREDICATE_SELECTIVITY)
  - **HashJoin (nodeID-only)**: probe_card * build_card / max(1, denominator)
  - **HashJoin (non-ID)**: probe_card * build_card * 0.01^k
  - **CrossProduct**: probe_card * build_card
  - **Flatten**: child_card * multiplier
  - **Limit**: literal limit value
  - **Aggregate**: 1 if no keys, else child_card
  - **Default** (single-child): propagate child cardinality
- Store estimates on `cardinality` field of each operator

*File: `kuzu-core/kuzu-optimizer/src/passes.rs`*

**Step A5** — Wire both passes into `Optimizer`

- Modify `Optimizer::new()` in `optimizer.rs` to register both passes in the tree-based pipeline
- Keep ALL existing 8 flat passes — add tree passes after them (or integrate into same pipeline)
- Update `Optimizer::optimize()` to run both flat and tree passes
- Update pass_names test

*File: `kuzu-core/kuzu-optimizer/src/optimizer.rs`*

**Step A6** — Test the new passes

- Add unit tests for `FactorizationRewriting`: star join pattern, hash join with probe/build flattening
- Add unit tests for `CardinalityEstimation`: scan node, filter (PK/non-PK), hash join, cross product
- Verify that existing flat passes still work (regression tests)

*File: `kuzu-core/kuzu-optimizer/src/passes.rs` (append tests)*

**Dependencies**: A1 → A2 → A3, A4 (parallel after A2) → A5 → A6

---

## Phase B: Fase 19 — Extension Porting (9 remaining)

### Step B1 — ALGO Extension (`kuzu-algo`, Medium complexity, no external deps)

**What it registers**: 10 table functions (with 4 aliases):
- `STRONGLY_CONNECTED_COMPONENTS` (alias `SCC`)
- `STRONGLY_CONNECTED_COMPONENTS_KOSARAJU` (alias `SCC_KO`)
- `WEAKLY_CONNECTED_COMPONENTS` (alias `WCC`)
- `PAGE_RANK` (alias `PR`)
- `K_CORE_DECOMPOSITION` (alias `KCORE`)
- `LOUVAIN`
- `SPANNING_FOREST` (alias `SF`)

**Strategy**: Wrap existing `kuzu-graph` algorithms (BFS, PageRank, WCC) + add missing ones as Rust implementations (SCC, K-Core, Louvain, Spanning Forest).

**Actions**:
1. Create `kuzu-core/kuzu-algo/` with `Cargo.toml`, `src/lib.rs`
2. Implement `AlgoExtension` struct + `Extension` trait
3. Port each algorithm as a Rust function (or wrap existing graph algorithms):
   - **PageRank** — already exists in `kuzu-graph::algorithms::page_rank()` → wrap as table function
   - **WCC** — already exists → wrap
   - **SCC (Tarjan)** — new implementation (~80 lines)
   - **SCC Kosaraju** — new implementation (~50 lines)
   - **K-Core decomposition** — new implementation (~60 lines)
   - **Louvain** — new modularity optimization (~150 lines)
   - **Spanning Forest (Kruskal)** — new implementation (~50 lines)
   - **Component IDs** — new utility (~30 lines)
4. Register each as a table function via `context.register_table_function()`
5. Register aliases (SCC, SCC_KO, WCC, PR, KCORE, SF)
6. Add `algo-extension` feature to `kuzu-main/Cargo.toml`
7. Add `kuzu-algo` to workspace members in root `Cargo.toml`
8. Wire registration in `database.rs` behind `#[cfg(feature = "algo-extension")]`

*New files: `kuzu-core/kuzu-algo/Cargo.toml`, `kuzu-core/kuzu-algo/src/lib.rs`*
*Modified files: `kuzu-core/Cargo.toml`, `kuzu-core/kuzu-main/Cargo.toml`, `kuzu-core/kuzu-main/src/database.rs`*

### Step B2 — NEO4J Extension (`kuzu-neo4j`, Low complexity, no external deps)

**What it registers**: 1 standalone table function: `NEO4J_MIGRATE`

**Strategy**: Parse Neo4j Cypher dump format and create Kuzu schema/nodes/rels. Standalone function, no external dependencies.

**Actions**:
1. Create `kuzu-core/kuzu-neo4j/` with `Cargo.toml`, `src/lib.rs`
2. Implement `Neo4jExtension` struct + `Extension` trait
3. Implement `neo4j_migrate()` parser — reads Neo4j dump format (CREATE CONSTRAINT, CREATE INDEX, CREATE (n:Label {props}), MATCH ... CREATE (a)-[:REL]->(b))
4. Register as table function
5. Add feature flag + workspace member + database.rs registration

*New files: `kuzu-core/kuzu-neo4j/Cargo.toml`, `kuzu-core/kuzu-neo4j/src/lib.rs`*
*Modified files: same as B1*

### Step B3 — LLM Extension (`kuzu-llm`, Medium complexity, needs HTTP client)

**What it registers**: 1 scalar function: `CREATE_EMBEDDING`

**Strategy**: HTTP client for LLM provider APIs (OpenAI, VoyageAI, Google Vertex, Ollama, etc.). Needs a Rust HTTP client (`reqwest` or `ureq`) and JSON parsing (`serde_json` already available).

**Actions**:
1. Create `kuzu-core/kuzu-llm/` with `Cargo.toml`, `src/lib.rs`
2. Add `ureq` or `reqwest` as dependency (lightweight HTTP client)
3. Implement `LlmExtension` struct + `Extension` trait
4. Implement providers: `open_ai.rs`, `ollama.rs` (simplest two to start)
5. Implement `create_embedding()` scalar function
6. Register via `context.register_scalar_function()`
7. Add feature flag + workspace member + database.rs registration

*New files: `kuzu-core/kuzu-llm/Cargo.toml`, `kuzu-core/kuzu-llm/src/lib.rs`, `kuzu-core/kuzu-llm/src/open_ai.rs`, `kuzu-core/kuzu-llm/src/ollama.rs`*
*Modified files: same as B1*

### Step B4 — DuckDB-dependent extensions (placeholders, BLOCKED)

**Extensions**: AZURE, DELTA, ICEBERG, POSTGRES, SQLITE, UNITY_CATALOG

**Blocker**: All 6 depend on a real DuckDB Rust binding. Current `kuzu-duckdb` is a connector stub.

**Strategy**: For each, create a minimal placeholder crate that:
1. Defines the `Extension` struct
2. Implements `fn load()` that logs a warning: `"Extension '{name}' loaded — DuckDB connector stub. Full functionality requires the \`duckdb\` crate."`
3. Registers no actual functions
4. Adds the feature flag + workspace member

This documents the intention and makes the count explicit (5 of 14 ported → the placeholder approach means we acknowledge the remaining 6).

**Actions** (per extension, identical pattern):
1. Create `kuzu-core/kuzu-{ext}/Cargo.toml` with `kuzu-extension` dependency
2. Create `kuzu-core/kuzu-{ext}/src/lib.rs` with Extension impl
3. Add feature `{ext}-extension` to `kuzu-main/Cargo.toml`
4. Add crate to workspace members
5. Add registration in `database.rs` behind feature flag

*New directories: `kuzu-core/kuzu-azure/`, `kuzu-core/kuzu-delta/`, `kuzu-core/kuzu-iceberg/`, `kuzu-core/kuzu-postgres/`, `kuzu-core/kuzu-sqlite/`, `kuzu-core/kuzu-unity-catalog/`*
*Modified files: same as B1*

### Dependency within Phase B

B1, B2, B3 are **independent** — can run in parallel.
B4 is **independent** of B1-B3.

---

## Relevant Files

### Fase 17
- `kuzu-core/kuzu-planner/src/logical_operator.rs` — add `cardinality`, `LogicalFlatten`, tree helpers
- `kuzu-core/kuzu-planner/src/lib.rs` — re-export updates
- `kuzu-core/kuzu-optimizer/src/passes.rs` — new `TreeOptimizationPass`, both pass impls, tests
- `kuzu-core/kuzu-optimizer/src/optimizer.rs` — wire tree passes into pipeline
- `kuzu-core/kuzu-optimizer/src/lib.rs` — re-export updates
- Reference: `src/optimizer/factorization_rewriter.cpp` (C++ 153 lines)
- Reference: `src/optimizer/cardinality_updater.cpp` (C++ 120 lines)
- Reference: `src/planner/join_order/cardinality_estimator.cpp` (C++ 250 lines)
- Reference: `src/include/optimizer/logical_operator_visitor.h` (visitor pattern)

### Fase 19
- `kuzu-core/kuzu-graph/src/algorithms.rs` — existing BFS, PageRank, WCC for ALGO wrapping
- `kuzu-core/kuzu-graph/src/graph.rs` — CSR adjacency for ALGO
- `kuzu-core/kuzu-json/src/lib.rs` — reference extension pattern
- `kuzu-core/kuzu-extension/src/lib.rs` — `Extension` trait
- `kuzu-core/kuzu-extension/src/context.rs` — `ExtensionContext` API
- `kuzu-core/kuzu-extension/src/registry.rs` — `ExtensionRegistry`
- `kuzu-core/kuzu-main/src/database.rs` — extension registration hub
- `kuzu-core/kuzu-main/Cargo.toml` — feature flags
- `kuzu-core/Cargo.toml` — workspace members
- `kuzu-core/kuzu-function/src/registry.rs` — `ScalarFunction`, `TableFunction` enums (may need `Custom` variant for extension-specific functions)
- C++ refs: `extension/algo/`, `extension/neo4j/`, `extension/llm/`

---

## Verification

### Fase 17
1. `cargo test -p kuzu-optimizer` — all 9 existing tests pass + new tests for factorization rewriting and cardinality estimation
2. `cargo test -p kuzu-planner` — no regressions
3. `cargo test --workspace` — full test suite still passes (203 tests)
4. Manual: construct a query plan with star join, run `FactorizationRewriting`, verify `LogicalFlatten` nodes inserted
5. Manual: construct a query plan with filter on PK, run `CardinalityEstimation`, verify cardinality=1

### Fase 19
1. `cargo check --workspace` — compiles clean
2. `cargo check --target wasm32-unknown-unknown` — WASM compatibility
3. `cargo test -p kuzu-algo` — algorithm unit tests pass
4. `cargo test -p kuzu-neo4j` — migration parser tests pass
5. `cargo test -p kuzu-llm` — embedding function tests pass (mocked HTTP)
6. For each new extension: verify feature flag toggle works (`--features algo-extension`)
7. For placeholder extensions: verify warning log on load

---

## Decisions

- **Tree vs Flat**: Adding `TreeOptimizationPass` alongside flat passes (not replacing). Existing passes keep working on flat lists. New passes use tree visitor. `Optimizer` runs flat passes first, then tree passes.
- **Cardinality source**: Static selectivity constants only (no storage dependency). Real stats integration deferred to later phase.
- **ALGO strategy**: Wrap existing `kuzu-graph` algorithms rather than reimplementing from C++. New algorithms (SCC, K-Core, Louvain, Spanning Forest) implemented directly in Rust.
- **DuckDB-dependent extensions**: Placeholder crates only. Real implementation blocked on `duckdb` Rust crate integration.
- **ScalarFunction/TableFunction enums**: May need to add a `Custom` variant to the `ScalarFunction` and `TableFunction` enums to support extension-specific evaluation behavior (currently, extensions abuse `UtilityOp::Coalesce` as a placeholder).

## Scope Excluded
- Storage-engine-backed cardinality estimation (uses real table stats from storage)
- Join order enumeration based on cardinality estimates
- PreparedStatement implementation
- C++ code removal
- CI/CD setup
- tools/rust_api integration
